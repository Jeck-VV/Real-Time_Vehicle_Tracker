# Manuale del Progettista - Sistema Georuggine

## 1. Introduzione e Obiettivi del Progetto
Georuggine è un sistema distribuito client-server sviluppato integralmente in **Rust**. L'obiettivo principale è la gestione, il tracciamento e la comunicazione con una flotta di veicoli in tempo reale. 
Il sistema soddisfa requisiti stringenti di affidabilità, concorrenza e performance, garantendo al contempo la compatibilità multipiattaforma (compilabile per Windows, Linux, macOS, iOS e Android).

### 1.1 Scelte Architetturali di Base
Il progetto è stato strutturato sfruttando le potenzialità dei **Cargo Workspace**, dividendolo in tre crate per favorire il riuso del codice, velocizzare i tempi di compilazione e preservare la pulizia semantica:
* **`common`**: Libreria pura (senza I/O di rete) che incapsula la definizione del protocollo (`ClientMessage`, `ServerMessage`), le costanti universali (soglie di distanza e tempo) e le funzioni matematiche per la geometria terrestre. Condivisa esplicitamente tra client e server per garantire che entrambi "parlino la stessa lingua".
* **`server`**: Un'applicazione fortemente concorrente basata sull'ecosistema **Tokio**. Gestisce migliaia di connessioni TCP multiplexate su pochi thread del sistema operativo, elabora lo stato degli utenti e memorizza in modo persistente le credenziali.
* **`client`**: Un binario con duplice natura: simulatore di posizioni (tramite riga di comando o interattivo) e interfaccia grafica (GUI) sviluppata in `eframe`/`egui`.

---

## 2. Il Protocollo di Comunicazione (Wire Protocol)
Nelle applicazioni di tracciamento continuo (es. ping GPS ogni 30 secondi per migliaia di utenti), la serializzazione dei dati impatta pesantemente sulla CPU e sulla banda di rete.

### 2.1 Perché un protocollo custom Line-Based?
Invece di affidarci a formati testuali verbosi (come JSON) o formati binari rigidi e difficili da debuggare senza tooling esterno (come Protobuf), è stato progettato un protocollo testuale custom. I messaggi sono separati dal carattere pipe (`|`) e un singolo messaggio è delimitato dal carattere di a capo (`\n`). 
* **Vantaggi di CPU/Rete:** Costo computazionale quasi nullo (basta un `str::split('|')`). Non c'è un parser AST in memoria.
* **Vantaggi Operativi:** Facilmente ispezionabile tramite shell (`telnet` o `netcat`).
* **Sicurezza (Sanitizzazione contro Framing Attack):** Se un utente invia come password o messaggio chat una stringa contenente un `\n`, il protocollo rischierebbe di dividere la stringa trattandola come due comandi distinti (TCP framing corruption). Per neutralizzare questa minaccia, il crate `common` forza una funzione di `sanitizza_campo()` che sostituisce tutti i `\n` e `\r` con degli spazi prima di immettere la stringa nel payload finale. 

### 2.2 Prevenzione Memoria (Max Line Length)
Per leggere le righe, il server sfrutta `FramedRead` e `LinesCodec` dal crate `tokio-util`. Questo evita un attacco DoS (Denial of Service) classico: se un attaccante inviasse un milione di byte continui *senza mai inviare un `\n`*, un normale `BufReader` allocerebbe RAM all'infinito fino al crash del server (OOM - Out of Memory). Imponendo a livello di Codec un limite di `4096 byte` per riga, il server disconnette forzatamente l'attaccante in sicurezza.

### 2.3 Specifica dei Comandi
* **Registrazione e Autenticazione:**
  * `REGISTER|username|password` $\rightarrow$ `REGISTER_OK` | `REGISTER_ERR|motivo`
  * `LOGIN|username|password` $\rightarrow$ `LOGIN_OK` | `LOGIN_ERR|motivo`
* **Geolocalizzazione:**
  * `POS|lat|lon|timestamp` $\rightarrow$ Nessun echo verso la rete, il server risponde solo con un `NOTICE` privato di conferma e aggiorna il calcolo dello stato (FSM).
* **Messaggistica (Chat):**
  * `TEXT|ALL|contenuto` (Broadcast)
  * `TEXT|utente_destinatario|contenuto` (Unicast / Messaggio privato)
* **Analisi Storica:**
  * `REQANALYSIS|DAY` (oppure `WEEK`, `MONTH`) $\rightarrow$ `ANALYSIS|utente|intervallo|dist_km|vel_kmh|mov_sec|pausa_sec`.

---

## 3. Concorrenza e Gestione della Memoria (Server)
Il server sfrutta il runtime asincrono M:N di Tokio (Multiplexing di M task logici su N thread fisici del SO). Il pattern centrale è *One-Task-Per-Connection*.

### 3.1 Utilizzo granulare di `Arc<RwLock<T>>`
Per mantenere lo stato condiviso in modo thread-safe (evitando *data races* pur senza bloccare l'executor con i pesanti `std::sync::Mutex`), la logica impiega lock a doppia polarità (`RwLock` di Tokio):
* **`UserDb`**: Mappa Username $\rightarrow$ Hash password. Dato che il 99% delle operazioni è una "Lettura" (durante le fasi di Login), un `RwLock` permette a $N$ thread di leggere simultaneamente. Il blocco scatta solo in caso di "Scrittura" (nuova registrazione).
* **`ActiveUsers`**: Mappa gli utenti *attualmente online* ai rispettivi canali di comunicazione privata (`mpsc::Sender`). Serve a instradare messaggi di testo privati tra client con O(1) di lookup time.
* **`UserSessions`**: Memorizza la cronologia posizionale (tracciato) e lo stato attuale (Ferm/InMov) della Macchina a Stati.

### 3.2 Hashing Sicuro e "Blocking Tasks"
L'hashing crittografico delle password tramite **Bcrypt** è appositamente CPU-Bound e lento (per prevenire attacchi brute-force). Eseguire Bcrypt sull'event-loop asincrono causerebbe lo *starvation* di tutti gli altri task (il server non risponderebbe ai ping). La soluzione è delegare l'hash a `tokio::task::spawn_blocking`, che "affitta" un thread nativo dal pool di backup del SO, lasciando Tokio libero di continuare a processare le migliaia di altre socket I/O nel frattempo.

### 3.3 Gestione Multiplexata degli Eventi (Macro `tokio::select!`)
In un singolo task utente convergono 3 fonti asincrone simultanee:
1. Lettura dal socket TCP (Client che scrive al server).
2. Lettura dal canale `mpsc` (Un altro client che ci invia un messaggio privato).
3. Lettura dal canale `broadcast` (Il server o altri client emettono comunicazioni globali).

Senza un corretto multiplexing, restare in attesa bloccante (es. `.recv()`) su un canale impedirebbe la lettura dagli altri. La macro `tokio::select!` gestisce questo asincronismo sbloccando l'esecuzione non appena un *qualsiasi* evento è pronto per il consumo.
**Filtro Anti-Eco nel Broadcast**: Quando un evento `Broadcast` arriva alla `select!`, il server lo decodifica. Se il campo `mittente` coincide col proprio nome utente, il messaggio viene ignorato (`continue`), evitando che chi invia un messaggio in piazza lo riceva indietro due volte.

---

## 4. Macchina a Stati Finita (FSM) e Geometria Terrestre
L'identificazione dello stato veicolare deve tenere in conto il rumore hardware: l'instabilità fisiologica del segnale GPS (drift/tremolio spaziale).

### 4.1 Logica delle Transizioni (Haversine)
1. **Distanza di Haversine**: Poiché la Terra è una sfera oblata, non possiamo usare il teorema di Pitagora (geometria piana Euclidea) sulle coordinate in gradi. `distanza_haversine` esegue complessa trigonometria sferica basata su radianti e il raggio di 6371 km per ottenere l'arco di distanza in Kilometri con precisione assoluta su scala globale.
2. **Soglie (Low-pass Filter Spaziale e Temporale)**:
   * `SOGLIA_MOVIMENTO_KM = 0.005` (5 metri). Qualsiasi spostamento minore non ripristina il contatore del movimento.
   * `SOGLIA_PAUSA_SEC = 180` (3 minuti).
3. **Flusso FSM**: L'utente inizia nello stato **Disconnesso**. Invia la prima POS $\rightarrow$ **InMovimento**. Se le successive POS inviate distano dall'ultima *meno di 5 metri* e passa una durata complessiva *superiore a 3 minuti*, la FSM transita su **Fermo**. Appena la coordinata dista di nuovo a più di 5 metri, torna istantaneamente a **InMovimento**.

---

## 5. Algoritmo di Analisi Storica (`analisi.rs`)
I calcoli per i contachilometri parziali (`REQANALYSIS`) presentano le insidie dei sistemi asincroni e degli orologi hardware (Clock Skew).

### 5.1 Sanificazione dei Tempi
1. **Time-Drift e Filtro nel Futuro**: A causa di orologi sballati o manomessi (Cheating), un client potrebbe inviare timestamp collocati nel futuro, sfalsando i calcoli di velocità. Viene applicata una tolleranza massima (`TOLLERANZA_CLOCK_SKEW_SEC = 60`). Punti oltre i 60s nel futuro rispetto al clock del server vengono categoricamente rigettati con errore (Err) prima di impattare sull'analisi.
2. **Riordinamento**: In condizioni di alto stress o da importazioni via file, un vettore cronologico potrebbe ricevere append asincroni fuori ordine (es. il punto delle 14:00 processato dopo il punto delle 14:01). Prima di eseguire la somma iterativa sui frammenti di traiettoria (con il metodo `windows(2)` nativo di Rust), lo slice in RAM viene rigorosamente riordinato per `timestamp`.

Tramite questi step, il calcolo della `velocita_media_kmh` (km/h) calcolato sui tempi di movimento (escludendo i tempi calcolati sotto SOGLIA_MOVIMENTO_KM) risulta esente da falsi positivi causati dal tremolio GPS da parcheggio.

---

## 6. Il Client (e GUI Decoupling)
L'app `client` possiede una GUI interattiva fornita da `egui`, una libreria *immediate-mode*.

### 6.1 Separazione tra View e Network Loop
Le interfacce grafiche moderne non possono (e non devono) bloccarsi aspettando pacchetti di rete. 
Abbiamo separato le "vite" delle due identità:
* **Il thread della rete TCP**: Funziona su `Tokio`, gestisce i ping/pong e pusha gli aggiornamenti di stato e chat verso un canale unidirezionale `mpsc::Sender<GuiEvent>`.
* **Il thread del Render (GUI)**: Esegue a 60 frame per secondo. All'inizio di ogni frame cicla un `try_recv()` sul canale. Se ci sono pacchetti nuovi li estrae asincronamente senza aspettare, vernicia (draw) a video il nuovo tracciato o la nuova chat, per poi tornare a dormire. Questo decoupling radicale annulla del tutto fenomeni di UI-Lagging.

### 6.2 Modulo Simulatore (`simulator.rs`) e Condivisione Atomica
Il simulatore genera coordinate fittizie o riproduce storici dal tratto CSV (`sample_route.csv`).
L'imputazione da GUI del comando START/STOP Simulator è governata tramite l'uso di puntatori atomici thread-safe `Arc<AtomicBool>`. Cliccando STOP nella GUI, l'oggetto booleano viene settato `false` (usando `Ordering::SeqCst` per forzare l'inversione della barriera di memoria), portando il thread simulatore interno ad uccidersi in totale sicurezza (graceful shutdown) invece che bloccarsi su un loop cieco.

---

## 7. Testing Strategy: Architecture and Coverage
Rust promuove testing integrato forte.
* **Unit Tests (`#[cfg(test)]`)**: Affidati a funzioni isolate (`common/src/lib.rs` e calcoli puri di `analisi.rs`). In questo stadio si testano le conversioni JSON, errori d'orologio o coordinate NaN/Infinito.
* **Test di Integrazione Completi (`tests/`)**: A differenza dei classici Unit Test, questi simulano *server completi end-to-end*. Ma per testarli in parallelo e non scontrarsi su indirizzi occupati (Errore `EADDRINUSE`), la suite forza i server virtuali all'ascolto sull'indirizzo `127.0.0.1:0`. Lo `0` delega al kernel del sistema operativo il compito di trovare una porta TCP alta libera randomica. Il client di test effettua l'hook su quella precisa porta locale. Ne consegue che decine di server-test possono ruotare sulla stessa CPU contemporaneamente, accorciando drasticamente il tempo per CI/CD. Per evitare overhead da test e timeout, in compilazione debug/test la crittografia Bcrypt vede ridotto il `cost` (da 12 a 4).

---

## 8. Misurazioni, Tuning OS e Prestazioni
### 8.1 Tracciamento CPU in Loop Separato
Il mandato di progetto richiede di loggare la CPU ogni 2 minuti. Eseguire l'analisi di `sysinfo` per i thread dell'intero sistema su Tokio significherebbe bloccarlo. Quindi, il `sysinfo` viene incapsulato in un task `spawn` distaccato con uno sleep esatto di 119 secondi. L'evento non intasa gli I/O queue manager (`epoll` su linux, `kqueue` su macOS).

### 8.2 Dimensione degli Eseguibili (Profile Release)
Il compilatore `rustc` attua pesanti passaggi LTO (Link Time Optimization). 
Di seguito sono riportate le dimensioni per le due principali piattaforme supportate:

**macOS (Apple Silicon / Intel):**
* **Server**: **1.566.176 byte (~1.6 MB)**. Non avendo dipendenze dal runtime complesse e gestite staticamente, l'eseguibile Unix-like è altamente compatto.
* **Client Simulator CLI**: Estremamente minimale, **1.117.072 byte (~1.1 MB)**.
* **Client GUI**: Risulta di circa **13.669.024 byte (~13.7 MB)**. L'impennata dimensionale è dovuta all'integrazione di `wgpu` e all'inclusione nativa dell'API grafica Metal per l'accelerazione hardware.

**Windows (.exe):**
* **Server.exe**: **1.148.928 byte (~1.1 MB)**
* **Simulator_Client.exe**: **732.160 byte (~0.7 MB)**
* **Client.exe**: **14.972.416 byte (~15 MB)**. Come su Mac, l'inclusione delle API host (es. DirectX12 / Vulkan) alza il peso del client visivo a fronte di enormi vantaggi in termini di frame-rate e sgravio della CPU.

---

## 9. Risultati e Conclusioni
Il design di "Georuggine" rispecchia l'ecosistema di Rust moderno.
1. **Safety Zero-Cost**: Evitati memory leaks grazie all'assenza di puntatori grezzi GC-driven.
2. **Concorrenza Fearless**: Nonostante migliaia di letture e scritture parziali incrociate, la struttura rigorosa garantita a compile-time del Borrow Checker impedisce deadlocks catastrofici (garantita dagli scope limitati in `RwLock`).
3. **Resilienza ai Dati Sporchi**: L'aggiunta di filtraggi (Clock Skew, Buffer Overrun Size, Line-Formatting) rende l'architettura blindata contro crash non gestiti (il temuto Panic!), garantendo stabilità Server di altissimo livello enterprise-grade.
