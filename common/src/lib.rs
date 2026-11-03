// Ogni messaggio è una riga di testo con campi separati da '|'. Encoding e decoding fatti a mano.

// Il protocollo è line-based (un messaggio = una riga terminata da '\n'). Un campo
// testuale libero (username, password, contenuto di un messaggio...) non deve MAI
// contenere '\n' o '\r', altrimenti spezzerebbe il framing lato ricevente. Qui
// sostituiamo questi caratteri con uno spazio prima di incapsulare il campo nella riga.
fn sanitizza_campo(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

// Enum che incapsula messaggi inviati dal CLIENT al SERVER
#[derive(Debug, Clone)]
pub enum ClientMessage {
    Register { username: String, password: String },
    Login { username: String, password: String },
    PositionUpdate { lat: f64, lon: f64, timestamp: u64 },
    Text { to: Option<String>, content: String }, // to = None -> messaggio broadcast
    RequestAnalysis { interval: Interval },       // Day | Week | Month
}

// Enum per selezionare l'intervallo temporale sul quale calcolare le metriche
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Interval {
    Day,
    Week,
    Month,
}

impl ClientMessage {
    // Trasforma il messaggio in una riga di testo da spedire sul socket.
    pub fn encode(&self) -> String {
        match self {
            ClientMessage::Register { username, password } => {
                format!(
                    "REGISTER|{}|{}",
                    sanitizza_campo(username),
                    sanitizza_campo(password)
                )
            }
            ClientMessage::Login { username, password } => {
                format!(
                    "LOGIN|{}|{}",
                    sanitizza_campo(username),
                    sanitizza_campo(password)
                )
            }
            ClientMessage::PositionUpdate {
                lat,
                lon,
                timestamp,
            } => {
                format!("POS|{lat}|{lon}|{timestamp}")
            }
            ClientMessage::Text { to, content } => {
                let dest = to
                    .clone()
                    .map(|d| sanitizza_campo(&d))
                    .unwrap_or_else(|| "ALL".to_string());
                format!("TEXT|{}|{}", dest, sanitizza_campo(content))
            }
            ClientMessage::RequestAnalysis { interval } => {
                let int_str = match interval {
                    Interval::Day => "DAY",
                    Interval::Week => "WEEK",
                    Interval::Month => "MONTH",
                };
                format!("REQANALYSIS|{int_str}")
            }
        }
    }

    // partendo da una riga di testo ricevuta dal socket viene trasformata in ClientMessage.
    pub fn decode(line: &str) -> Result<Self, String> {
        let mut parts = line.trim().splitn(2, '|');
        let cmd = parts.next().ok_or("messaggio vuoto")?;
        let rest = parts.next().unwrap_or("");

        match cmd {
            "REGISTER" => {
                let mut p = rest.splitn(2, '|');
                let username = p.next().ok_or("manca username")?.to_string();
                let password = p.next().ok_or("manca password")?.to_string();
                Ok(ClientMessage::Register { username, password })
            }
            "LOGIN" => {
                let mut p = rest.splitn(2, '|');
                let username = p.next().ok_or("manca username")?.to_string();
                let password = p.next().ok_or("manca password")?.to_string();
                Ok(ClientMessage::Login { username, password })
            }
            "POS" => {
                let mut p = rest.splitn(3, '|');
                let lat: f64 = p
                    .next()
                    .ok_or("manca lat")?
                    .parse()
                    .map_err(|_| "lat non valida")?;
                let lon: f64 = p
                    .next()
                    .ok_or("manca lon")?
                    .parse()
                    .map_err(|_| "lon non valida")?;
                let timestamp: u64 = p
                    .next()
                    .ok_or("manca timestamp")?
                    .parse()
                    .map_err(|_| "timestamp non valido")?;
                Ok(ClientMessage::PositionUpdate {
                    lat,
                    lon,
                    timestamp,
                })
            }
            "TEXT" => {
                let mut p = rest.splitn(2, '|');
                let dest = p.next().ok_or("manca destinatario")?.to_string();
                let content = p.next().unwrap_or("").to_string();
                let to = if dest == "ALL" { None } else { Some(dest) };
                Ok(ClientMessage::Text { to, content })
            }
            "REQANALYSIS" => {
                let interval_enum = match rest {
                    "DAY" => Interval::Day,
                    "WEEK" => Interval::Week,
                    "MONTH" => Interval::Month,
                    _ => return Err(format!("Intervallo non valido: {}", rest)),
                };
                Ok(ClientMessage::RequestAnalysis {
                    interval: interval_enum,
                })
            }
            other => Err(format!("comando sconosciuto: {other}")),
        }
    }
}

// Messaggi che il SERVER invia al CLIENT
#[derive(Debug, Clone)]
pub enum ServerMessage {
    RegisterOk,
    RegisterErr(String),
    LoginOk,
    LoginErr(String),
    Notice(String),
    Text {
        from: String,
        content: String,
    },
    /// Risultato di una richiesta di analisi.
    AnalysisResult {
        utente: String,
        intervallo: Interval,
        report: ReportAnalisi,
    },
}

impl ServerMessage {
    pub fn encode(&self) -> String {
        match self {
            ServerMessage::RegisterOk => "REGISTER_OK".to_string(),
            ServerMessage::RegisterErr(reason) => {
                format!("REGISTER_ERR|{}", sanitizza_campo(reason))
            }
            ServerMessage::LoginOk => "LOGIN_OK".to_string(),
            ServerMessage::LoginErr(reason) => format!("LOGIN_ERR|{}", sanitizza_campo(reason)),
            ServerMessage::Notice(text) => format!("NOTICE|{}", sanitizza_campo(text)),
            ServerMessage::Text { from, content } => {
                format!(
                    "TEXT|{}|{}",
                    sanitizza_campo(from),
                    sanitizza_campo(content)
                )
            }
            // Formato wire: ANALYSIS|utente|intervallo|dist_km|vel_kmh|mov_sec|pausa_sec
            ServerMessage::AnalysisResult {
                utente,
                intervallo,
                report,
            } => {
                let int_str = match intervallo {
                    Interval::Day => "DAY",
                    Interval::Week => "WEEK",
                    Interval::Month => "MONTH",
                };
                format!(
                    "ANALYSIS|{utente}|{int_str}|{:.6}|{:.6}|{}|{}",
                    report.distanza_totale_km,
                    report.velocita_media_kmh,
                    report.tempo_in_movimento_sec,
                    report.tempo_in_pausa_sec
                )
            }
        }
    }

    pub fn decode(line: &str) -> Result<Self, String> {
        let mut parts = line.trim().splitn(2, '|');
        let cmd = parts.next().ok_or("messaggio vuoto")?;
        let rest = parts.next().unwrap_or("");

        match cmd {
            "REGISTER_OK" => Ok(ServerMessage::RegisterOk),
            "REGISTER_ERR" => Ok(ServerMessage::RegisterErr(rest.to_string())),
            "LOGIN_OK" => Ok(ServerMessage::LoginOk),
            "LOGIN_ERR" => Ok(ServerMessage::LoginErr(rest.to_string())),
            "NOTICE" => Ok(ServerMessage::Notice(rest.to_string())),
            "TEXT" => {
                let mut p = rest.splitn(2, '|');
                let from = p.next().ok_or("manca mittente")?.to_string();
                let content = p.next().unwrap_or("").to_string();
                Ok(ServerMessage::Text { from, content })
            }
            // Formato wire: ANALYSIS|utente|intervallo|dist_km|vel_kmh|mov_sec|pausa_sec
            "ANALYSIS" => {
                let mut p = rest.splitn(6, '|');
                let utente = p.next().ok_or("manca utente")?.to_string();
                let int_str = p.next().ok_or("manca intervallo")?;
                let intervallo = match int_str {
                    "DAY" => Interval::Day,
                    "WEEK" => Interval::Week,
                    "MONTH" => Interval::Month,
                    other => return Err(format!("intervallo sconosciuto: {other}")),
                };
                let distanza_totale_km: f64 = p
                    .next()
                    .ok_or("manca distanza")?
                    .parse()
                    .map_err(|_| "distanza non valida")?;
                let velocita_media_kmh: f64 = p
                    .next()
                    .ok_or("manca velocità")?
                    .parse()
                    .map_err(|_| "velocità non valida")?;
                let tempo_in_movimento_sec: u64 = p
                    .next()
                    .ok_or("manca tempo movimento")?
                    .parse()
                    .map_err(|_| "tempo movimento non valido")?;
                let tempo_in_pausa_sec: u64 = p
                    .next()
                    .ok_or("manca tempo pausa")?
                    .parse()
                    .map_err(|_| "tempo pausa non valido")?;
                Ok(ServerMessage::AnalysisResult {
                    utente,
                    intervallo,
                    report: ReportAnalisi {
                        distanza_totale_km,
                        velocita_media_kmh,
                        tempo_in_movimento_sec,
                        tempo_in_pausa_sec,
                    },
                })
            }
            other => Err(format!("comando sconosciuto: {other}")),
        }
    }
}

// Stato di un utente
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserState {
    Disconnesso,
    Fermo,
    InMovimento,
}

// Struttura per incapsulare la posizione
#[derive(Debug, Clone, Copy)]
pub struct Posizione {
    pub lat: f64,
    pub lon: f64,
    pub timestamp: u64,
}

// Definizione condivisa di "movimento" vs "fermo".
// Sia la FSM in tempo reale (server/src/main.rs) sia l'analisi statistica
// (server/src/analisi.rs) devono usare la STESSA soglia di distanza e la
// STESSA soglia temporale di pausa

/// Raggio medio della Terra in km
pub const RAGGIO_TERRA_KM: f64 = 6371.0;

/// Distanza tra due posizioni consecutive sotto la quale l'utente è considerato fermo.
pub const SOGLIA_MOVIMENTO_KM: f64 = 0.005; // 5 metri

/// Tempo minimo, a coordinate invariate (sotto SOGLIA_MOVIMENTO_KM), perché lo
/// stato passi da "in movimento" a "fermo".
pub const SOGLIA_PAUSA_SEC: u64 = 180; // 3 minuti

/// Vero se lat/lon sono numeri finiti e dentro i range geografici validi
/// ([-90, 90] per la latitudine, [-180, 180] per la longitudine). Condivisa tra
/// server (validazione dei POS in ingresso) e client (validazione del CSV del
/// simulatore), così che server e client concordano su cosa sia una posizione valida.
pub fn coordinate_valide(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && lon.is_finite()
        && (-180.0..=180.0).contains(&lon)
}

/// Distanza in km tra due posizioni secondo (formula di Haversine).
pub fn distanza_haversine(p1: &Posizione, p2: &Posizione) -> f64 {
    let d_lat = (p2.lat - p1.lat).to_radians();
    let d_lon = (p2.lon - p1.lon).to_radians();

    let lat1 = p1.lat.to_radians();
    let lat2 = p2.lat.to_radians();

    let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);

    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    RAGGIO_TERRA_KM * c
}

// Struttura ausiliaria per il calcolo delle metriche
#[derive(Debug, Clone)]
pub struct ReportAnalisi {
    pub distanza_totale_km: f64,
    pub velocita_media_kmh: f64,
    pub tempo_in_movimento_sec: u64,
    pub tempo_in_pausa_sec: u64,
}

// Strutture usate solo per il canale broadcast interno.
// Rispetto a passarsi delle stringhe grezze, qui abbiamo il campo `mittente` separato:
// in questo modo l'anti-eco si fa con un confronto pulito (==) anziché un fragile starts_with().

/// Tipo di evento che può transitare nel canale broadcast interno del server.
#[derive(Debug, Clone)]
pub enum TipoEvento {
    /// Un utente ha inviato un messaggio a tutti.
    MessaggioTesto { contenuto: String },
    /// Un utente ha aggiornato la propria posizione.
    AggiornamentoPosizione {
        lat: f64,
        lon: f64,
        stato: UserState,
    },
}

/// Evento che transita nel canale `broadcast::channel<BroadcastEvent>` del server.
/// Il campo `mittente` è usato per il filtro anti-eco: l'utente che ha generato
/// l'evento non lo riceve di ritorno.
#[derive(Debug, Clone)]
pub struct BroadcastEvent {
    pub mittente: String,
    pub tipo: TipoEvento,
}

impl BroadcastEvent {
    /// Serializza l'evento nel testo che verrà inviato ai client remoti.
    pub fn formatta_per_client(&self) -> String {
        match &self.tipo {
            TipoEvento::MessaggioTesto { contenuto } => {
                format!("MSG BROADCAST da {}: {}", self.mittente, contenuto)
            }
            TipoEvento::AggiornamentoPosizione { lat, lon, stato } => {
                let stato_str = match stato {
                    UserState::Fermo => "FERMO",
                    UserState::InMovimento => "IN MOVIMENTO",
                    UserState::Disconnesso => "DISCONNESSO",
                };
                format!(
                    "POSIZIONE di {}: [Lat: {:.6}, Lon: {:.6}] — stato: {}",
                    self.mittente, lat, lon, stato_str
                )
            }
        }
    }
}
