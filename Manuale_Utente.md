# Georuggine — Manuale Utente

Guida pratica all'uso dell'applicazione Georuggine: registrazione, invio della posizione, simulazione del movimento, messaggistica e analisi degli spostamenti.

---

## 1. Cos'è Georuggine

Georuggine è un sistema client/server per la gestione di una flotta di veicoli:

- ogni utente si registra e accede con un account;
- la posizione di ogni utente viene tracciata (in modo emulato, non con GPS reale) e trasmessa periodicamente al server;
- il server tiene traccia dello stato di ogni utente (**sconnesso**, **fermo**, **in movimento**) e può calcolare statistiche sui suoi spostamenti;
- utenti e server possono scambiarsi messaggi di testo.

Il progetto è composto da tre programmi (Cargo li chiama "binari"):

| Binario | Cosa fa |
|---|---|
| `server` | Il server centrale: gestisce account, posizioni, messaggistica e analisi. |
| `client` | Client con interfaccia grafica (GUI), pensato per l'uso normale. |
| `simulator_client` | Client testuale da riga di comando, utile per test rapidi o per ambienti senza interfaccia grafica. |

---

## 2. Compilazione e avvio

Dalla radice del progetto:

```
cargo build --release
```

Gli eseguibili vengono generati in `target/release/` (`server.exe`, `client.exe`, `simulator_client.exe` su Windows; senza estensione su Linux/macOS).

### Avviare il server

```
cargo run --release --bin server
```

oppure, dopo la build, eseguire direttamente `target/release/server`. Il server si mette in ascolto sulla porta **7878** su tutte le interfacce di rete. In console vedrai:

```
Server Georuggine in ascolto sulla porta 7878...
Console server attiva. Comandi: b <msg> | u <user> <msg> | lista
```

Il server deve restare in esecuzione per tutta la sessione: client e simulatori si connettono a `127.0.0.1:7878` (stessa macchina). Per connettersi da un'altra macchina della rete, va usato l'indirizzo IP del server al posto di `127.0.0.1` (il codice del client andrebbe adattato per accettare un indirizzo diverso da quello di default).

### Avviare il client grafico

```
cargo run --release --bin client
```

### Avviare il client testuale

```
cargo run --release --bin simulator_client
```

---

## 3. File generati dal server

Il server crea due file **nella stessa cartella dell'eseguibile** (es. `target/release/` se lanciato da lì):

| File | Contenuto |
|---|---|
| `utenti_georuggine.json` | Database degli utenti registrati (username + password con hash bcrypt — mai in chiaro). |
| `server_cpu.log` | Una riga ogni ~2 minuti con l'utilizzo di CPU del server. |

Per "svuotare" gli utenti registrati basta cancellare `utenti_georuggine.json` e riavviare il server: ripartirà con un database vuoto. Se il file risultasse danneggiato (es. per un arresto improvviso durante la scrittura), il server non lo cancella: lo rinomina in `utenti_georuggine.json.corrotto` per permetterti di recuperarlo a mano, e riparte comunque con un database vuoto.

---

## 4. Usare il client grafico

All'avvio la finestra è divisa in due colonne: **controlli** a sinistra, **chat** a destra.

### 4.1 Registrazione e login

Nella sezione "🔑 Autenticazione":

1. Inserisci **Username** e **Password**.
2. Premi **Registrati** per creare un nuovo account, oppure **Login** se l'account esiste già.

Regole sull'username: solo lettere, numeri e underscore (`_`), lunghezza tra 1 e 32 caratteri. Il nome `SERVER` è riservato e non può essere usato per un account (è il destinatario speciale per i messaggi all'operatore, vedi §4.4).

Dopo il login la sezione di autenticazione scompare e diventano disponibili posizione, simulatore e analisi.

### 4.2 Invio manuale della posizione

Nella sezione "📍 Posizione (manuale)", inserisci Latitudine e Longitudine (numeri decimali, es. `45.07` e `7.68`) e premi **Invia Posizione**. Se i valori non sono numeri validi compare un messaggio di errore in rosso e nulla viene inviato.

Coordinate valide: latitudine tra -90 e 90, longitudine tra -180 e 180. Il server rifiuta posizioni fuori da questi limiti o con timestamp palesemente nel futuro.

### 4.3 Simulatore di movimento

Nella sezione "🚗 Simulatore" puoi scegliere come generare automaticamente le posizioni:

- **Off** — nessuna simulazione, solo invio manuale.
- **🚶 Random Walk** — genera posizioni pseudo-casuali a partire da un punto centrale (di default: Torino), con piccoli spostamenti ad ogni invio.
- **📂 Da file CSV** — legge una sequenza di posizioni da un file di testo.

Se scegli "Da file CSV", nel campo **CSV** puoi scrivere:

- solo il nome del file (es. `sample_route.csv`) se il file si trova nella cartella del progetto client, **oppure**
- un percorso relativo alla cartella da cui hai lanciato il programma, **oppure**
- un percorso assoluto (es. `C:\percorso\mio_file.csv`).

Formato del file CSV: una posizione per riga, `latitudine,longitudine` (es. `45.070,7.680`). Righe vuote o che iniziano con `#` sono ignorate come commenti. Righe con valori non numerici o fuori dai range geografici validi vengono scartate con un avviso in console, senza interrompere il caricamento delle righe restanti.

Premi **▶ Avvia** per far partire l'invio automatico (una posizione ogni 30 secondi); il bottone è disabilitato finché non hai fatto login. Premi **⏹ Ferma** per interromperlo in qualsiasi momento. Con la strategia "Da file CSV", il simulatore si ferma automaticamente quando le posizioni del file sono finite.

### 4.4 Chat

Sulla destra trovi tre tipi di conversazione, organizzate in tab:

- **📢 Tutti** — messaggi broadcast, visti da tutti gli utenti connessi (tranne te, che non ricevi eco del tuo stesso messaggio).
- **🔒 &lt;nome utente&gt;** — conversazione privata con un altro utente. La tab compare automaticamente al primo messaggio scambiato con quell'utente; puoi anche iniziarne una tu digitando lo username nel campo **A:** in basso.
- **🖥 Server** — canale riservato per scrivere direttamente all'operatore del server (non ad altri utenti). Clicca la tab per impostare automaticamente il destinatario, scrivi il messaggio e premi **Invia**.

Per inviare: scrivi nel campo di testo in basso e premi **Invia** oppure Invio da tastiera. Il campo **A:** indica il destinatario: vuoto (o "tutti") per il broadcast, uno username per un messaggio privato, oppure resta su "SERVER" se sei nella tab Server.

### 4.5 Analisi del movimento

Nella sezione "📈 Analisi movimento", premi **Giorno**, **Settimana** o **Mese** per richiedere al server un report sui tuoi spostamenti in quell'intervallo (giorno/settimana/mese **corrente**, cioè da mezzanotte di oggi, da lunedì di questa settimana, dal primo del mese, non le ultime 24/168/720 ore). Il risultato compare nel log di sistema:

```
📊 Analisi mario (giorno): 1.23 km | 5.67 km/h | mov: 3600s | pausa: 600s
```

I quattro valori sono: distanza totale percorsa, velocità media, secondi in movimento, secondi in pausa. Se non hai ancora inviato nessuna posizione in quell'intervallo, il server risponde che non ci sono dati disponibili.

### 4.6 Log di sistema

Sotto il simulatore, il pannello "📋 Log di sistema" mostra le risposte del server che non sono messaggi di chat: conferme di login/registrazione, ack di posizione, esiti delle analisi, errori.

### 4.7 Altri controlli

- **🌙 / ☀️** in alto a destra cambiano il tema (scuro/chiaro) dell'interfaccia.
- **🚪 Esci** chiude immediatamente l'applicazione.

---

## 5. Usare il client testuale (`simulator_client`)

Pensato per test rapidi da terminale, senza interfaccia grafica. All'avvio si connette al server e guida l'utente in tre fasi.

### Fase 1 — Autenticazione

```
--- Autenticazione ---
Comandi disponibili:
  REGISTER|username|password
  LOGIN|username|password
Inserisci un comando per procedere:
```

Digita uno dei due comandi (esattamente in questo formato, con le barre verticali `|` come separatori) e premi Invio. Il programma resta in questa fase finché non ricevi conferma di login.

### Fase 2 — Scelta della simulazione

```
--- Simulatore di movimento ---
Scegli una strategia:
  1) Carica posizioni da file CSV
  2) Random walk (movimento pseudo-casuale)
  3) Nessuna simulazione (solo comandi manuali)
```

Digita `1`, `2` o `3`. Con l'opzione 1 ti verrà chiesto il percorso del file CSV (stesse regole di risoluzione del percorso descritte al §4.3).

### Fase 3 — Sessione attiva

Da qui in poi puoi digitare liberamente i comandi del protocollo:

```
TEXT|ALL|messaggio          → broadcast a tutti
TEXT|username|messaggio     → messaggio privato
TEXT|SERVER|messaggio       → messaggio all'operatore del server
REQANALYSIS|DAY/WEEK/MONTH  → richiedi analisi movimento
POS|lat|lon|timestamp       → invio posizione manuale (timestamp in secondi, epoch Unix)
```

Le risposte del server compaiono precedute da `<< Server:`. Se hai scelto una strategia di simulazione, le posizioni vengono inviate automaticamente ogni 30 secondi in parallelo ai comandi che digiti.

---

## 6. Console dell'operatore del server

Chi fa girare il server ha a disposizione una piccola console interattiva, digitando direttamente nel terminale dove il server è in esecuzione (nessun client necessario):

| Comando | Effetto |
|---|---|
| `b <messaggio>` | Invia un messaggio broadcast a tutti i client connessi. |
| `u <username> <messaggio>` | Invia un messaggio privato a un utente specifico (compare nella sua tab "🖥 Server"). |
| `lista` | Mostra l'elenco degli utenti attualmente connessi. |

Esempio:

```
b Ricordatevi di fare rifornimento
[Console] Broadcast inviato ai client.
u mario Il cliente sta aspettando, sbrigati
[Console] Messaggio inviato a 'mario'.
lista
[Console] Utenti connessi: mario, anna
```

---

## 7. Stati dell'utente e soglie

Ogni utente connesso si trova in uno di tre stati:

- **Disconnesso** — nessuna sessione attiva.
- **In movimento** — la posizione è cambiata di più di **5 metri** rispetto all'ultima registrata.
- **Fermo** — la posizione è rimasta entro i 5 metri per almeno **3 minuti consecutivi**.

Queste soglie (5 metri, 3 minuti) sono le stesse usate sia per lo stato mostrato in tempo reale sia per il calcolo di "tempo in movimento" / "tempo in pausa" nelle analisi, quindi i due numeri sono sempre coerenti tra loro.

---

## 8. Domande frequenti / problemi comuni

**"Impossibile connettersi al Server!"** — il server non è in esecuzione, oppure è in ascolto su una macchina diversa da quella indicata (di default `127.0.0.1`, cioè la stessa macchina). Avvia il server prima del client.

**"Username non valido o riservato"** — l'username contiene caratteri non ammessi (solo lettere, numeri e `_`), è vuoto, supera i 32 caratteri, oppure hai provato a registrarti come `SERVER` (nome riservato).

**"Utente già connesso da un'altra sessione"** — quello username ha già una sessione attiva altrove; disconnettila prima di rifare login, oppure usa un altro account.

**"Posizione rifiutata: ..."** — le coordinate inviate sono fuori dai range geografici validi, non sono numeri finiti, oppure il timestamp è troppo avanti nel futuro rispetto all'orologio del server (tolleranza: 60 secondi, utile per piccoli sfasamenti tra gli orologi di client e server).

**Il simulatore da file CSV non parte / "Errore nel caricare il file"** — controlla che il percorso sia corretto (vedi §4.3) e che il file contenga almeno una riga con coordinate valide nel formato `lat,lon`.

**Il bottone "▶ Avvia" del simulatore è disabilitato** — devi prima effettuare il login.

**Messaggi troppo lunghi** — il protocollo accetta righe fino a 4096 byte; oltre questo limite il server chiude la connessione con un avviso, a protezione da client malformati o malevoli.
