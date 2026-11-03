<!--
Come usare questo documento:
- Ogni sezione tra `---` è una slide. Il titolo (##) diventa il titolo della
  slide, i bullet il contenuto visibile.
- I blocchi `::: notes ... :::` sono note per il relatore: Pandoc li converte
  nelle note-relatore reali di PowerPoint (visibili solo a chi presenta, non
  al pubblico). Questo commento HTML invece non compare mai nell'output
  (né nel .pptx né nell'anteprima renderizzata su GitHub) - è solo per chi
  legge il sorgente .md.

Conversione in .pptx:
  pandoc Presentazione_Georuggine.md -o Presentazione_Georuggine.pptx --slide-level=2

Timing totale stimato: ~11:15, per 4 relatori (uno per persona) più una slide
trasversale sulla GUI. Sono indicazioni, non cronometraggio rigido -
provatelo ad alta voce almeno una volta prima della consegna. Se dovete
rientrare stretti in 10 minuti, la slide più comprimibile è quella sulla GUI
(riepilogo visivo, va bene più veloce se l'interfaccia si è già vista prima).
-->

# Presentazione Georuggine

---

## Georuggine

**Sistema client/server per geolocalizzazione e comunicazione con una flotta di veicoli**

- Progetto di Programmazione in Rust — A.A. 2025-2026
- Gruppo 48 — Persona A, Persona B, Persona C, Persona D
- Stack: Rust, Tokio, egui, bcrypt, chrono, sysinfo

::: notes
**Relatore:** chi apre — 20s

Presentatevi rapidamente (nome + parte di cui parlerete), poi si passa subito alla panoramica. Non soffermatevi qui, è solo 15-20 secondi.
:::

---

## Panoramica e architettura

- Applicazione **client/server** in Rust puro, organizzata come **Cargo workspace** con 3 crate:
  - `common` — protocollo condiviso (`ClientMessage`/`ServerMessage`), costanti e funzioni geometriche
  - `server` — logica applicativa, concorrenza, persistenza
  - `client` — GUI (`egui`) + simulatore di movimento, più un client testuale (`simulator_client`)
- Ogni utente: si registra, effettua login, invia la propria posizione (emulata) ogni 30s, comunica via testo
- Il server traccia stato, storico posizioni e produce analisi statistiche su richiesta

::: notes
**Relatore:** chi apre — 50s

Questa slide serve a dare il contesto prima di entrare nel dettaglio per persona. Menzionate che il workspace a 3 crate è stato pensato apposta per condividere il protocollo tra client e server senza duplicarlo — un solo punto di verità per il formato dei messaggi.
:::

---

## [Persona A] Workspace e protocollo di comunicazione

- Cargo workspace con `common` come libreria condivisa (nessuna dipendenza di rete al suo interno)
- Protocollo testuale, una riga per messaggio, campi separati da `|` (es. `REGISTER|username|password`)
- Tipi tipizzati in Rust (`ClientMessage`, `ServerMessage`) con `encode()`/`decode()`, non stringhe libere
- Sanitizzazione dei campi testuali (niente `\n`/`\r` nel payload) per non spezzare il framing a righe

::: notes
**Relatore:** Persona A — 60s

Sottolineate che avere `ClientMessage`/`ServerMessage` come enum Rust — invece di stringhe grezze — permette al compilatore di garantire che client e server restino sincronizzati sul formato: se cambia un campo, tutti i punti che lo usano non compilano più finché non li aggiornate.
:::

---

## [Persona A] Server: connessioni, autenticazione, disconnessione

- `tokio::net::TcpListener` in ascolto sulla porta 7878, **un task asincrono per ogni client connesso**
- Registrazione: password con hash **bcrypt**, username validato (solo alfanumerico + `_`), persistenza su file JSON con scrittura atomica (temp file + rename)
- Login: verifica hash, protezione contro **doppio login dello stesso utente** da connessioni diverse
- Alla disconnessione: l'utente viene rimosso dalla tabella degli utenti attivi e il suo stato passa a **Disconnesso**

::: notes
**Relatore:** Persona A — 70s

Se vi chiedono della concorrenza, menzionate `Arc<RwLock<HashMap<...>>>` per gli utenti registrati e per gli utenti attivi — letture concorrenti libere, scritture esclusive solo su registrazione/login/disconnessione. Utile citare anche `spawn_blocking` per l'hashing bcrypt: è un'operazione volutamente lenta (anti brute-force), e va tenuta fuori dal thread asincrono principale per non bloccare gli altri client.
:::

---

## [Persona B] Macchina a stati (FSM)

- Ogni utente è in uno di tre stati: **Disconnesso**, **Fermo**, **In movimento**
- Transizione a **In movimento**: la nuova posizione dista più di 5 metri (formula di Haversine) dall'ultima registrata
- Transizione a **Fermo**: la posizione resta entro i 5 metri per almeno **3 minuti consecutivi**
- Soglia di distanza pensata come filtro contro il "tremolio" naturale delle coordinate GPS, per non scambiare rumore di misura per movimento reale

::: notes
**Relatore:** Persona B — 70s

Se il docente fa notare che la specifica dice "al primo cambiamento di coordinata", spiegate che è una scelta di robustezza deliberata e documentata nel manuale del progettista — con coordinate emulate al millimetro, un'uguaglianza esatta renderebbe lo stato "Fermo" quasi irraggiungibile con il simulatore random-walk. Le stesse soglie (5m, 3min) sono condivise col modulo di analisi, così stato live e report statistici raccontano sempre la stessa storia.
:::

---

## [Persona B] Simulatore di movimento e invio posizione

- Lato client, due strategie di generazione posizioni (oltre all'inserimento manuale da GUI):
  - **Random walk**: piccoli spostamenti pseudo-casuali da un punto centrale
  - **Da file CSV**: sequenza di coordinate lette da file (`lat,lon` per riga)
- Le posizioni vengono inviate al server **ogni 30 secondi**, sia dal client GUI sia dal client testuale
- Storico posizioni per utente mantenuto lato server (timestamp + coordinate), usato sia dalla FSM che dall'analisi

::: notes
**Relatore:** Persona B — 60s

Se avete tempo, questo è il punto naturale per una breve demo live — avviare il server, far partire un client con random walk, mostrare lo stato che passa a "IN MOVIMENTO" in console. Se non c'è tempo per la demo qui, rimandatela alla slide finale.
:::

---

## [Persona C] Analisi del movimento

- Su richiesta, il server calcola per l'utente autenticato:
  - **Distanza totale percorsa** (somma di distanze Haversine tra punti consecutivi)
  - **Velocità media** (distanza / tempo in movimento)
  - **Durata del movimento** e **durata delle pause**, in secondi
- Formula di Haversine: calcola la distanza reale tra due coordinate geografiche su una sfera (la Terra), non in linea retta euclidea
- Punti con timestamp fuori sequenza vengono riordinati prima del calcolo; timestamp futuri o coordinate non valide vengono scartati a monte

::: notes
**Relatore:** Persona C — 70s

Se chiedono "posso analizzare il movimento di un altro utente?", la risposta onesta è no — il protocollo attuale fa sì che ogni utente possa richiedere solo l'analisi dei **propri** spostamenti (l'utente della sessione autenticata). Non è prevista una funzione amministrativa cross-utente nella versione attuale.
:::

---

## [Persona C] Intervalli temporali e protocollo di richiesta

- Tre intervalli selezionabili: **giorno**, **settimana**, **mese** — sempre quelli **correnti** (da mezzanotte di oggi, da lunedì di questa settimana, dal primo del mese), calcolati con `chrono`, non finestre mobili di 24h/7gg/30gg
- Messaggio di richiesta: `REQANALYSIS|DAY` (o `WEEK`/`MONTH`)
- Risposta del server: risultato strutturato e tipizzato (non testo libero), con distanza, velocità, tempo movimento e tempo pausa

::: notes
**Relatore:** Persona C — 50s

Enfatizzate la differenza tra "finestra mobile" e "periodo di calendario corrente" — è un dettaglio a cui i docenti di sistemi spesso fanno caso, e l'avete gestito correttamente.
:::

---

## [Persona D] Messaggistica: broadcast, diretta, verso il server

- Ogni utente può inviare messaggi **broadcast** (a tutti i connessi) o **privati** (a un utente specifico), tramite lo stesso comando `TEXT`
- Canale `broadcast` di Tokio per i messaggi a tutti, con filtro anti-eco (il mittente non riceve il proprio messaggio); canali `mpsc` dedicati per utente per i messaggi privati
- Target riservato `SERVER`: un utente può scrivere direttamente all'operatore del server (comunicazione utente → server richiesta dalla traccia)
- L'operatore del server ha una console interattiva (`b <msg>`, `u <utente> <msg>`, `lista`) per rispondere o avviare comunicazioni broadcast/dirette verso i client

::: notes
**Relatore:** Persona D — 60s

Se chiedono un esempio pratico del canale SERVER, potete citare lo scenario dato dal docente: l'operatore che avvisa tutti di "fare rifornimento" (broadcast) o dice a un singolo autista "sbrigati, il cliente aspetta" (diretto).
:::

---

## [Persona D] Prestazioni: log periodico del tempo di CPU

- Ogni **~2 minuti**, il server scrive su file (`server_cpu.log`) il tempo di CPU realmente consumato dal **proprio processo** (non un aggregato di tutto il sistema)
- Misurato con la libreria `sysinfo`, interrogando il processo per il proprio PID
- Riportati sia il tempo di CPU totale accumulato sia la percentuale di utilizzo istantanea, come dato di contesto aggiuntivo

::: notes
**Relatore:** Persona D — 40s

Se chiedono perché non la percentuale di sistema, spiegate che misurare la CPU dell'intero sistema includerebbe anche altri programmi in esecuzione sulla stessa macchina — un dato non richiesto e potenzialmente fuorviante. La consegna chiede esplicitamente il tempo di CPU "da parte del server".
:::

---

## [Persona D] Multipiattaforma e dimensione degli eseguibili

- Build e test eseguiti su **Windows** e **macOS** *(confermare quale seconda piattaforma avete effettivamente testato prima di presentare questa slide)*
- Dimensioni degli eseguibili in release *(aggiornare con le misure definitive prima della consegna)*:

| Eseguibile | Windows | macOS |
|---|---|---|
| `server` | ~1,1 MB | ~1,6 MB |
| `simulator_client` | ~0,7 MB | ~1,1 MB |
| `client` (GUI) | ~15 MB | ~13,7 MB |

- Il client GUI è più pesante per via delle dipendenze grafiche (`egui`/`wgpu`, accelerazione hardware nativa); server e client testuale restano molto compatti

::: notes
**Relatore:** Persona D — 40s

**Verificate questi numeri prima di presentare** — rifateli con `cargo build --release` sulle macchine reali che userete per la demo, non fidatevi ciecamente di questa tabella. Se non avete davvero testato su macOS, ditelo con onestà e sostituite con la piattaforma che avete testato per davvero.
:::

---

## L'interfaccia grafica (GUI)

- Client con GUI nativa in `egui` (immediate-mode, stesso codice sorgente su Windows/Linux/macOS)
- Layout a due colonne:
  - **Sinistra**: autenticazione, invio posizione manuale, controlli del simulatore (Random Walk / CSV), richiesta analisi (Giorno/Settimana/Mese), log di sistema
  - **Destra**: chat a tab — Broadcast, una tab per ogni conversazione privata (creata automaticamente al primo messaggio scambiato), tab riservata al Server
- Validazione e stato dei controlli riflettono lo stato dell'app in tempo reale: coordinate non numeriche bloccate prima dell'invio, bottone "Avvia simulatore" disabilitato finché non si è loggati
- Thread di rete **separato** dal thread di rendering: la UI legge gli eventi già pronti da una coda interna (`GuiEvent`), non si blocca mai in attesa di risposte dal server

::: notes
**Relatore:** da assegnare — è trasversale a più parti, sceglietevi in base a chi guida la demo — 55s

È la slide "ponte" tra il dettaglio tecnico appena spiegato e la demo che segue — mostra come i contributi di tutti e 4 confluiscono in un'unica interfaccia (login di Persona A, simulatore/posizione di Persona B, bottoni di analisi di Persona C, chat di Persona D). Se chiedono perché il thread di rete è separato: un'interfaccia grafica non deve mai congelarsi in attesa di I/O di rete, quindi il rendering a schermo non aspetta mai direttamente sul socket.
:::

---

## Conclusioni e demo

- Sistema completo: autenticazione, tracciamento posizione con macchina a stati, simulazione movimento, analisi statistiche, messaggistica multi-canale, logging delle prestazioni
- Documentazione: manuale utente, manuale del progettista, piano di test, changelog delle scelte tecniche
- **Demo live** (se il tempo lo consente): avvio server → registrazione/login da GUI → simulatore in movimento → richiesta di analisi → scambio messaggi
- Domande?

::: notes
**Relatore:** chi chiude, o tutti e 4 insieme per le domande — 30s

Decidete PRIMA chi risponde a quali domande in base alla parte di competenza (rimandatevi la palla a vicenda invece di rispondere tutti insieme). Se qualcuno fa una domanda su un punto che avete deliberatamente scelto di interpretare in un certo modo (es. soglia FSM, analisi solo per sé stessi), è meglio ammetterlo apertamente che arrampicarsi — è già tutto documentato nel manuale del progettista.
:::
