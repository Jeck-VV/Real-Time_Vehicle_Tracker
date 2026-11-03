# Georuggine — Piano di Test

Elenco dei casi di test per coprire tutte le funzionalità del sistema, organizzati per area.
Ogni riga indica: cosa provare, l'input esatto da usare, e il risultato atteso.

---

## 1. Autenticazione

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 1.1 | Registrazione valida | `REGISTER\|anto\|1234` | `REGISTER_OK` |
| 1.2 | Registrazione utente duplicato | `REGISTER\|anto\|1234` (ripetuto) | `REGISTER_ERR\|Utente già esistente` |
| 1.3 | Login con credenziali corrette | `LOGIN\|anto\|1234` | `LOGIN_OK` |
| 1.4 | Login con password sbagliata | `LOGIN\|anto\|password_sbagliata` | `LOGIN_ERR\|Credenziali non valide` |
| 1.5 | Login con utente inesistente | `LOGIN\|utente_mai_registrato\|1234` | `LOGIN_ERR\|Credenziali non valide` |
| 1.6 | Registrazione con password vuota | `REGISTER\|anto2\|` | Verificare comportamento (accettata o rifiutata?) |
| 1.7 | Registrazione con username vuoto | `REGISTER\|\|1234` | Verificare comportamento |
| 1.8 | Persistenza: registra, riavvia server, rifai login | Register → stop server → restart → Login | `LOGIN_OK` (utente ancora presente) |
| 1.9 | Doppia registrazione simultanea stesso username (2 client) | Due client mandano `REGISTER\|stesso_nome\|pw` quasi in contemporanea | Solo uno riceve `REGISTER_OK`, l'altro `REGISTER_ERR` |

## 2. Protocollo / parsing messaggi

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 2.1 | Comando sconosciuto | `PIPPO\|test` | `NOTICE\|Comando non riconosciuto: ...` |
| 2.2 | Messaggio con spazi iniziali | `   REGISTER\|anto3\|1234` | Deve funzionare comunque (grazie al `trim()`) |
| 2.3 | Messaggio vuoto | (riga vuota) | Gestito senza crash del server |
| 2.4 | REGISTER con campi mancanti | `REGISTER\|soloUsername` | Errore di parsing gestito, nessun crash |
| 2.5 | POS con valori non numerici | `POS\|abc\|xyz\|123` | Errore di parsing gestito (via terminale); da GUI: bloccato prima dell'invio con messaggio rosso |

## 3. Posizione e macchina a stati (FSM)

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 3.1 | Prima posizione inviata da un utente | `POS\|45.07\|7.68\|<timestamp attuale>` | Stato → `IN MOVIMENTO` |
| 3.2 | Seconda posizione diversa dalla prima | `POS\|45.10\|7.70\|<timestamp+X>` | Stato resta/torna `IN MOVIMENTO` |
| 3.3 | Stessa posizione ripetuta, meno di 3 minuti | 2 invii identici con timestamp distanti <180s | Stato resta `IN MOVIMENTO` |
| 3.4 | Stessa posizione ripetuta, almeno 3 minuti | 2 invii identici con timestamp distanti ≥180s | Stato → `FERMO` |
| 3.5 | Da fermo, invio posizione diversa | Dopo 3.4, invia coordinate diverse | Stato → `IN MOVIMENTO` di nuovo |
| 3.6 | Disconnessione | Chiudi il client dopo il login | Log server: `[FSM] Utente disconnesso → stato: Disconnesso` |
| 3.7 | Riconnessione dopo disconnessione | Richiudi e riapri client, rifai login | Stato riparte da `FERMO`/`IN MOVIMENTO`, storico precedente mantenuto |
| 3.8 | Invio posizione senza login | `POS\|45\|7\|123` prima del login | `NOTICE\|Devi fare il login per inviare la posizione` |

## 4. Messaggistica

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 4.1 | Broadcast a tutti | `TEXT\|ALL\|ciao a tutti` (o campo destinatario vuoto in GUI) | Tutti gli altri client connessi lo ricevono |
| 4.2 | Il mittente non riceve il proprio broadcast | Dopo 4.1, controlla il log del mittente | Nessun eco del proprio messaggio |
| 4.3 | Messaggio privato a utente online | `TEXT\|utente2\|messaggio privato` | Solo `utente2` lo riceve |
| 4.4 | Messaggio privato a utente offline/inesistente | `TEXT\|utente_non_connesso\|ciao` | `NOTICE\|Errore: l'utente ... non è online` |
| 4.5 | Messaggio senza login | `TEXT\|ALL\|test` prima del login | `NOTICE\|Devi fare il login per scrivere` |
| 4.6 | Messaggio con contenuto vuoto | `TEXT\|ALL\|` | Verificare comportamento (inviato vuoto o rifiutato?) |
| 4.7 | Messaggio con `\|` nel contenuto | `TEXT\|ALL\|prova con | pipe dentro` | Il contenuto va oltre il primo `\|` (verificare che non spezzi il parsing) |

## 5. Analisi movimento

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 5.1 | Analisi senza dati di posizione | `REQANALYSIS\|DAY` (utente senza mai inviato POS) | `Nessun dato di posizione disponibile` |
| 5.2 | Analisi con una sola posizione | `REQANALYSIS\|DAY` dopo un solo `POS` | Distanza/velocità/tempo a 0 (serve almeno 2 punti) |
| 5.3 | Analisi con timestamp fuori dall'intervallo | `POS` con timestamp vecchio di anni, poi `REQANALYSIS\|DAY` | Filtrato, risultato a 0 |
| 5.4 | Analisi con dati validi, intervallo Day | 2+ `POS` con timestamp recenti, poi `REQANALYSIS\|DAY` | Distanza/velocità/tempo coerenti coi punti inviati |
| 5.5 | Analisi con dati validi, intervallo Week | Come sopra, `REQANALYSIS\|WEEK` | Include anche dati più vecchi (fino a 7gg) |
| 5.6 | Analisi con dati validi, intervallo Month | Come sopra, `REQANALYSIS\|MONTH` | Include dati fino a 30gg |
| 5.7 | Pausa registrata correttamente dopo ripartenza | Ferma >3min, poi riparti, poi chiedi analisi | `pausa` riflette il tempo fermo, non torna a 0 |
| 5.8 | Analisi senza login | `REQANALYSIS\|DAY` prima del login | `NOTICE\|Devi fare il login per chiedere l'analisi` |
| 5.9 | Coerenza numerica | Confronta manualmente distanza Haversine attesa tra 2 punti noti vs risultato | Scarto minimo, solo arrotondamento |

## 6. GUI

| # | Caso | Azione | Risultato atteso |
|---|---|---|---|
| 6.1 | Login da GUI | Compila campi, premi Login | Passa alla vista principale |
| 6.2 | Validazione posizione non numerica | Scrivi "abc" in Lat, premi Invia Posizione | Messaggio rosso di errore, nessun invio |
| 6.3 | Simulatore random walk | Seleziona "Random Walk", premi Avvia | Posizioni inviate automaticamente ogni 30s |
| 6.4 | Simulatore da file CSV | Seleziona "Da file CSV", inserisci percorso, Avvia | Posizioni lette dal file, inviate in sequenza |
| 6.5 | Simulatore con file CSV inesistente | Percorso file sbagliato, Avvia | Messaggio di errore nel log, simulatore non parte |
| 6.6 | Stop simulatore | Premi Ferma durante l'esecuzione | Il simulatore si interrompe, bottone torna disabilitato |
| 6.7 | Tab chat: broadcast | Ricevi un messaggio broadcast | Compare nella tab "Tutti" |
| 6.8 | Tab chat: privato | Ricevi un messaggio privato da un utente nuovo | Si crea una nuova tab con quel nome |
| 6.9 | Tab chat: server | Ricevi un messaggio dalla console server (`u`) | Compare nella tab "SERVER" |
| 6.10 | Cambio tema | Premi 🌙 e ☀️ | L'interfaccia cambia colore |
| 6.11 | Reattività durante l'attesa | Interagisci con la GUI mentre il simulatore gira | Nessun blocco/freeze percepibile |

## 7. Console del server

| # | Caso | Comando | Risultato atteso |
|---|---|---|---|
| 7.1 | Lista utenti connessi | `lista` | Elenco corretto degli utenti attualmente online |
| 7.2 | Lista senza utenti connessi | `lista` (nessun client) | `Nessun utente connesso` |
| 7.3 | Broadcast da console | `b messaggio di prova` | Tutti i client connessi lo ricevono |
| 7.4 | Messaggio privato da console | `u anto ciao` | Solo `anto` lo riceve, in tab SERVER |
| 7.5 | Messaggio privato a utente non connesso | `u utente_inesistente ciao` | `Utente ... non connesso` |
| 7.6 | Comando malformato | `xyz` | `Comando non riconosciuto` |

## 8. Prestazioni e requisiti tecnici

| # | Caso | Come verificare | Risultato atteso |
|---|---|---|---|
| 8.1 | Log CPU ogni 2 minuti | Lascia il server acceso ~5 min, controlla `server_cpu.log` | Una riga ogni ~121s |
| 8.2 | Percorso file indipendente dalla working directory | Lancia il server da cartelle diverse (`cd server` vs radice repo) | Il file utenti/log è sempre nello stesso posto |
| 8.3 | Build in release | `cargo build --release` | Compila senza errori |
| 8.4 | Dimensione eseguibile | Controlla dimensione file in `target/release/` | Annotare il valore per il report |
| 8.5 | Build su seconda piattaforma | `cargo build` su Linux/macOS (o WSL) | Compila ed esegue correttamente |
| 8.6 | Molti client connessi contemporaneamente | Avvia 5+ client insieme | Nessun crash, tutti autenticabili e funzionanti |

## 9. Casi limite / robustezza

| # | Caso | Input | Risultato atteso |
|---|---|---|---|
| 9.1 | Coordinate ai limiti geografici | `POS\|90\|180\|<ts>` e `POS\|-90\|-180\|<ts>` | Accettate, nessun crash nel calcolo Haversine |
| 9.2 | Timestamp futuro | `POS` con timestamp nel futuro rispetto a "ora" | Verificare che non generi calcoli assurdi (tempo negativo ecc.) |
| 9.3 | Disconnessione improvvisa (Ctrl+C) durante l'invio | Chiudi il client bruscamente mentre manda dati | Il server rileva la disconnessione senza crash |
| 9.4 | Riavvio server con client ancora connessi | Ferma e riavvia il server mentre un client è collegato | Il client rileva "connessione chiusa dal server" |

