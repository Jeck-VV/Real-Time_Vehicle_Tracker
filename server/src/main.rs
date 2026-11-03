use common::{BroadcastEvent, ClientMessage, Posizione, ServerMessage, TipoEvento, UserState};
use server::{posizione_valida, username_valido, UserSessionData};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

// Librerie per log della cpu
use chrono::Local;
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::fs::OpenOptions;

const SERVER_TARGET: &str = "SERVER";

// Canale broadcast per comunicazione e messaggi
use tokio::sync::{broadcast, mpsc};

// Librerie per la persistenza
use serde::{Deserialize, Serialize};
use std::fs;

// Dati di un utente registrato.
#[derive(Serialize, Deserialize, Clone)]
struct User {
    password_hash: String,
}

fn directory_dati() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn percorso_db() -> std::path::PathBuf {
    directory_dati().join("utenti_georuggine.json")
}

// Carica il DB all'avvio
fn carica_utenti() -> HashMap<String, User> {
    let path = percorso_db();
    match fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str::<HashMap<String, User>>(&data) {
            Ok(utenti_salvati) => {
                println!("Caricati {} utenti dal database.", utenti_salvati.len());
                utenti_salvati
            }
            Err(e) => {
                eprintln!("ATTENZIONE: database utenti corrotto ({e}).");
                let backup = path.with_extension("json.corrotto");
                match fs::rename(&path, &backup) {
                    Ok(()) => eprintln!("File corrotto conservato in: {}", backup.display()),
                    Err(err) => eprintln!("Impossibile mettere da parte il file corrotto: {err}"),
                }
                HashMap::new()
            }
        },
        Err(_) => {
            println!("Nessun database trovato, parto da zero.");
            HashMap::new()
        }
    }
}

fn salva_utenti(db: &HashMap<String, User>) {
    match serde_json::to_string_pretty(db) {
        Ok(json_data) => {
            let path = percorso_db();
            let path_temp = path.with_extension("json.tmp");
            if let Err(e) = fs::write(&path_temp, json_data) {
                eprintln!("Errore nel salvataggio del database utenti (file temporaneo): {e}");
                return;
            }
            if let Err(e) = fs::rename(&path_temp, &path) {
                eprintln!("Errore nel salvataggio del database utenti (rename): {e}");
            }
        }
        Err(e) => eprintln!("Errore nella serializzazione del database utenti: {e}"),
    }
}

type UserDb = Arc<RwLock<HashMap<String, User>>>;
type ActiveUsers = Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>;
type UserSessions = Arc<RwLock<HashMap<String, UserSessionData>>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::spawn(async move {
        // La consegna chiede il tempo di CPU "utilizzato da parte del server":
        // il tempo CPU del PROCESSO server, non un aggregato di tutto il
        // sistema (che includerebbe anche altri programmi in esecuzione sulla
        // stessa macchina, dato non richiesto e fuorviante).
        let pid =
            sysinfo::get_current_pid().expect("impossibile ottenere il PID del processo server");
        let mut sys = System::new();
        let refresh_cpu = ProcessRefreshKind::nothing().with_cpu();

        loop {
            // Prima misura: stabilisce la baseline per il calcolo del delta di CPU.
            sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh_cpu);
            tokio::time::sleep(Duration::from_secs(1)).await;
            // Seconda misura un secondo dopo: sysinfo calcola cpu_usage() come
            // differenza tra i due refresh, quindi ora riflette l'uso reale
            // nell'ultimo secondo.
            sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh_cpu);

            if let Some(processo) = sys.process(pid) {
                // accumulated_cpu_time(): tempo di CPU totale (ms) consumato dal
                // server da quando è partito — è la metrica richiesta dalla
                // consegna. cpu_usage(): percentuale istantanea del processo
                // (può superare il 100% su macchine multi-core), mostrata in
                // aggiunta come dato di contesto.
                let tempo_cpu_totale_sec = processo.accumulated_cpu_time() as f64 / 1000.0;
                let cpu_percentuale = processo.cpu_usage();

                let ora_attuale = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                let log_line = format!(
                    "[{}] Tempo CPU totale server: {:.3}s | Utilizzo CPU istantaneo: {:.2}%\n",
                    ora_attuale, tempo_cpu_totale_sec, cpu_percentuale
                );

                println!("(Logger CPU) - {}", log_line.trim());

                match OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(directory_dati().join("server_cpu.log"))
                    .await
                {
                    Ok(mut file) => {
                        if let Err(e) = file.write_all(log_line.as_bytes()).await {
                            eprintln!("Errore nella scrittura del log CPU: {e}");
                        }
                    }
                    Err(e) => eprintln!("Errore nell'apertura del file di log CPU: {e}"),
                }
            } else {
                eprintln!("Impossibile leggere i dati del processo server (pid {pid})");
            }

            // Aspetta 2 min prima di ripetere il log
            tokio::time::sleep(Duration::from_secs(119)).await;
        }
    });

    let listener = TcpListener::bind("0.0.0.0:7878").await?;
    let initial_users = carica_utenti();
    let users: UserDb = Arc::new(RwLock::new(initial_users));
    let active_users: ActiveUsers = Arc::new(RwLock::new(HashMap::new()));
    let user_sessions: UserSessions = Arc::new(RwLock::new(HashMap::new()));

    println!("Server Georuggine in ascolto sulla porta 7878...");
    let (tx, _rx) = broadcast::channel::<BroadcastEvent>(100);

    // -----------------------------------------------------------------------
    // Console interattiva del server: permette all'operatore di inviare
    // messaggi broadcast (a tutti) o unicast (a un singolo utente) senza
    // bisogno di un client connesso.
    //
    // Comandi disponibili:
    //   b <messaggio>            → broadcast a tutti i client connessi
    //   u <username> <messaggio> → messaggio privato a un utente specifico
    //   lista                    → mostra gli utenti attualmente connessi
    // -----------------------------------------------------------------------
    {
        let tx_console = tx.clone();
        let active_users_console = active_users.clone();

        tokio::spawn(async move {
            println!("Console server attiva. Comandi: b <msg> | u <user> <msg> | lista");

            let mut stdin = BufReader::new(tokio::io::stdin()).lines();

            while let Ok(Some(line)) = stdin.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                if line == "lista" {
                    let actives = active_users_console.read().await;
                    if actives.is_empty() {
                        println!("[Console] Nessun utente connesso.");
                    } else {
                        println!(
                            "[Console] Utenti connessi: {}",
                            actives.keys().cloned().collect::<Vec<_>>().join(", ")
                        );
                    }
                } else if let Some(msg) = line.strip_prefix("b ") {
                    // Broadcast a tutti i client connessi
                    let event = BroadcastEvent {
                        mittente: "SERVER".into(),
                        tipo: TipoEvento::MessaggioTesto {
                            contenuto: msg.to_string(),
                        },
                    };
                    match tx_console.send(event) {
                        Ok(_) => println!("[Console] Broadcast inviato ai client."),
                        Err(_) => println!("[Console] Nessun client connesso."),
                    }
                } else if let Some(rest) = line.strip_prefix("u ") {
                    // Unicast a un utente specifico: "u <username> <messaggio>"
                    let mut parts = rest.splitn(2, ' ');
                    let username = parts.next().unwrap_or("").trim();
                    let msg = parts.next().unwrap_or("").trim();

                    if username.is_empty() || msg.is_empty() {
                        println!("[Console] Uso: u <username> <messaggio>");
                        continue;
                    }

                    let canale = {
                        let actives = active_users_console.read().await;
                        actives.get(username).cloned()
                    };
                    match canale {
                        Some(ch) => {
                            let payload = format!("[SERVER] {}", msg);
                            if ch.send(payload).await.is_ok() {
                                println!("[Console] Messaggio inviato a '{username}'.");
                            } else {
                                println!("[Console] Impossibile recapitare a '{username}' (canale chiuso).");
                            }
                        }
                        None => println!("[Console] Utente '{username}' non connesso."),
                    }
                } else {
                    println!(
                        "[Console] Comando non riconosciuto. Usa: b <msg> | u <user> <msg> | lista"
                    );
                }
            }
        });
    }

    loop {
        let (socket, addr) = listener.accept().await?;
        let users = users.clone();
        let active_users = active_users.clone();
        let user_sessions = user_sessions.clone();
        let tx = tx.clone();

        println!("Nuova connessione: {addr}");

        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, users, active_users, user_sessions, tx).await {
                eprintln!("Errore con client {addr}: {e}");
            }
        });
    }
}

async fn handle_client(
    socket: TcpStream,
    users: UserDb,
    active_users: ActiveUsers,
    user_sessions: UserSessions,
    tx: broadcast::Sender<BroadcastEvent>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = socket.into_split();
    const MAX_LUNGHEZZA_RIGA: usize = 4096;
    let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(MAX_LUNGHEZZA_RIGA));
    let mut rx = tx.subscribe();

    let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<String>(32);

    let mut logged_in_user: Option<String> = None;

    // Ogni riga ricevuta = un messaggio nel formato "COMANDO|campo1|campo2|..."

    loop {
        tokio::select! {
            result = lines.next() => {
                let line = match result {
                    Some(Ok(l)) => l,
                    None => break,
                    Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                        eprintln!("Riga oltre il limite di {MAX_LUNGHEZZA_RIGA} byte ricevuta da un client: chiudo la connessione.");
                        let out = ServerMessage::Notice("Messaggio troppo lungo: connessione chiusa".into()).encode() + "\n";
                        let _ = writer.write_all(out.as_bytes()).await;
                        break;
                    }
                    Some(Err(e)) => {
                        eprintln!("Errore di lettura dal client: {e}");
                        break;
                    }
                };

                let msg = match ClientMessage::decode(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("Messaggio malformato ('{line}'): {e}");
                        let err_response = ServerMessage::Notice(format!("Comando non riconosciuto: {}", e));
                        let out = err_response.encode() + "\n";
                        writer.write_all(out.as_bytes()).await?;
                        continue;
                    }
                };

                let response = match msg {
                    ClientMessage::Register { username, password } => {
                        if !username_valido(&username) {
                            ServerMessage::RegisterErr(
                                "Username non valido o riservato: usa solo lettere, numeri e '_' (1-32 caratteri)".into()
                            )
                        } else if password.is_empty() {
                            ServerMessage::RegisterErr("La password non può essere vuota".into())
                        } else {
                        let esiste_gia = {
                            let db = users.read().await;
                            db.contains_key(&username)
                        };

                        if esiste_gia {
                            ServerMessage::RegisterErr("Utente già esistente".into())
                        } else {
                            let password_clone = password.clone();
                            let hash_result = tokio::task::spawn_blocking(move || {
                                bcrypt::hash(&password_clone, bcrypt::DEFAULT_COST)
                            }).await;

                            match hash_result {
                                Ok(Ok(hash)) => {
                                    let mut db = users.write().await;
                                    if db.contains_key(&username) {
                                        ServerMessage::RegisterErr("Utente già esistente".into())
                                    } else {
                                        db.insert(username.clone(), User { password_hash: hash });
                                        let db_snapshot = db.clone();
                                        drop(db);
                                        let _ = tokio::task::spawn_blocking(move || salva_utenti(&db_snapshot)).await;
                                        println!("Registrato nuovo utente: {username}");
                                        ServerMessage::RegisterOk
                                    }
                                }
                                Ok(Err(e)) => {
                                    eprintln!("Errore durante l'hashing della password: {e}");
                                    ServerMessage::RegisterErr("Errore interno del server".into())
                                }
                                Err(e) => {
                                    eprintln!("Task di hashing fallito: {e}");
                                    ServerMessage::RegisterErr("Errore interno del server".into())
                                }
                            }
                        }
                        }
                    }
                    ClientMessage::Login { username, password } => {
                        let password_hash_opt = {
                            let db = users.read().await;
                            db.get(&username).map(|u| u.password_hash.clone())
                        };

                        let valido = match password_hash_opt {
                            Some(hash) => {
                                tokio::task::spawn_blocking(move || bcrypt::verify(&password, &hash))
                                    .await
                                    .unwrap_or(Ok(false))
                                    .unwrap_or(false)
                            }
                            None => false,
                        };

                        if valido {
                            let gia_connesso = active_users.read().await.contains_key(&username);

                            if gia_connesso {
                                ServerMessage::LoginErr("Utente già connesso da un'altra sessione".into())
                            } else {
                                if let Some(vecchio_user) = logged_in_user.take() {
                                    active_users.write().await.remove(&vecchio_user);
                                }

                                logged_in_user = Some(username.clone());
                                active_users.write().await.insert(username.clone(), mpsc_tx.clone());
                                println!("Login effettuato: {username}");
                                ServerMessage::LoginOk
                            }
                        } else {
                            ServerMessage::LoginErr("Credenziali non valide".into())
                        }
                    }
                    ClientMessage::Text { to, content } => {
                        //controllo login
                        if let Some(user) = &logged_in_user {
                            match to {
                                Some(target_username) if target_username.eq_ignore_ascii_case(SERVER_TARGET) => {
                                    println!("[MSG UTENTE→SERVER] da {}: {}", user, content);
                                    ServerMessage::Notice("Messaggio inviato al server".into())
                                }
                                Some(target_username) => {
                                    let canale = {
                                        let actives = active_users.read().await;
                                        actives.get(&target_username).cloned()
                                    };
                                    if let Some(target_channel) = canale {
                                        let priv_msg = format!("MSG PRIVATO da {}: {}", user, content);
                                        let _ = target_channel.send(priv_msg).await;
                                        ServerMessage::Notice(format!("Messaggio inviato in privato a {}", target_username))
                                    } else {
                                        ServerMessage::Notice(format!("Errore: l'utente {} non è online", target_username))
                                    }
                                }
                                None => {
                                    let event = BroadcastEvent {
                                        mittente: user.clone(),
                                        tipo: TipoEvento::MessaggioTesto { contenuto: content },
                                    };
                                    let _ = tx.send(event);
                                    ServerMessage::Notice("Messaggio inviato a tutti".into())
                                }
                            }
                        } else {
                            ServerMessage::Notice("Devi fare il login per scrivere".into())
                        }
                    }
                    ClientMessage::PositionUpdate { lat, lon, timestamp } => {
                        if let Some(user) = &logged_in_user {
                            if let Err(motivo) = posizione_valida(lat, lon, timestamp) {
                                ServerMessage::Notice(format!("Posizione rifiutata: {motivo}"))
                            } else {
                            // Aggiorna FSM e storico per questo utente
                            let nuovo_stato = {
                                let mut sessions = user_sessions.write().await;
                                let session = sessions.entry(user.clone()).or_insert_with(UserSessionData::new);
                                let pos = Posizione { lat, lon, timestamp };
                                session.aggiorna_posizione(pos)
                            };

                            let stato_str = match nuovo_stato {
                                UserState::Fermo => "FERMO",
                                UserState::InMovimento => "IN MOVIMENTO",
                                UserState::Disconnesso => "DISCONNESSO",
                            };

                            println!(
                                "Posizione di {}: [{:.6}, {:.6}] — stato: {}",
                                user, lat, lon, stato_str
                            );

                            // Le coordinate sono dati privati: solo l'utente che le invia
                            // riceve l'ack. Nessun broadcast agli altri utenti connessi.
                            ServerMessage::Notice(format!(
                                "Posizione [{:.6}, {:.6}] aggiornata — stato: {}", lat, lon, stato_str
                            ))
                            }
                        } else {
                            ServerMessage::Notice("Devi fare il login per inviare la posizione".into())
                        }
                    }
                    ClientMessage::RequestAnalysis { interval } => {
                        if let Some(user) = &logged_in_user {
                            // Legge lo storico reale dall'UserSessionData
                            let storico = {
                                let sessions = user_sessions.read().await;
                                match sessions.get(user) {
                                    Some(session) => session.storico.clone(),
                                    None => Vec::new(),
                                }
                            };

                            if storico.is_empty() {
                                ServerMessage::Notice(format!(
                                    "Nessun dato di posizione disponibile per {}", user
                                ))
                            } else {
                                let report = server::analisi::elabora_statistiche(&storico, &interval);
                                ServerMessage::AnalysisResult {
                                    utente: user.clone(),
                                    intervallo: interval,
                                    report,
                                }
                            }
                        } else {
                            ServerMessage::Notice("Devi fare il login per chiedere l'analisi".into())
                        }
                    }
                };

                let out = response.encode() + "\n";
                writer.write_all(out.as_bytes()).await?;
            }

            //Un utente riceve un altro messaggio da un secondo utente tramite canale broadcast
            result = rx.recv() => {
                let event = match result {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                };

                let Some(user) = &logged_in_user else {
                    continue;
                };

                if event.mittente == *user {
                    continue;
                }

                let out = ServerMessage::Notice(event.formatta_per_client()).encode() + "\n";
                writer.write_all(out.as_bytes()).await?;
            }

            Some(msg) = mpsc_rx.recv() => {
                let out = ServerMessage::Notice(msg).encode() + "\n";
                writer.write_all(out.as_bytes()).await?;
            }
        }
    }

    // Alla disconnessione: stato → Disconnesso
    if let Some(user) = &logged_in_user {
        active_users.write().await.remove(user);

        // Aggiorna FSM → Disconnesso
        {
            let mut sessions = user_sessions.write().await;
            if let Some(session) = sessions.get_mut(user) {
                session.stato = UserState::Disconnesso;
                println!("[FSM] Utente '{}' disconnesso → stato: Disconnesso", user);
            }
        }
    }
    match &logged_in_user {
        Some(user) => println!("Client disconnesso (utente: {user})"),
        None => println!("Client disconnesso (nessun utente autenticato)"),
    }
    Ok(())
}
