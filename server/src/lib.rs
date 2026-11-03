use chrono::Utc;
use common::{Posizione, UserState};

pub mod analisi;

const SERVER_TARGET: &str = "SERVER";
const TOLLERANZA_CLOCK_SKEW_SEC: u64 = 60;

/// Username minimo accettabile.
pub fn username_valido(username: &str) -> bool {
    let lunghezza_ok = (1..=32).contains(&username.chars().count());
    let charset_ok = username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_');
    let non_riservato = !username.eq_ignore_ascii_case(SERVER_TARGET);
    lunghezza_ok && charset_ok && non_riservato
}

/// Valida coordinate e timestamp di una posizione ricevuta da un client.
pub fn posizione_valida(lat: f64, lon: f64, timestamp: u64) -> Result<(), &'static str> {
    if !common::coordinate_valide(lat, lon) {
        return Err("coordinate non valide: lat deve essere in [-90, 90] e lon in [-180, 180]");
    }
    let ora_attuale = Utc::now().timestamp() as u64;
    if timestamp > ora_attuale.saturating_add(TOLLERANZA_CLOCK_SKEW_SEC) {
        return Err("timestamp nel futuro: controlla l'orologio del client");
    }
    Ok(())
}

pub struct UserSessionData {
    pub stato: UserState,
    pub storico: Vec<Posizione>,
    pub ultima_posizione: Option<Posizione>,
    pub timestamp_ultimo_cambio: u64,
}

impl Default for UserSessionData {
    fn default() -> Self {
        Self::new()
    }
}

impl UserSessionData {
    pub fn new() -> Self {
        Self {
            stato: UserState::Disconnesso,
            storico: Vec::new(),
            ultima_posizione: None,
            timestamp_ultimo_cambio: 0,
        }
    }

    pub fn aggiorna_posizione(&mut self, pos: Posizione) -> UserState {
        let stato_precedente = self.stato;

        match &self.ultima_posizione {
            None => {
                self.stato = UserState::InMovimento;
                self.timestamp_ultimo_cambio = pos.timestamp;
            }
            Some(ultima) => {
                let fermo_rispetto_a_ultima =
                    common::distanza_haversine(ultima, &pos) <= common::SOGLIA_MOVIMENTO_KM;

                if fermo_rispetto_a_ultima {
                    let secondi_fermo = pos.timestamp.saturating_sub(self.timestamp_ultimo_cambio);
                    if secondi_fermo >= common::SOGLIA_PAUSA_SEC {
                        self.stato = UserState::Fermo;
                    }
                } else {
                    self.stato = UserState::InMovimento;
                    self.timestamp_ultimo_cambio = pos.timestamp;
                }
            }
        }

        self.ultima_posizione = Some(pos);
        self.storico.push(pos);

        if self.stato != stato_precedente {
            println!(
                "  [FSM] Transizione stato: {:?} → {:?}",
                stato_precedente, self.stato
            );
        }

        self.stato
    }
}
