# Guida allo studio — Presentazione Georuggine

Materiale di preparazione per l'orale. La valutazione è **individuale** anche se il progetto è di gruppo ("verranno valutate, nel colloquio orale, le singole persone e i rispettivi contributi al lavoro" — dalle regole del corso): significa che ognuno deve saper spiegare a fondo la propria parte, **e** avere una comprensione minima decente delle altre tre, perché il docente può fare domande incrociate.

Come è organizzata questa guida:
1. **Parte comune** — la deve sapere chiunque, indipendentemente dal ruolo.
2. **Una sezione per persona** — dettaglio tecnico della propria parte + domande plausibili con risposta pronta.
3. **Domande trasversali** — concetti di Rust/sistemi che chiunque dei 4 potrebbe doversi sentire chiedere.
4. **Punti deboli noti** — le cose che *sappiamo* essere discutibili o incomplete. Meglio arrivarci preparati che scoprirle in diretta.
5. **Consigli pratici per l'orale**.

---

## 1. Parte comune (la deve sapere chiunque)

### Cos'è il progetto, in una frase
Sistema client/server in Rust per tracciare la posizione (emulata) di una flotta di utenti/veicoli, gestirne lo stato (fermo/in movimento/disconnesso), fornire analisi statistiche sui loro spostamenti, e permettere messaggistica testuale tra utenti e con il server.

### Architettura: 3 crate in un Cargo workspace
- **`common`**: nessuna dipendenza di rete. Contiene `ClientMessage`/`ServerMessage` (il protocollo), `Posizione`, `UserState`, `Interval`, `ReportAnalisi`, la funzione `distanza_haversine`, le soglie condivise (`SOGLIA_MOVIMENTO_KM`, `SOGLIA_PAUSA_SEC`), e `coordinate_valide`. Il motivo per cui esiste come crate separato: client e server devono "parlare la stessa lingua" — se cambia un campo del protocollo, il compilatore obbliga ad aggiornare tutti i punti che lo usano, in entrambi i binari.
- **`server`**: un binario (`main.rs`) + una libreria (`lib.rs`, con `username_valido`, `posizione_valida`, `UserSessionData`, usata anche dai test). Gira su Tokio, un task asincrono per client connesso.
- **`client`**: due binari — `client` (GUI in `egui`) e `simulator_client` (CLI testuale). Entrambi usano `simulator.rs` per generare posizioni.

### Il protocollo (wire format)
Testo, una riga per messaggio, campi separati da `|`, terminata da `\n`. Esempi:
```
REGISTER|username|password  →  REGISTER_OK  |  REGISTER_ERR|motivo
LOGIN|username|password     →  LOGIN_OK     |  LOGIN_ERR|motivo
POS|lat|lon|timestamp       →  NOTICE|... (ack privato, mai in broadcast)
TEXT|ALL|contenuto          (broadcast)
TEXT|username|contenuto     (privato)
TEXT|SERVER|contenuto       (all'operatore del server)
REQANALYSIS|DAY|WEEK|MONTH  →  ANALYSIS|utente|intervallo|dist_km|vel_kmh|mov_sec|pausa_sec
```
**Perché testo e non JSON/binario?** Compromesso deliberato: ispezionabile a mano (basta `telnet`/`netcat`), parsing quasi gratis (`str::split('|')`), nessuna libreria di serializzazione necessaria. Svantaggio: niente escaping automatico — un `\n` o `|` in un campo libero romperebbe il framing, per questo esiste `sanitizza_campo()` (sostituisce `\n`/`\r` con spazi prima di incapsulare qualunque stringa libera nel payload).

### Stati dell'utente (FSM)
`Disconnesso` → `InMovimento` (prima posizione, o spostamento oltre 5m dall'ultima) → `Fermo` (posizione entro 5m per almeno 180s consecutivi) → torna a `InMovimento` al primo spostamento oltre soglia. Le soglie (`SOGLIA_MOVIMENTO_KM = 0.005`, `SOGLIA_PAUSA_SEC = 180`) sono condivise tra la FSM live (`server/src/lib.rs`) e il modulo di analisi (`server/src/analisi.rs`), così i due non si contraddicono mai.

### Analisi del movimento
Su richiesta (`REQANALYSIS`), il server calcola per l'utente **che ha fatto la richiesta** (non per un utente arbitrario): distanza totale (somma di Haversine tra punti consecutivi), velocità media, tempo in movimento, tempo in pausa — filtrati sull'intervallo **calendariale corrente** (oggi da mezzanotte UTC, questa settimana da lunedì, questo mese dal giorno 1) tramite `chrono`.

### Messaggistica
Tre canali: broadcast (`tokio::sync::broadcast`, con filtro anti-eco sul campo tipizzato `mittente`), privato (un `mpsc::Sender` per utente in una `HashMap` condivisa), e un target riservato `SERVER` per scrivere all'operatore. L'operatore ha una console (`b`, `u`, `lista`) per rispondere.

### Dove vivono i file generati
`utenti_georuggine.json` (DB utenti) e `server_cpu.log` (log CPU) si trovano **accanto all'eseguibile in esecuzione** (calcolato a runtime con `std::env::current_exe()`), non in un percorso fisso legato al codice sorgente — importante per la portabilità multipiattaforma.

---

## 2. [Persona A] Networking e autenticazione

### Cosa sapere a fondo
- **Un task Tokio per connessione**: `listener.accept().await` in loop, ogni socket accettato viene passato a `tokio::spawn(handle_client(...))`. Migliaia di connessioni possono coesistere su pochi thread OS perché Tokio moltiplexa i task su un pool di worker (modello M:N).
- **`tokio::select!`** dentro `handle_client`: un singolo task deve reagire a tre sorgenti asincrone contemporaneamente — lettura dal socket del client, ricezione da un canale `mpsc` (messaggi privati per lui), ricezione dal canale `broadcast` (messaggi globali). `select!` sblocca sul primo evento pronto, senza bloccare sugli altri.
- **Registrazione**: controllo rapido di esistenza (in lettura), poi hashing bcrypt **fuori da qualunque lock** (`tokio::task::spawn_blocking`), poi un secondo controllo di esistenza sotto lock di scrittura prima di inserire — pattern "double-checked locking" per evitare che due registrazioni con lo stesso username in corsa si sovrascrivano a vicenda.
- **Perché `spawn_blocking` per bcrypt**: bcrypt è *deliberatamente* lento (anti brute-force, ~100-300ms per hash al cost di default 12). Eseguirlo direttamente in un task asincrono bloccherebbe il thread worker di Tokio, congelando tutti gli altri client multiplexati su quel thread. `spawn_blocking` lo sposta su un pool di thread dedicato alle operazioni bloccanti.
- **Login**: verifica hash, poi controllo che lo username non sia già online altrove (altrimenti `LoginErr`); se la stessa connessione aveva già fatto login con un altro username, quella vecchia entry viene ripulita prima di registrare la nuova.
- **`Arc<RwLock<HashMap<...>>>`** per tre mappe condivise: utenti registrati (`UserDb`), utenti online (`ActiveUsers`, mappa username → canale mpsc), sessioni/storico (`UserSessions`). `RwLock` invece di `Mutex`: la maggior parte delle operazioni sono letture (login, invio messaggi) che possono avvenire in parallelo; solo scritture (registrazione, connessione/disconnessione) richiedono accesso esclusivo.
- **Persistenza atomica**: `salva_utenti` scrive su un file temporaneo e poi lo rinomina sopra il file finale (`fs::rename`) — il rename è atomico a livello filesystem, quindi un crash a metà scrittura non lascia mai il DB troncato. `carica_utenti` distingue "file assente" (normale) da "file corrotto" (JSON non valido): nel secondo caso il file viene rinominato in `.json.corrotto` per recupero manuale, non silenziosamente scartato.
- **Disconnessione**: rimozione dalla mappa utenti attivi, stato → `Disconnesso`. Non si azzera il timestamp dell'ultimo cambio di posizione, così una riconnessione calcola correttamente il tempo trascorso invece di scattare artificialmente a "Fermo".

### Domande plausibili
- *"Cosa succede se due client si registrano con lo stesso username nello stesso istante?"* → Il doppio controllo (prima e dopo l'hashing) garantisce che solo uno riceva `REGISTER_OK`; c'è un test automatico che lo verifica (`test_registrazione_simultanea_race_condition`).
- *"Perché `RwLock` e non `Mutex`?"* → Le letture (molto più frequenti) possono avvenire in parallelo con `RwLock`; con `Mutex` sarebbero serializzate anche tra loro, inutilmente.
- *"Le password sono al sicuro?"* → Hashate con bcrypt (non in chiaro) nel DB; **non** c'è TLS sul trasporto, quindi viaggiano in chiaro sulla rete — limite noto, documentato, accettabile per un progetto didattico locale.
- *"Cosa succede se un client si disconnette bruscamente mentre scrive?"* → Il socket restituisce errore/EOF sulla prossima lettura, il task esce dal loop e fa la pulizia (rimozione da utenti attivi, stato → Disconnesso). Nessun panic, nessuna risorsa persa.

---

## 3. [Persona B] Stato utente, storico posizioni e simulatore

### Cosa sapere a fondo
- **Perché la distanza di Haversine e non Pitagora**: le coordinate sono gradi su una sfera (la Terra), non un piano cartesiano. Pitagora su lat/lon darebbe risultati sbagliati (l'errore cresce con la distanza e la latitudine). Haversine calcola l'arco di cerchio massimo tra due punti su una sfera di raggio noto (6371 km).
- **Soglie della FSM**: `SOGLIA_MOVIMENTO_KM = 0.005` (5 metri) e `SOGLIA_PAUSA_SEC = 180` (3 minuti), definite in `common` e condivise con l'analisi. La FSM confronta ogni nuova posizione con l'**ultima registrata** (non con un punto di ancoraggio fisso): se la distanza supera 5m, reset del timer e stato → InMovimento; altrimenti si accumula tempo e, oltre 180s, stato → Fermo.
- **Perché una soglia di distanza e non l'uguaglianza esatta delle coordinate** (anche se la specifica dice testualmente "al primo cambiamento di coordinata"): con coordinate `f64` generate da un simulatore, un'uguaglianza esatta renderebbe "Fermo" quasi irraggiungibile nella pratica (il random walk cambia sempre qualcosa, anche minimamente) e sarebbe fragile a qualunque rumore. La soglia dei 5m è una scelta di robustezza esplicita, documentata nel manuale del progettista — **sapetela argomentare**, non è un errore ma una decisione.
- **Storico posizioni**: `Vec<Posizione>` per utente dentro `UserSessionData`, popolato a ogni `POS` ricevuto. Usato sia dalla FSM (solo l'ultima posizione) sia dall'analisi (l'intero storico, filtrato per intervallo temporale).
- **Simulatore — due strategie implementate** (la traccia ne richiede almeno una, "ecc. ecc." lascia libertà):
  - **Random walk**: parte da un punto centrale, a ogni tick applica un delta casuale in gradi (`rand::rng().random_range`), poi clampa lat/lon ai range validi (`-90..=90`, `-180..=180`). Non termina mai da solo.
  - **File CSV**: legge righe `lat,lon` in anticipo (all'apertura del file), righe vuote o che iniziano con `#` sono commenti. Righe non numeriche o con coordinate fuori range vengono **scartate con un avviso**, non interrompono il caricamento delle altre. Termina quando le posizioni finiscono.
  - Più l'inserimento manuale da GUI, che tecnicamente è una terza modalità.
- **Invio ogni 30 secondi**: sia nel client GUI (task Tokio con `sleep(Duration::from_secs(30))` in loop) sia nel client CLI, generano una `PositionUpdate` col timestamp corrente e la mandano al task di rete.
- **Validazione posizione lato server** (non è "vostra" in senso stretto ma la userete): `posizione_valida()` rifiuta lat/lon non finiti o fuori range, e timestamp troppo nel futuro (tolleranza 60s per sfasamento di orologio).

### Domande plausibili
- *"Cosa succede se il GPS ha rumore e la posizione oscilla di 1-2 metri continuamente?"* → Resta sotto soglia (5m), quindi non conta come movimento: esattamente il comportamento voluto (basso-passa spaziale).
- *"Come aggiungereste una terza strategia con punto di partenza/arrivo e pause esplicite?"* → Servirebbe un nuovo `impl PositionSource` che interpola tra due coordinate nel tempo e, negli intervalli di pausa dichiarati, ripete l'ultima posizione — si inserirebbe nell'enum `SimulationStrategy` come le altre due.
- *"Perché il file CSV usa `lat,lon` e non il formato con timestamp che ha mostrato il docente nelle slide?"* → Onestamente: è una scelta di formato diversa e più semplice, non compatibile riga per riga col file di esempio del docente. Da menzionare se emerge, non da nascondere.

---

## 4. [Persona C] Analisi del movimento

### Cosa sapere a fondo
- **Formula di Haversine** (dovreste saperla scrivere a grandi linee sulla lavagna se chiesto): dati due punti (lat1,lon1), (lat2,lon2) in radianti,
  ```
  a = sin²(Δlat/2) + cos(lat1)·cos(lat2)·sin²(Δlon/2)
  c = 2·atan2(√a, √(1-a))
  distanza = R · c        (R = 6371 km)
  ```
- **Calcolo di `ReportAnalisi`**: si itera lo storico filtrato a coppie consecutive (`windows(2)`). Se la distanza tra due punti supera la soglia (5m), quel tratto conta come movimento (si somma alla distanza totale, si "scarica" il tempo fermo accumulato in pausa); altrimenti il tempo tra i due punti si accumula come pausa. Il tempo in movimento si ottiene per differenza: `tempo_totale - tempo_pausa`. La velocità media è `distanza_totale / tempo_in_movimento` (in ore) — **attenzione**: è la velocità media *durante il movimento*, non sul tempo totale (se fosse fermo per metà del periodo, la velocità media sul tempo totale sarebbe la metà).
- **Intervalli come "corrente", non finestra mobile**: `Interval::Day` = da mezzanotte UTC di oggi; `Week` = da lunedì (convenzione ISO) di questa settimana; `Month` = dal giorno 1 del mese corrente. Calcolato con `chrono` (`Utc::now().date_naive()`, `Datelike::weekday()`, `NaiveDate::with_day(1)`). **Non** sono "ultime 24h / ultimi 7 giorni / ultimi 30 giorni" — è una distinzione importante e un dettaglio a cui i docenti fanno spesso caso.
- **Ordinamento e filtro dei timestamp**: prima di calcolare, lo storico filtrato viene riordinato per `timestamp` (un client con orologio disallineato o pacchetti fuori ordine non deve falsare i calcoli), e i timestamp nel futuro rispetto ad "adesso" vengono esclusi (stesso motivo: dati non plausibili non devono influenzare l'analisi).
- **Limite noto**: il protocollo (`RequestAnalysis { interval }`) non porta uno username target — il server analizza sempre e solo l'utente della sessione che ha fatto la richiesta. Non esiste una funzione "analizza l'utente X" per un operatore/amministratore.

### Domande plausibili
- *"Perché UTC e non l'ora locale per calcolare l'inizio del giorno?"* → I timestamp delle posizioni sono epoch UTC generati sia dal client GUI che da quello CLI; usare UTC come riferimento evita ambiguità se client e server sono su fusi orari diversi.
- *"Cosa succede con un solo punto di posizione nello storico?"* → Serve almeno 2 punti per calcolare una distanza/durata; con 1 solo punto (o 0), tutti i valori restano a 0 — c'è un test dedicato (`test_reqanalysis_senza_dati` e simili).
- *"Come gestite un timestamp del client indietro nel tempo rispetto al precedente?"* → Lo storico viene riordinato prima del calcolo, quindi l'ordine di arrivo non conta, solo l'ordine cronologico reale.
- *"Posso chiedere l'analisi di un mio collega?"* → No, solo la propria — limite noto e dichiarato.

---

## 5. [Persona D] Messaggistica, prestazioni, multipiattaforma, report

### Cosa sapere a fondo
- **Broadcast**: `tokio::sync::broadcast::channel<BroadcastEvent>(100)`. Ogni client sottoscrive il canale alla connessione (anche prima del login, ma i messaggi vengono scartati finché non è loggato — altrimenti un client non autenticato potrebbe leggere conversazioni altrui). Il filtro anti-eco confronta `event.mittente` con l'username della sessione corrente — tipizzato, non un fragile confronto su prefissi di stringa.
- **Privato**: una `HashMap<String, mpsc::Sender<String>>` mappa ogni utente online al proprio canale. Per evitare di tenere il `RwLock` occupato durante un `send().await` (che può bloccare se il destinatario è lento/pieno), il `Sender` viene **clonato e il lock rilasciato prima** dell'attesa.
- **Target riservato `SERVER`**: `TEXT|SERVER|messaggio` non cerca un utente — stampa direttamente sulla console dell'operatore. Nessun account può registrarsi con quel nome (riservato in `username_valido`).
- **Console operatore**: comandi testuali letti da stdin del processo server, in un task Tokio separato — `b <msg>` (broadcast), `u <user> <msg>` (diretto), `lista` (chi è online).
- **Protezione da riga troppo lunga**: `FramedRead` + `LinesCodec::new_with_max_length(4096)` invece di un semplice `BufReader::lines()`. Un client che invia byte senza mai un `\n` verrebbe altrimenti bufferizzato all'infinito (rischio di esaurimento memoria) — con `LinesCodec` la connessione viene chiusa non appena il limite è superato.
- **Log CPU**: ogni ~2 minuti (1 secondo di misura + 119 di attesa = 120s), usando `sysinfo` per interrogare **il processo del server stesso** (via il proprio PID, non un aggregato di tutto il sistema). Vengono loggati sia il tempo di CPU totale accumulato (`accumulated_cpu_time()`, in millisecondi) sia la percentuale istantanea (`cpu_usage()`), come dato di contesto. La misura richiede due refresh a un secondo di distanza perché `sysinfo` calcola l'uso come differenza tra due istantanee.
- **Dimensioni eseguibili**: il client GUI è molto più pesante (~15 MB) del server (~1,1 MB) e del client CLI (~0,7 MB) per via delle dipendenze grafiche (`egui`/`wgpu`, con supporto nativo DirectX/Metal/Vulkan a seconda della piattaforma).
- **Documentazione prodotta**: README (specifica trascritta), Manuale Utente, Manuale del Progettista, Piano di Test, changelog delle scelte tecniche, questa presentazione. Sapete dire dove si trova ognuno di questi documenti nel repository.

### Domande plausibili
- *"Perché misurare il tempo CPU del solo processo e non di tutto il sistema?"* → La consegna chiede esplicitamente il tempo CPU "da parte del server" — un aggregato di sistema includerebbe altri programmi in esecuzione sulla stessa macchina, dato non richiesto e fuorviante.
- *"Come limitereste le connessioni simultanee o il traffico da un singolo client (rate limiting)?"* → Onestamente non implementato in questa versione; si potrebbe aggiungere un contatore di connessioni attive con un tetto massimo, o un token bucket per IP. È un limite noto, non nascondetelo se chiesto.
- *"Perché non TLS?"* → Scelta consapevole per un progetto didattico locale; le password sono comunque hashate a riposo (bcrypt), ma viaggiano in chiaro sul trasporto.
- *"Su quali piattaforme avete verificato che il codice compili ed esegua davvero?"* → **Rispondete con quello che avete davvero testato**, non con quello che è scritto nella slide/manuale se non l'avete verificato voi stessi.

---

## 6. Domande trasversali (chiunque può doverle affrontare)

Anche se non è la vostra parte, un minimo di comprensione di questi concetti generali di Rust/sistemi vi salva in caso di domande incrociate:

- **Ownership e borrowing**: Rust previene i data race a *compile time* — non serve un garbage collector, e il borrow checker impedisce di avere contemporaneamente un riferimento mutabile e uno immutabile allo stesso dato. Per questo il codice usa `Arc` (per condividere dati tra task/thread con conteggio di riferimenti) e `RwLock`/`Mutex` (per la mutabilità condivisa, verificata a runtime invece che a compile time).
- **Modello asincrono di Tokio**: un piccolo numero di thread reali (worker) esegue moltissimi task logici (`async fn`), che si "cedono il passo" a vicenda ogni volta che aspettano I/O (`.await`). Un task che fa lavoro bloccante *senza* `.await` (es. bcrypt, `fs::write` sincrono) monopolizza il thread worker e blocca tutti gli altri task su quel thread — da qui la necessità di `spawn_blocking` per le operazioni CPU-bound o di I/O sincrono.
- **`Mutex` vs `RwLock`**: `Mutex` dà accesso esclusivo a un solo "proprietario" alla volta (lettura o scrittura); `RwLock` permette letture concorrenti multiple ma scritture esclusive — conviene quando le letture sono molto più frequenti delle scritture, come nel nostro caso (login/lookup vs registrazione).
- **`mpsc` vs `broadcast`**: `mpsc` (multi-producer, single-consumer) è un canale punto-a-punto, usato qui per i messaggi privati (un solo destinatario). `broadcast` (multi-producer, multi-consumer) duplica ogni messaggio a tutti i sottoscrittori — usato per i messaggi a tutti.
- **Perché un protocollo testuale invece di JSON o Protobuf**: compromesso tra semplicità/debuggabilità (JSON sarebbe più verboso e servirebbe una libreria di serializzazione; Protobuf richiederebbe uno schema e tooling esterno) e assenza di validazione automatica (bisogna gestire a mano l'escaping — vedi `sanitizza_campo`).
- **Perché bcrypt e non un hash veloce come SHA-256**: SHA-256 è progettato per essere *veloce*, il che lo rende vulnerabile ad attacchi brute-force/rainbow-table su password. Bcrypt è deliberatamente lento e include un salt automatico, rendendo gli attacchi computazionalmente costosi.

---

## 7. Punti deboli noti — non nascondeteli se emergono

Meglio arrivarci con una risposta onesta pronta che essere colti impreparati:

1. **Soglia FSM (5m/3min) invece di "uguaglianza esatta delle coordinate"** come dice testualmente la traccia — scelta di robustezza motivata, vedi §3.
2. **Formato del file CSV** diverso da quello di esempio mostrato dal docente nelle slide (che ha un formato `mm:ss lat,lon` con timestamp esplicito) — il nostro è più semplice (`lat,lon`).
3. **Analisi solo per l'utente autenticato**, nessuna funzione amministrativa per analizzare un altro utente — vedi §4.
4. **Nessun TLS**: comunicazione in chiaro sul trasporto (password hashate solo a riposo, non in transito).
5. **Nessun rate limiting / limite al numero di connessioni concorrenti** — solo un limite alla lunghezza massima di una riga.
6. **Verifica multipiattaforma**: assicuratevi di sapere ESATTAMENTE su quali piattaforme avete davvero compilato ed eseguito il progetto prima di dichiararlo in presentazione — non fidatevi ciecamente di numeri scritti in precedenza se non li avete verificati voi.

Se il docente fa notare uno di questi punti, la risposta migliore è: riconoscerlo apertamente, spiegare la motivazione (se c'è, come per i punti 1 e 3), e se non c'è una motivazione forte (punti 4-5) ammettere che è un compromesso di tempo/scope per un progetto didattico.

---

## 8. Consigli pratici per l'orale

- **Studiate a fondo la vostra parte, capite a grandi linee tutte le altre.** Il docente valuta singolarmente; non basta sapere ripetere le slide, bisogna saper rispondere a "perché hai fatto così?" sulla propria parte.
- **Decidete prima chi risponde a cosa.** Se una domanda tocca più parti, chi ha più competenza risponde per primo, gli altri integrano se serve — non rispondete tutti insieme in disordine.
- **Fate almeno una prova generale ad alta voce**, cronometrata, usando le slide (`Presentazione_Georuggine.md`/`.pptx`) — la stima è ~11 minuti, verificate che regga per davvero con la vostra velocità di parlato.
- **Preparate la demo in anticipo**: server avviato, un utente già registrato pronto per il login, eventualmente un secondo terminale/client pronto per mostrare broadcast o messaggi privati. Non improvvisate la sequenza di comandi davanti al docente.
- **Se qualcosa va storto durante la demo dal vivo** (crash, connessione rifiutata), non fatevi prendere dal panico: spiegate cosa stava per succedere e passate oltre — meglio una demo imperfetta gestita con calma che uno stallo di due minuti in silenzio.
