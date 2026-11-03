// =============================================================================
// Test analisi movimento — server/tests/analisi_tests.rs
//
// Testa `server::analisi::elabora_statistiche` tramite la sua API pubblica.
// Copertura:
//   5.1 — Storico vuoto → tutto a zero
//   5.2 — Un solo punto → tutto a zero
//   5.3 — Timestamp nel passato → filtrati
//   5.4 — Dati validi Day
//   5.5 — Intervallo Week
//   5.6 — Intervallo Month
//   5.7 — Pausa registrata
//   5.9 — Coerenza numerica Haversine
//   9.2 — Timestamp futuro escluso
// =============================================================================

use chrono::{Datelike, TimeZone, Utc};
use common::{Interval, Posizione};
use server::analisi::elabora_statistiche;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn pos(lat: f64, lon: f64, ts: u64) -> Posizione {
    Posizione {
        lat,
        lon,
        timestamp: ts,
    }
}

fn ora_attuale() -> u64 {
    Utc::now().timestamp() as u64
}

// ─── Test ────────────────────────────────────────────────────────────────────

// 5.1 — Storico vuoto → tutto a zero
#[test]
fn analisi_storico_vuoto() {
    let report = elabora_statistiche(&[], &Interval::Day);
    assert_eq!(report.distanza_totale_km, 0.0);
    assert_eq!(report.velocita_media_kmh, 0.0);
    assert_eq!(report.tempo_in_movimento_sec, 0);
    assert_eq!(report.tempo_in_pausa_sec, 0);
}

// 5.2 — Un solo punto → tutto a zero (serve almeno 2 punti)
#[test]
fn analisi_un_solo_punto() {
    let storico = vec![pos(45.07, 7.68, ora_attuale())];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert_eq!(report.distanza_totale_km, 0.0);
    assert_eq!(report.velocita_media_kmh, 0.0);
    assert_eq!(report.tempo_in_movimento_sec, 0);
    assert_eq!(report.tempo_in_pausa_sec, 0);
}

// 5.3 — Timestamp fuori dall'intervallo (2020) → filtrati, risultato a zero
#[test]
fn analisi_timestamp_fuori_intervallo_day() {
    let storico = vec![
        pos(45.07, 7.68, 1_580_000_000),
        pos(45.10, 7.70, 1_580_001_000),
    ];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert_eq!(
        report.distanza_totale_km, 0.0,
        "Punti del 2020 devono essere filtrati per Day"
    );
}

// 5.3 variante: timestamp fuori dalla settimana corrente
#[test]
fn analisi_timestamp_fuori_intervallo_week() {
    let storico = vec![
        pos(45.07, 7.68, 1_580_000_000),
        pos(45.10, 7.70, 1_580_001_000),
    ];
    let report = elabora_statistiche(&storico, &Interval::Week);
    assert_eq!(
        report.distanza_totale_km, 0.0,
        "Punti del 2020 devono essere filtrati per Week"
    );
}

// 9.2 — Timestamp nel futuro → escluso dal calcolo
#[test]
fn analisi_timestamp_futuro_escluso() {
    let ora = ora_attuale();
    let futuro = ora + 86400; // domani
    let storico = vec![pos(45.07, 7.68, ora - 100), pos(45.10, 7.70, futuro)];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert_eq!(
        report.distanza_totale_km, 0.0,
        "Il punto futuro deve essere filtrato"
    );
}

// 5.4 — Dati validi con intervallo Day: distanza e tempo coerenti
#[test]
fn analisi_dati_validi_day() {
    let ora = ora_attuale();
    // Due punti distanti ~111 m (Δlat = 0.001°), 600 secondi di distanza
    let storico = vec![pos(45.0, 7.0, ora - 1000), pos(45.001, 7.0, ora - 400)];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert!(
        report.distanza_totale_km > 0.0,
        "Distanza attesa > 0: {}",
        report.distanza_totale_km
    );
    assert_eq!(report.tempo_in_pausa_sec, 0, "Nessuna pausa attesa");
    assert_eq!(
        report.tempo_in_movimento_sec, 600,
        "Tutto il tempo deve essere in movimento"
    );
}

// 5.9 — Coerenza numerica Haversine: Torino → Milano ≈ 126 km / 126 km/h
#[test]
fn analisi_coerenza_numerica_haversine() {
    let ora = ora_attuale();
    let storico = vec![
        pos(45.0703, 7.6869, ora - 7200), // 2h fa
        pos(45.4654, 9.1859, ora - 3600), // 1h fa
    ];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert!(
        (report.distanza_totale_km - 126.0).abs() < 2.0,
        "Distanza Torino-Milano attesa ~126 km, ottenuta {:.2} km",
        report.distanza_totale_km
    );
    assert!(
        (report.velocita_media_kmh - 126.0).abs() < 2.0,
        "Velocità media attesa ~126 km/h, ottenuta {:.2} km/h",
        report.velocita_media_kmh
    );
}

// 5.7 — Pausa registrata correttamente dopo ripartenza
#[test]
fn analisi_pausa_registrata() {
    let ora = ora_attuale();
    let storico = vec![
        pos(45.0, 7.0, ora - 700),   // punto iniziale
        pos(45.0, 7.0, ora - 400),   // stesso posto, +300s (fermo)
        pos(45.001, 7.0, ora - 300), // si sposta (sopra soglia)
        pos(45.002, 7.0, ora - 100), // continua a muoversi
    ];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert!(
        report.tempo_in_pausa_sec > 0,
        "Deve esserci tempo in pausa: pausa={}",
        report.tempo_in_pausa_sec
    );
    assert!(
        report.tempo_in_movimento_sec > 0,
        "Deve esserci tempo in movimento: mov={}",
        report.tempo_in_movimento_sec
    );
    let tempo_totale = report.tempo_in_pausa_sec + report.tempo_in_movimento_sec;
    assert_eq!(
        tempo_totale, 600,
        "Tempo totale deve essere 600s: {tempo_totale}"
    );
}

// 5.5 — Intervallo Week include dati di 3 giorni fa (skip condizionale se lunedì/martedì)
#[test]
fn analisi_intervallo_week() {
    let ora = ora_attuale();
    let tre_giorni_fa = ora.saturating_sub(3 * 86400);
    let inizio_week = {
        let oggi = Utc::now().date_naive();
        let giorni = oggi.weekday().num_days_from_monday() as u64;
        ora.saturating_sub(giorni * 86400 + ora % 86400)
    };
    if tre_giorni_fa < inizio_week {
        return; // 3 giorni fa è la settimana scorsa
    }
    let storico = vec![
        pos(45.0, 7.0, tre_giorni_fa),
        pos(45.001, 7.0, tre_giorni_fa + 3600),
    ];
    let report = elabora_statistiche(&storico, &Interval::Week);
    assert!(
        report.distanza_totale_km > 0.0,
        "Punti di 3 giorni fa devono essere inclusi nell'intervallo Week"
    );
}

// 5.6 — Intervallo Month include dati di 10 giorni fa (skip condizionale se <10 del mese)
#[test]
fn analisi_intervallo_month() {
    let ora = ora_attuale();
    let dieci_giorni_fa = ora.saturating_sub(10 * 86400);
    let inizio_mese = {
        let oggi = Utc::now().date_naive();
        let primo = oggi.with_day(1).unwrap();
        let inizio = primo.and_hms_opt(0, 0, 0).unwrap();
        Utc.from_utc_datetime(&inizio).timestamp() as u64
    };
    if dieci_giorni_fa < inizio_mese {
        return; // 10 giorni fa era il mese scorso
    }
    let storico = vec![
        pos(45.0, 7.0, dieci_giorni_fa),
        pos(45.001, 7.0, dieci_giorni_fa + 3600),
    ];
    let report = elabora_statistiche(&storico, &Interval::Month);
    assert!(
        report.distanza_totale_km > 0.0,
        "Punti di 10 giorni fa devono essere inclusi nell'intervallo Month"
    );
}

// Punti sotto soglia (< 5m) non aumentano la distanza
#[test]
fn analisi_punti_sotto_soglia_non_contano() {
    let ora = ora_attuale();
    let storico = vec![pos(45.0, 7.0, ora - 1000), pos(45.000009, 7.0, ora - 500)];
    let report = elabora_statistiche(&storico, &Interval::Day);
    assert_eq!(
        report.distanza_totale_km, 0.0,
        "Spostamento sotto soglia non deve essere contato"
    );
    assert_eq!(report.tempo_in_pausa_sec, 500);
    assert_eq!(report.tempo_in_movimento_sec, 0);
}

// Punti fuori ordine vengono riordinati per timestamp prima del calcolo
#[test]
fn analisi_punti_fuori_ordine_riordinati() {
    let ora = ora_attuale();
    let storico_inv = vec![
        pos(45.001, 7.0, ora - 400), // secondo cronologicamente
        pos(45.0, 7.0, ora - 1000),  // primo cronologicamente
    ];
    let storico_ord = vec![pos(45.0, 7.0, ora - 1000), pos(45.001, 7.0, ora - 400)];
    let r_inv = elabora_statistiche(&storico_inv, &Interval::Day);
    let r_ord = elabora_statistiche(&storico_ord, &Interval::Day);
    assert!(
        (r_inv.distanza_totale_km - r_ord.distanza_totale_km).abs() < 1e-9,
        "L'ordinamento interno deve produrre lo stesso risultato"
    );
}
