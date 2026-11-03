// =============================================================================
// Test di integrazione — server/tests/integration_tests.rs
//
// Ogni test avvia un server Georuggine in-process su una porta casuale (bind
// su :0) e interagisce con esso tramite client TCP in-process.
// Il DB utenti è puramente in-memory: nessun file su disco.
//
// Copertura:
//   Sezione 1 — Autenticazione (1.1–1.5, 1.6, 1.7, 1.9)
//   Sezione 2 — Protocollo (2.1, 2.3, 2.4, 2.5)
//   Sezione 3 — FSM / posizione (3.1, 3.8, 9.2)
//   Sezione 4 — Messaggistica (4.1, 4.2, 4.3, 4.4, 4.5)
//   Sezione 5 — Analisi (5.1, 5.4, 5.8)
// =============================================================================

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::timeout;

use common::{BroadcastEvent, ClientMessage, ServerMessage, TipoEvento, UserState};
use server::{posizione_valida, username_valido, UserSessionData};

// ─── Costanti ────────────────────────────────────────────────────────────────

/// Bcrypt cost ridotto per i test: il default (12) renderebbe ogni test lento
/// di centinaia di ms. Con cost=4 ogni hash richiede ~5ms.
const TEST_BCRYPT_COST: u32 = 4;

/// Timeout per le risposte del server nei test.
const TIMEOUT: Duration = Duration::from_secs(5);

// ─── Struttura User (specchio di server/src/main.rs) ─────────────────────────

#[derive(Clone)]
struct User {
    password_hash: String,
}

type UserDb = Arc<RwLock<HashMap<String, User>>>;
type ActiveUsers = Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>;
type UserSessions = Arc<RwLock<HashMap<String, UserSessionData>>>;

// ─── Server di test ───────────────────────────────────────────────────────────

async fn avvia_server_test() -> (
    std::net::SocketAddr,
    UserDb,
    ActiveUsers,
    UserSessions,
    broadcast::Sender<BroadcastEvent>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let users: UserDb = Arc::new(RwLock::new(HashMap::new()));
    let active_users: ActiveUsers = Arc::new(RwLock::new(HashMap::new()));
    let user_sessions: UserSessions = Arc::new(RwLock::new(HashMap::new()));
    let (tx, _rx) = broadcast::channel::<BroadcastEvent>(100);

    let users_srv = users.clone();
    let active_srv = active_users.clone();
    let sessions_srv = user_sessions.clone();
    let tx_srv = tx.clone();

    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let u = users_srv.clone();
            let a = active_srv.clone();
            let s = sessions_srv.clone();
            let t = tx_srv.clone();
            tokio::spawn(async move {
                let _ = handle_client_test(socket, u, a, s, t).await;
            });
        }
    });

    (addr, users, active_users, user_sessions, tx)
}

/// Versione test di handle_client: usa TEST_BCRYPT_COST invece del default.
async fn handle_client_test(
    socket: TcpStream,
    users: UserDb,
    active_users: ActiveUsers,
    user_sessions: UserSessions,
    tx: broadcast::Sender<BroadcastEvent>,
) -> anyhow::Result<()> {
    use tokio_stream::StreamExt;
    use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

    let (reader, mut writer) = socket.into_split();
    let mut lines = FramedRead::new(reader, LinesCodec::new_with_max_length(4096));
    let mut rx = tx.subscribe();
    let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<String>(32);
    let mut logged_in_user: Option<String> = None;

    loop {
        tokio::select! {
            result = lines.next() => {
                let line = match result {
                    Some(Ok(l)) => l,
                    None => break,
                    Some(Err(LinesCodecError::MaxLineLengthExceeded)) => break,
                    Some(Err(_)) => break,
                };

                let msg = match ClientMessage::decode(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        let out = ServerMessage::Notice(format!("Comando non riconosciuto: {e}")).encode() + "\n";
                        writer.write_all(out.as_bytes()).await?;
                        continue;
                    }
                };

                let response = match msg {
                    ClientMessage::Register { username, password } => {
                        if !username_valido(&username) {
                            ServerMessage::RegisterErr("Username non valido o riservato".into())
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
                                let pw = password.clone();
                                let hash_result = tokio::task::spawn_blocking(move || {
                                    bcrypt::hash(&pw, TEST_BCRYPT_COST)
                                }).await;
                                match hash_result {
                                    Ok(Ok(hash)) => {
                                        let mut db = users.write().await;
                                        if db.contains_key(&username) {
                                            ServerMessage::RegisterErr("Utente già esistente".into())
                                        } else {
                                            db.insert(username.clone(), User { password_hash: hash });
                                            drop(db);
                                            ServerMessage::RegisterOk
                                        }
                                    }
                                    _ => ServerMessage::RegisterErr("Errore interno".into()),
                                }
                            }
                        }
                    }
                    ClientMessage::Login { username, password } => {
                        let hash_opt = {
                            let db = users.read().await;
                            db.get(&username).map(|u| u.password_hash.clone())
                        };
                        let valido = match hash_opt {
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
                                ServerMessage::LoginErr("Utente già connesso".into())
                            } else {
                                if let Some(old) = logged_in_user.take() {
                                    active_users.write().await.remove(&old);
                                }
                                logged_in_user = Some(username.clone());
                                active_users.write().await.insert(username.clone(), mpsc_tx.clone());
                                ServerMessage::LoginOk
                            }
                        } else {
                            ServerMessage::LoginErr("Credenziali non valide".into())
                        }
                    }
                    ClientMessage::Text { to, content } => {
                        if let Some(user) = &logged_in_user {
                            match to {
                                Some(ref target) if target.eq_ignore_ascii_case("SERVER") => {
                                    ServerMessage::Notice("Messaggio inviato al server".into())
                                }
                                Some(target) => {
                                    let canale = {
                                        let a = active_users.read().await;
                                        a.get(&target).cloned()
                                    };
                                    if let Some(ch) = canale {
                                        let _ = ch.send(format!("MSG PRIVATO da {}: {}", user, content)).await;
                                        ServerMessage::Notice(format!("Messaggio inviato in privato a {}", target))
                                    } else {
                                        ServerMessage::Notice(format!("Errore: l'utente {} non è online", target))
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
                                let nuovo_stato = {
                                    let mut sessions = user_sessions.write().await;
                                    let session = sessions
                                        .entry(user.clone())
                                        .or_insert_with(UserSessionData::new);
                                    let p = common::Posizione { lat, lon, timestamp };
                                    session.aggiorna_posizione(p)
                                };
                                let stato_str = match nuovo_stato {
                                    UserState::Fermo => "FERMO",
                                    UserState::InMovimento => "IN MOVIMENTO",
                                    UserState::Disconnesso => "DISCONNESSO",
                                };
                                ServerMessage::Notice(format!(
                                    "Posizione [{:.6}, {:.6}] aggiornata — stato: {}",
                                    lat, lon, stato_str
                                ))
                            }
                        } else {
                            ServerMessage::Notice("Devi fare il login per inviare la posizione".into())
                        }
                    }
                    ClientMessage::RequestAnalysis { interval } => {
                        if let Some(user) = &logged_in_user {
                            let storico = {
                                let sessions = user_sessions.read().await;
                                sessions.get(user).map(|s| s.storico.clone()).unwrap_or_default()
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

            result = rx.recv() => {
                let event = match result {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                };
                let Some(user) = &logged_in_user else { continue };
                if event.mittente == *user { continue }
                let out = ServerMessage::Notice(event.formatta_per_client()).encode() + "\n";
                writer.write_all(out.as_bytes()).await?;
            }

            Some(msg) = mpsc_rx.recv() => {
                let out = ServerMessage::Notice(msg).encode() + "\n";
                writer.write_all(out.as_bytes()).await?;
            }
        }
    }

    if let Some(user) = &logged_in_user {
        active_users.write().await.remove(user);
        let mut sessions = user_sessions.write().await;
        if let Some(s) = sessions.get_mut(user) {
            s.stato = UserState::Disconnesso;
        }
    }
    Ok(())
}

// ─── Helpers client ───────────────────────────────────────────────────────────

async fn invia_e_ricevi(addr: std::net::SocketAddr, messaggio: &str) -> ServerMessage {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    writer
        .write_all(format!("{messaggio}\n").as_bytes())
        .await
        .unwrap();
    let line = timeout(TIMEOUT, lines.next_line())
        .await
        .expect("Timeout sulla risposta del server")
        .unwrap()
        .expect("Connessione chiusa senza risposta");
    ServerMessage::decode(&line).unwrap()
}

async fn sessione(addr: std::net::SocketAddr, messaggi: Vec<&str>) -> Vec<ServerMessage> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut risposte = Vec::new();
    for msg in messaggi {
        writer
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .unwrap();
        let line = timeout(TIMEOUT, lines.next_line())
            .await
            .expect("Timeout sulla risposta del server")
            .unwrap()
            .expect("Connessione chiusa senza risposta");
        risposte.push(ServerMessage::decode(&line).unwrap());
    }
    risposte
}

// ─── Test: Sezione 1 — Autenticazione ────────────────────────────────────────

/// 1.1 — Registrazione valida → REGISTER_OK
#[tokio::test]
async fn test_registrazione_valida() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REGISTER|anto|1234").await;
    assert!(
        matches!(risposta, ServerMessage::RegisterOk),
        "{risposta:?}"
    );
}

/// 1.2 — Registrazione utente duplicato → REGISTER_ERR
#[tokio::test]
async fn test_registrazione_duplicata() {
    let (addr, ..) = avvia_server_test().await;
    let r1 = invia_e_ricevi(addr, "REGISTER|anto|1234").await;
    assert!(matches!(r1, ServerMessage::RegisterOk), "Prima reg: {r1:?}");
    let r2 = invia_e_ricevi(addr, "REGISTER|anto|altrapw").await;
    match r2 {
        ServerMessage::RegisterErr(msg) => assert!(
            msg.contains("già") || msg.to_lowercase().contains("exist"),
            "{msg}"
        ),
        other => panic!("Atteso RegisterErr, ottenuto {other:?}"),
    }
}

/// 1.3 — Login corretto → LOGIN_OK
#[tokio::test]
async fn test_login_corretto() {
    let (addr, ..) = avvia_server_test().await;
    invia_e_ricevi(addr, "REGISTER|anto|1234").await;
    let risposta = invia_e_ricevi(addr, "LOGIN|anto|1234").await;
    assert!(matches!(risposta, ServerMessage::LoginOk), "{risposta:?}");
}

/// 1.4 — Login con password sbagliata → LOGIN_ERR
#[tokio::test]
async fn test_login_password_sbagliata() {
    let (addr, ..) = avvia_server_test().await;
    invia_e_ricevi(addr, "REGISTER|anto|1234").await;
    let risposta = invia_e_ricevi(addr, "LOGIN|anto|sbagliata").await;
    match risposta {
        ServerMessage::LoginErr(msg) => assert!(
            msg.contains("Credenziali") || msg.to_lowercase().contains("invalid"),
            "{msg}"
        ),
        other => panic!("Atteso LoginErr, ottenuto {other:?}"),
    }
}

/// 1.5 — Login utente inesistente → LOGIN_ERR
#[tokio::test]
async fn test_login_utente_inesistente() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "LOGIN|fantasma|1234").await;
    assert!(
        matches!(risposta, ServerMessage::LoginErr(_)),
        "{risposta:?}"
    );
}

/// 1.6 — Registrazione con password vuota → REGISTER_ERR
#[tokio::test]
async fn test_registrazione_password_vuota() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REGISTER|anto2|").await;
    assert!(
        matches!(risposta, ServerMessage::RegisterErr(_)),
        "Password vuota deve essere rifiutata: {risposta:?}"
    );
}

/// 1.7 — Registrazione con username vuoto → REGISTER_ERR
#[tokio::test]
async fn test_registrazione_username_vuoto() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REGISTER||1234").await;
    assert!(
        matches!(risposta, ServerMessage::RegisterErr(_)),
        "Username vuoto deve essere rifiutato: {risposta:?}"
    );
}

/// 1.7 variante — Username riservato "SERVER" → REGISTER_ERR
#[tokio::test]
async fn test_registrazione_username_riservato_server() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REGISTER|SERVER|pass").await;
    assert!(
        matches!(risposta, ServerMessage::RegisterErr(_)),
        "SERVER è riservato e deve essere rifiutato: {risposta:?}"
    );
}

/// 1.9 — Race condition: due client registrano stesso username → solo uno Ok
#[tokio::test]
async fn test_registrazione_simultanea_race_condition() {
    let (addr, ..) = avvia_server_test().await;
    let (r1, r2) = tokio::join!(
        invia_e_ricevi(addr, "REGISTER|stesso_nome|pw1"),
        invia_e_ricevi(addr, "REGISTER|stesso_nome|pw2"),
    );
    let ok = [&r1, &r2]
        .iter()
        .filter(|r| matches!(r, ServerMessage::RegisterOk))
        .count();
    let err = [&r1, &r2]
        .iter()
        .filter(|r| matches!(r, ServerMessage::RegisterErr(_)))
        .count();
    assert_eq!(ok, 1, "Esattamente uno RegisterOk: {r1:?} {r2:?}");
    assert_eq!(err, 1, "Esattamente uno RegisterErr: {r1:?} {r2:?}");
}

// ─── Test: Sezione 2 — Protocollo ────────────────────────────────────────────

/// 2.1 — Comando sconosciuto → NOTICE di errore
#[tokio::test]
async fn test_comando_sconosciuto() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "PIPPO|test").await;
    match risposta {
        ServerMessage::Notice(msg) => assert!(
            msg.to_lowercase().contains("sconosciuto")
                || msg.to_lowercase().contains("riconosciuto"),
            "{msg}"
        ),
        other => panic!("Atteso Notice, ottenuto {other:?}"),
    }
}

/// 2.3 — Riga vuota → il server non crasha
#[tokio::test]
async fn test_riga_vuota_non_crasha() {
    let (addr, ..) = avvia_server_test().await;
    let stream = TcpStream::connect(addr).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    writer.write_all(b"\n").await.unwrap();
    // Risposta o EOF sono entrambi accettabili; basta non fare timeout/panic
    let result = timeout(TIMEOUT, lines.next_line()).await;
    assert!(result.is_ok(), "Il server non deve bloccarsi su riga vuota");
}

/// 2.4 — REGISTER con campi mancanti → errore (RegisterErr o Notice)
#[tokio::test]
async fn test_register_campi_mancanti() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REGISTER|soloUsername").await;
    let e_errore = matches!(&risposta, ServerMessage::RegisterErr(_))
        || matches!(&risposta, ServerMessage::Notice(_));
    assert!(
        e_errore,
        "REGISTER senza password deve produrre errore: {risposta:?}"
    );
}

/// 2.5 — POS con lat/lon non numerica → NOTICE di errore (dopo login)
#[tokio::test]
async fn test_pos_valori_non_numerici() {
    let (addr, ..) = avvia_server_test().await;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|testuser|pass",
            "LOGIN|testuser|pass",
            "POS|abc|xyz|123",
        ],
    )
    .await;
    let ultima = risposte.last().unwrap();
    assert!(
        matches!(ultima, ServerMessage::Notice(_)),
        "POS non numerica deve produrre Notice: {ultima:?}"
    );
}

// ─── Test: Sezione 3 — FSM / posizione ───────────────────────────────────────

/// 3.8 — POS senza login → NOTICE con "login"
#[tokio::test]
async fn test_pos_senza_login() {
    let (addr, ..) = avvia_server_test().await;
    let ts = chrono::Utc::now().timestamp() as u64;
    let risposta = invia_e_ricevi(addr, &format!("POS|45.0|7.0|{ts}")).await;
    match risposta {
        ServerMessage::Notice(msg) => assert!(msg.contains("login"), "{msg}"),
        other => panic!("Atteso Notice, ottenuto {other:?}"),
    }
}

/// 3.1 — Prima POS → risposta con "IN MOVIMENTO"
#[tokio::test]
async fn test_prima_posizione_in_movimento() {
    let (addr, ..) = avvia_server_test().await;
    let ts = chrono::Utc::now().timestamp() as u64;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|mover|pass",
            "LOGIN|mover|pass",
            &format!("POS|45.07|7.68|{ts}"),
        ],
    )
    .await;
    match risposte.last().unwrap() {
        ServerMessage::Notice(msg) => assert!(
            msg.contains("IN MOVIMENTO"),
            "Prima POS deve produrre IN MOVIMENTO: {msg}"
        ),
        other => panic!("{other:?}"),
    }
}

/// 9.2 — POS con timestamp nel futuro → posizione rifiutata
#[tokio::test]
async fn test_pos_timestamp_futuro_rifiutato() {
    let (addr, ..) = avvia_server_test().await;
    let ts_futuro = chrono::Utc::now().timestamp() as u64 + 3600;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|futureuser|pass",
            "LOGIN|futureuser|pass",
            &format!("POS|45.0|7.0|{ts_futuro}"),
        ],
    )
    .await;
    match risposte.last().unwrap() {
        ServerMessage::Notice(msg) => assert!(
            msg.to_lowercase().contains("rifiutata") || msg.to_lowercase().contains("futuro"),
            "POS futura deve essere rifiutata: {msg}"
        ),
        other => panic!("{other:?}"),
    }
}

// ─── Test: Sezione 4 — Messaggistica ─────────────────────────────────────────

/// 4.5 — TEXT senza login → NOTICE con "login"
#[tokio::test]
async fn test_text_senza_login() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "TEXT|ALL|ciao").await;
    match risposta {
        ServerMessage::Notice(msg) => assert!(msg.contains("login"), "{msg}"),
        other => panic!("Atteso Notice, ottenuto {other:?}"),
    }
}

/// 4.2 — Il mittente del broadcast riceve conferma, non l'eco del proprio testo
#[tokio::test]
async fn test_broadcast_mittente_non_riceve_eco() {
    let (addr, ..) = avvia_server_test().await;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|broadcaster|pass",
            "LOGIN|broadcaster|pass",
            "TEXT|ALL|ciao a tutti",
        ],
    )
    .await;
    match risposte.last().unwrap() {
        ServerMessage::Notice(msg) => assert!(
            !msg.contains("ciao a tutti"),
            "Mittente non deve ricevere eco: {msg}"
        ),
        other => panic!("{other:?}"),
    }
}

/// 4.3 — Messaggio privato: il destinatario online lo riceve
#[tokio::test]
async fn test_messaggio_privato_utente_online() {
    let (addr, ..) = avvia_server_test().await;

    // Connetti e autentica il destinatario
    let stream_dest = TcpStream::connect(addr).await.unwrap();
    let (reader_dest, mut writer_dest) = stream_dest.into_split();
    let mut lines_dest = BufReader::new(reader_dest).lines();

    writer_dest
        .write_all(b"REGISTER|destinatario|pass\n")
        .await
        .unwrap();
    let _ = timeout(TIMEOUT, lines_dest.next_line())
        .await
        .unwrap()
        .unwrap();
    writer_dest
        .write_all(b"LOGIN|destinatario|pass\n")
        .await
        .unwrap();
    let _ = timeout(TIMEOUT, lines_dest.next_line())
        .await
        .unwrap()
        .unwrap();

    // Il mittente si connette e invia il messaggio privato
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|mittente|pass",
            "LOGIN|mittente|pass",
            "TEXT|destinatario|messaggio privato",
        ],
    )
    .await;
    assert!(
        matches!(risposte.last().unwrap(), ServerMessage::Notice(_)),
        "Mittente deve ricevere Notice: {:?}",
        risposte.last()
    );

    // Il destinatario deve ricevere il messaggio
    let riga = timeout(TIMEOUT, lines_dest.next_line())
        .await
        .expect("Timeout: destinatario non ha ricevuto il messaggio")
        .unwrap()
        .unwrap();
    let msg = ServerMessage::decode(&riga).unwrap();
    match msg {
        ServerMessage::Notice(testo) => assert!(
            testo.contains("mittente") && testo.contains("messaggio privato"),
            "Contenuto atteso non trovato: {testo}"
        ),
        other => panic!("{other:?}"),
    }
}

/// 4.4 — Messaggio privato a utente offline → NOTICE di errore
#[tokio::test]
async fn test_messaggio_privato_utente_offline() {
    let (addr, ..) = avvia_server_test().await;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|mittente2|pass",
            "LOGIN|mittente2|pass",
            "TEXT|utente_non_connesso|ciao",
        ],
    )
    .await;
    match risposte.last().unwrap() {
        ServerMessage::Notice(msg) => assert!(
            msg.contains("non è online") || msg.contains("non connesso"),
            "{msg}"
        ),
        other => panic!("{other:?}"),
    }
}

/// 4.1 — Broadcast: un secondo client connesso lo riceve
#[tokio::test]
async fn test_broadcast_ricevuto_da_altri() {
    let (addr, ..) = avvia_server_test().await;

    // Connetti e autentica il ricevitore
    let stream_recv = TcpStream::connect(addr).await.unwrap();
    let (reader_recv, mut writer_recv) = stream_recv.into_split();
    let mut lines_recv = BufReader::new(reader_recv).lines();

    writer_recv
        .write_all(b"REGISTER|ricevitore|pass\n")
        .await
        .unwrap();
    let _ = timeout(TIMEOUT, lines_recv.next_line())
        .await
        .unwrap()
        .unwrap();
    writer_recv
        .write_all(b"LOGIN|ricevitore|pass\n")
        .await
        .unwrap();
    let _ = timeout(TIMEOUT, lines_recv.next_line())
        .await
        .unwrap()
        .unwrap();

    // Il mittente invia il broadcast
    sessione(
        addr,
        vec![
            "REGISTER|sender|pass",
            "LOGIN|sender|pass",
            "TEXT|ALL|messaggio broadcast",
        ],
    )
    .await;

    // Il ricevitore deve riceverlo
    let riga = timeout(TIMEOUT, lines_recv.next_line())
        .await
        .expect("Timeout: ricevitore non ha ricevuto il broadcast")
        .unwrap()
        .unwrap();
    match ServerMessage::decode(&riga).unwrap() {
        ServerMessage::Notice(testo) => assert!(
            testo.contains("messaggio broadcast"),
            "Broadcast non contiene il testo: {testo}"
        ),
        other => panic!("{other:?}"),
    }
}

// ─── Test: Sezione 5 — Analisi ────────────────────────────────────────────────

/// 5.8 — REQANALYSIS senza login → NOTICE con "login"
#[tokio::test]
async fn test_reqanalysis_senza_login() {
    let (addr, ..) = avvia_server_test().await;
    let risposta = invia_e_ricevi(addr, "REQANALYSIS|DAY").await;
    match risposta {
        ServerMessage::Notice(msg) => assert!(msg.contains("login"), "{msg}"),
        other => panic!("{other:?}"),
    }
}

/// 5.1 — REQANALYSIS senza dati → NOTICE "nessun dato"
#[tokio::test]
async fn test_reqanalysis_senza_dati() {
    let (addr, ..) = avvia_server_test().await;
    let risposte = sessione(
        addr,
        vec![
            "REGISTER|analista|pass",
            "LOGIN|analista|pass",
            "REQANALYSIS|DAY",
        ],
    )
    .await;
    match risposte.last().unwrap() {
        ServerMessage::Notice(msg) => assert!(
            msg.to_lowercase().contains("nessun"),
            "Deve indicare assenza di dati: {msg}"
        ),
        other => panic!("{other:?}"),
    }
}

/// 5.4 — REQANALYSIS con dati → AnalysisResult con distanza > 0
#[tokio::test]
async fn test_reqanalysis_con_dati() {
    let (addr, ..) = avvia_server_test().await;
    let ora = chrono::Utc::now().timestamp() as u64;
    let pos1 = format!("POS|45.0|7.0|{}", ora - 1000);
    let pos2 = format!("POS|45.001|7.0|{}", ora - 400);

    let risposte = sessione(
        addr,
        vec![
            "REGISTER|tracciato|pass",
            "LOGIN|tracciato|pass",
            &pos1,
            &pos2,
            "REQANALYSIS|DAY",
        ],
    )
    .await;

    match risposte.last().unwrap() {
        ServerMessage::AnalysisResult { report, .. } => assert!(
            report.distanza_totale_km > 0.0,
            "Distanza attesa > 0: {}",
            report.distanza_totale_km
        ),
        other => panic!("Atteso AnalysisResult, ottenuto {other:?}"),
    }
}
