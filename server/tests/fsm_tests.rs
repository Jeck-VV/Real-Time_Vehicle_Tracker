// =============================================================================
// Test FSM e validazione — server/tests/fsm_tests.rs
//
// Testa le API pubbliche del crate `server`:
//   - username_valido     (1.6, 1.7 e casi limite)
//   - posizione_valida    (9.1, 9.2 e casi limite)
//   - UserSessionData::aggiorna_posizione (FSM: 3.1–3.5, soglia esatta)
// =============================================================================

use chrono::Utc;
use common::{Posizione, UserState};
use server::{posizione_valida, username_valido, UserSessionData};

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn ora_attuale() -> u64 {
    Utc::now().timestamp() as u64
}

fn pos(lat: f64, lon: f64, ts: u64) -> Posizione {
    Posizione {
        lat,
        lon,
        timestamp: ts,
    }
}

// ─── username_valido ──────────────────────────────────────────────────────────

// Username valido classico
#[test]
fn username_valido_normale() {
    assert!(username_valido("anto"));
    assert!(username_valido("utente_123"));
    assert!(username_valido("A"));
    assert!(username_valido("abc123"));
}

// Username vuoto → non valido (1.7)
#[test]
fn username_valido_vuoto() {
    assert!(
        !username_valido(""),
        "Username vuoto non deve essere valido"
    );
}

// Username riservato "SERVER" (case-insensitive) → non valido
#[test]
fn username_valido_riservato_server() {
    assert!(!username_valido("SERVER"), "SERVER è riservato");
    assert!(!username_valido("server"), "server (minuscolo) è riservato");
    assert!(!username_valido("Server"), "Server (misto) è riservato");
}

// Username con '|' → non valido (romperebbe il protocollo)
#[test]
fn username_valido_con_pipe() {
    assert!(
        !username_valido("use|r"),
        "Username con | non deve essere valido"
    );
}

// Username con spazio → non valido
#[test]
fn username_valido_con_spazio() {
    assert!(
        !username_valido("nome utente"),
        "Username con spazio non deve essere valido"
    );
}

// Username troppo lungo (>32 char) → non valido
#[test]
fn username_valido_troppo_lungo() {
    let lungo = "a".repeat(33);
    assert!(
        !username_valido(&lungo),
        "Username di 33 caratteri non deve essere valido"
    );
}

// Username di esattamente 32 caratteri → valido
#[test]
fn username_valido_32_caratteri() {
    let max = "a".repeat(32);
    assert!(
        username_valido(&max),
        "Username di 32 caratteri deve essere valido"
    );
}

// Username con caratteri speciali non ASCII → non valido
#[test]
fn username_valido_caratteri_speciali() {
    assert!(
        !username_valido("utènte"),
        "Caratteri non ASCII non ammessi"
    );
    assert!(!username_valido("user@domain"), "@ non ammesso");
    assert!(!username_valido("user-name"), "- non ammesso");
}

// ─── posizione_valida ─────────────────────────────────────────────────────────

// Coordinate valide, timestamp recente → Ok
#[test]
fn posizione_valida_ok() {
    let ts = ora_attuale();
    assert!(posizione_valida(45.07, 7.68, ts).is_ok());
}

// 9.1 — Limiti geografici esatti → Ok
#[test]
fn posizione_valida_limiti_geografici() {
    let ts = ora_attuale();
    assert!(posizione_valida(90.0, 180.0, ts).is_ok());
    assert!(posizione_valida(-90.0, -180.0, ts).is_ok());
}

// Coordinate fuori range → Err
#[test]
fn posizione_invalida_coordinate_fuori_range() {
    let ts = ora_attuale();
    assert!(posizione_valida(91.0, 0.0, ts).is_err());
    assert!(posizione_valida(0.0, 181.0, ts).is_err());
    assert!(posizione_valida(-91.0, 0.0, ts).is_err());
}

// 9.2 — Timestamp nel futuro oltre la tolleranza (1h) → Err
#[test]
fn posizione_invalida_timestamp_futuro() {
    let ts_futuro = ora_attuale() + 3600;
    assert!(
        posizione_valida(45.0, 7.0, ts_futuro).is_err(),
        "Timestamp nel futuro deve essere rifiutato"
    );
}

// Timestamp nel futuro ma entro la tolleranza di 60s → Ok
#[test]
fn posizione_valida_timestamp_futuro_entro_tolleranza() {
    let ts_quasi_futuro = ora_attuale() + 30;
    assert!(
        posizione_valida(45.0, 7.0, ts_quasi_futuro).is_ok(),
        "Timestamp entro la tolleranza di 60s deve essere accettato"
    );
}

// ─── FSM: UserSessionData::aggiorna_posizione ────────────────────────────────

// 3.1 — Prima posizione → InMovimento
#[test]
fn fsm_prima_posizione_in_movimento() {
    let mut session = UserSessionData::new();
    let stato = session.aggiorna_posizione(pos(45.07, 7.68, ora_attuale()));
    assert_eq!(
        stato,
        UserState::InMovimento,
        "Prima posizione deve portare a InMovimento"
    );
}

// 3.2 — Seconda posizione diversa → rimane InMovimento
#[test]
fn fsm_seconda_posizione_diversa_in_movimento() {
    let mut session = UserSessionData::new();
    let ora = ora_attuale();
    session.aggiorna_posizione(pos(45.07, 7.68, ora - 600));
    let stato = session.aggiorna_posizione(pos(45.10, 7.70, ora));
    assert_eq!(stato, UserState::InMovimento);
}

// 3.3 — Stessa posizione, meno di 3 minuti → rimane InMovimento
#[test]
fn fsm_stessa_posizione_meno_3min_in_movimento() {
    let mut session = UserSessionData::new();
    let ora = ora_attuale();
    session.aggiorna_posizione(pos(45.07, 7.68, ora - 100));
    let stato = session.aggiorna_posizione(pos(45.07, 7.68, ora));
    assert_eq!(
        stato,
        UserState::InMovimento,
        "Stessa posizione per meno di 3min non deve essere Fermo"
    );
}

// 3.4 — Stessa posizione, almeno 3 minuti → Fermo
#[test]
fn fsm_stessa_posizione_almeno_3min_fermo() {
    let mut session = UserSessionData::new();
    let ora = ora_attuale();
    session.aggiorna_posizione(pos(45.07, 7.68, ora - 300));
    let stato = session.aggiorna_posizione(pos(45.07, 7.68, ora));
    assert_eq!(
        stato,
        UserState::Fermo,
        "Stessa posizione per >= 3min deve essere Fermo"
    );
}

// 3.5 — Da Fermo, posizione diversa → torna InMovimento
#[test]
fn fsm_da_fermo_posizione_diversa_in_movimento() {
    let mut session = UserSessionData::new();
    let ora = ora_attuale();
    session.aggiorna_posizione(pos(45.07, 7.68, ora - 400));
    session.aggiorna_posizione(pos(45.07, 7.68, ora - 100));
    let stato = session.aggiorna_posizione(pos(45.10, 7.70, ora));
    assert_eq!(
        stato,
        UserState::InMovimento,
        "Da Fermo, una nuova posizione diversa deve portare a InMovimento"
    );
}

// Lo storico viene accumulato ad ogni aggiornamento
#[test]
fn fsm_storico_accumulato() {
    let mut session = UserSessionData::new();
    let ora = ora_attuale();
    session.aggiorna_posizione(pos(45.0, 7.0, ora - 200));
    session.aggiorna_posizione(pos(45.1, 7.1, ora - 100));
    session.aggiorna_posizione(pos(45.2, 7.2, ora));
    assert_eq!(
        session.storico.len(),
        3,
        "Ogni aggiornamento deve aggiungere un punto allo storico"
    );
}

// 3.6 — Lo stato iniziale è Disconnesso
#[test]
fn fsm_stato_iniziale_disconnesso() {
    let session = UserSessionData::new();
    assert_eq!(
        session.stato,
        UserState::Disconnesso,
        "Lo stato iniziale deve essere Disconnesso"
    );
    assert!(session.storico.is_empty());
    assert!(session.ultima_posizione.is_none());
}

// Soglia esatta: 179s non basta, 180s è sufficiente per Fermo
#[test]
fn fsm_soglia_pausa_esatta() {
    let ora = ora_attuale();

    // 179 secondi: NON deve essere Fermo
    let mut s1 = UserSessionData::new();
    s1.aggiorna_posizione(pos(45.07, 7.68, ora - 179));
    let stato_179 = s1.aggiorna_posizione(pos(45.07, 7.68, ora));
    assert_ne!(
        stato_179,
        UserState::Fermo,
        "179s non deve essere sufficiente per Fermo"
    );

    // 180 secondi: DEVE essere Fermo
    let mut s2 = UserSessionData::new();
    s2.aggiorna_posizione(pos(45.07, 7.68, ora - 180));
    let stato_180 = s2.aggiorna_posizione(pos(45.07, 7.68, ora));
    assert_eq!(
        stato_180,
        UserState::Fermo,
        "180s deve essere sufficiente per Fermo"
    );
}
