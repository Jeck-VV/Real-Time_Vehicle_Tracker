use chrono::{Datelike, TimeZone, Utc};
use common::{distanza_haversine, Interval, Posizione, ReportAnalisi, SOGLIA_MOVIMENTO_KM};

/// Inizio (00:00 UTC) del giorno corrente, in secondi epoch.
fn inizio_giorno_corrente() -> u64 {
    let oggi = Utc::now().date_naive();
    let inizio = oggi.and_hms_opt(0, 0, 0).unwrap();
    Utc.from_utc_datetime(&inizio).timestamp() as u64
}

/// Inizio (lunedì 00:00 UTC) della settimana corrente, in secondi epoch.
fn inizio_settimana_corrente() -> u64 {
    let oggi = Utc::now().date_naive();
    let giorni_da_lunedi = oggi.weekday().num_days_from_monday() as i64;
    let lunedi = oggi - chrono::Duration::days(giorni_da_lunedi);
    let inizio = lunedi.and_hms_opt(0, 0, 0).unwrap();
    Utc.from_utc_datetime(&inizio).timestamp() as u64
}

/// Inizio (1° del mese, 00:00 UTC) del mese corrente, in secondi epoch.
fn inizio_mese_corrente() -> u64 {
    let oggi = Utc::now().date_naive();
    let primo_del_mese = oggi.with_day(1).unwrap();
    let inizio = primo_del_mese.and_hms_opt(0, 0, 0).unwrap();
    Utc.from_utc_datetime(&inizio).timestamp() as u64
}

pub fn elabora_statistiche(storico: &[Posizione], intervallo: &Interval) -> ReportAnalisi {
    let cutoff_ts = match intervallo {
        Interval::Day => inizio_giorno_corrente(),
        Interval::Week => inizio_settimana_corrente(),
        Interval::Month => inizio_mese_corrente(),
    };

    let ora_attuale = Utc::now().timestamp() as u64;
    let mut storico_filtrato: Vec<&Posizione> = storico
        .iter()
        .filter(|p| p.timestamp >= cutoff_ts && p.timestamp <= ora_attuale)
        .collect();

    storico_filtrato.sort_by_key(|p| p.timestamp);

    let mut distanza_totale = 0.0;
    let mut tempo_in_pausa_sec = 0;
    let mut tempo_totale_sec = 0;

    if storico_filtrato.len() >= 2 {
        let primo_ts = storico_filtrato.first().unwrap().timestamp;
        let ultimo_ts = storico_filtrato.last().unwrap().timestamp;

        if ultimo_ts > primo_ts {
            tempo_totale_sec = ultimo_ts - primo_ts;
        }

        let mut tempo_fermo_corrente = 0;

        for window in storico_filtrato.windows(2) {
            let p1 = window[0];
            let p2 = window[1];

            let dist = distanza_haversine(p1, p2);
            let delta_t = p2.timestamp.saturating_sub(p1.timestamp);

            if dist > SOGLIA_MOVIMENTO_KM {
                distanza_totale += dist;
                tempo_in_pausa_sec += tempo_fermo_corrente;
                tempo_fermo_corrente = 0;
            } else {
                tempo_fermo_corrente += delta_t;
            }
        }
        tempo_in_pausa_sec += tempo_fermo_corrente;
    }

    let tempo_in_movimento_sec = tempo_totale_sec.saturating_sub(tempo_in_pausa_sec);

    let velocita_media_kmh = if tempo_in_movimento_sec > 0 {
        let tempo_in_ore = tempo_in_movimento_sec as f64 / 3600.0;
        distanza_totale / tempo_in_ore
    } else {
        0.0
    };

    ReportAnalisi {
        distanza_totale_km: distanza_totale,
        velocita_media_kmh,
        tempo_in_movimento_sec,
        tempo_in_pausa_sec,
    }
}
