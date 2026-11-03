mod simulator;

use common::ClientMessage;
use simulator::{FileSource, PositionSource, RandomWalkSource, SimulationStrategy};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

fn stampa_banner_sessione(con_simulatore: bool) {
    println!("\n--- Sessione attiva ---");
    println!("Comandi disponibili:");
    println!("  TEXT|ALL|messaggio          → broadcast a tutti");
    println!("  TEXT|username|messaggio     → messaggio privato");
    println!("  TEXT|SERVER|messaggio      → messaggio all'operatore del server");
    println!("  REQANALYSIS|DAY/WEEK/MONTH → richiedi analisi movimento");
    println!("  POS|lat|lon|timestamp      → invio posizione manuale");
    if con_simulatore {
        println!("(Il simulatore invia posizioni automaticamente)\n");
    } else {
        println!();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let stream = TcpStream::connect("127.0.0.1:7878").await?;
    println!("Connesso al server Georuggine!");

    let (reader, mut writer) = stream.into_split();
    let mut server_lines = BufReader::new(reader).lines();
    let mut stdin_lines = BufReader::new(tokio::io::stdin()).lines();

    // ------------------------------------------------------------------
    // Registrazione / Login
    // ------------------------------------------------------------------
    println!("\n--- Autenticazione ---");
    println!("Comandi disponibili:");
    println!("  REGISTER|username|password");
    println!("  LOGIN|username|password");
    println!("Inserisci un comando per procedere:");

    let mut autenticato = false;

    while !autenticato {
        tokio::select! {
            result = stdin_lines.next_line() => {
                if let Ok(Some(line)) = result {
                    let out = format!("{}\n", line);
                    writer.write_all(out.as_bytes()).await?;
                }
            }
            result = server_lines.next_line() => {
                match result {
                    Ok(Some(line)) => {
                        println!("<< Server: {}", line);
                        if let Ok(common::ServerMessage::LoginOk) = common::ServerMessage::decode(&line) {
                            autenticato = true;
                        }
                    }
                    Ok(None) => {
                        println!("Il server ha chiuso la connessione.");
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Errore di lettura dal server: {}", e);
                        return Ok(());
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Scelta della simulazione
    // ------------------------------------------------------------------
    println!("\n--- Simulatore di movimento ---");
    println!("Scegli una strategia:");
    println!("  1) Carica posizioni da file CSV");
    println!("  2) Random walk (movimento pseudo-casuale)");
    println!("  3) Nessuna simulazione (solo comandi manuali)");
    print!("Scelta [1/2/3]: ");

    let strategia_opt: Option<SimulationStrategy>;

    loop {
        if let Ok(Some(scelta)) = stdin_lines.next_line().await {
            let scelta = scelta.trim().to_string();
            match scelta.as_str() {
                "1" => {
                    println!("Inserisci il percorso del file CSV (es. sample_route.csv):");
                    if let Ok(Some(path)) = stdin_lines.next_line().await {
                        let path = path.trim().to_string();
                        match FileSource::new(&path) {
                            Ok(source) => {
                                strategia_opt = Some(SimulationStrategy::File(source));
                                break;
                            }
                            Err(e) => {
                                eprintln!("Errore nel caricare il file: {}. Riprova.", e);
                                println!("Scelta [1/2/3]: ");
                                continue;
                            }
                        }
                    }
                }
                "2" => {
                    // Valori di default: centro Torino, passo 0.1 km
                    let center_lat = 45.0628;
                    let center_lon = 7.6623;
                    let passo_km = 0.1;
                    println!(
                        "Random walk da ({}, {}), passo {:.2} km",
                        center_lat, center_lon, passo_km
                    );
                    strategia_opt = Some(SimulationStrategy::RandomWalk(RandomWalkSource::new(
                        center_lat, center_lon, passo_km,
                    )));
                    break;
                }
                "3" => {
                    println!("Nessuna simulazione selezionata.");
                    strategia_opt = None;
                    break;
                }
                _ => {
                    println!("Scelta non valida. Inserisci 1, 2 o 3:");
                    continue;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Avvio del simulatore in background
    // ------------------------------------------------------------------

    // Canale per inviare le righe generate dal simulatore al loop di scrittura
    let (pos_tx, mut pos_rx) = mpsc::channel::<String>(16);

    if let Some(strategia) = strategia_opt {
        let is_random_walk = matches!(strategia, SimulationStrategy::RandomWalk(_));
        if is_random_walk {
            stampa_banner_sessione(true);
        }

        // Task che genera posizioni ogni x secondi
        tokio::spawn(async move {
            let mut sim = strategia;
            let mut was_file = false;
            if matches!(sim, SimulationStrategy::File(_)) {
                was_file = true;
            }

            loop {
                match sim.next_position() {
                    Some((lat, lon)) => {
                        let ts = chrono::Utc::now().timestamp() as u64;
                        let msg = ClientMessage::PositionUpdate {
                            lat,
                            lon,
                            timestamp: ts,
                        };
                        let encoded = msg.encode();
                        println!(
                            "[Simulatore] Invio posizione: lat={:.6}, lon={:.6}",
                            lat, lon
                        );

                        if pos_tx.send(encoded).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        println!("[Simulatore] Sequenza di posizioni terminata.");
                        if was_file {
                            let _ = pos_tx.send("PRINT_BANNER".to_string()).await;
                        }
                        break;
                    }
                }
                // Attesa tra un invio e l'altro di 30s
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
    } else {
        stampa_banner_sessione(false);
    }

    // Loop principale: gestisce stdin, risposte server, e posizioni dal simulatore
    loop {
        tokio::select! {
            // Messaggi dal server
            result = server_lines.next_line() => {
                match result {
                    Ok(Some(line)) => println!("<< Server: {}", line),
                    Ok(None) => {
                        println!("Il server ha chiuso la connessione.");
                        break;
                    }
                    Err(e) => {
                        eprintln!("Errore di lettura dal server: {}", e);
                        break;
                    }
                }
            }
            // Comandi manuali da tastiera
            result = stdin_lines.next_line() => {
                if let Ok(Some(line)) = result {
                    let out = format!("{}\n", line);
                    writer.write_all(out.as_bytes()).await?;
                }
            }
            // Posizioni generate dal simulatore
            Some(encoded_msg) = pos_rx.recv() => {
                if encoded_msg == "PRINT_BANNER" {
                    stampa_banner_sessione(false);
                } else {
                    let out = format!("{}\n", encoded_msg);
                    writer.write_all(out.as_bytes()).await?;
                }
            }
        }
    }

    Ok(())
}
