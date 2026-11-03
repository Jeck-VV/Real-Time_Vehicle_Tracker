// =============================================================================
// Test protocollo e geometria — server/tests/common_tests.rs
//
// Testa le API pubbliche del crate `common`:
//   - ClientMessage encode/decode (round-trip, casi limite)
//   - ServerMessage encode/decode (round-trip, AnalysisResult, sanitizzazione)
//   - coordinate_valide (limiti geografici, NaN, infinito)
//   - distanza_haversine (valori noti, simmetria, soglie)
//   - BroadcastEvent::formatta_per_client
// =============================================================================

use common::{
    coordinate_valide, distanza_haversine, BroadcastEvent, ClientMessage, Interval, Posizione,
    ReportAnalisi, ServerMessage, TipoEvento, UserState, SOGLIA_MOVIMENTO_KM,
};

// ─── ClientMessage encode/decode ─────────────────────────────────────────────

// 2.1 — Comando sconosciuto restituisce Err
#[test]
fn client_decode_comando_sconosciuto() {
    let result = ClientMessage::decode("PIPPO|test");
    assert!(
        result.is_err(),
        "Un comando sconosciuto deve restituire Err"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.to_lowercase().contains("sconosciuto"),
        "Il messaggio di errore deve citare il problema: {msg}"
    );
}

// 2.2 — Spazi iniziali/finali tollerati dal trim() interno
#[test]
fn client_decode_con_spazi_iniziali() {
    let result = ClientMessage::decode("   REGISTER|anto3|1234");
    assert!(
        result.is_ok(),
        "REGISTER con spazi iniziali deve essere decodificato: {result:?}"
    );
    match result.unwrap() {
        ClientMessage::Register { username, password } => {
            assert_eq!(username, "anto3");
            assert_eq!(password, "1234");
        }
        other => panic!("Tipo messaggio inatteso: {other:?}"),
    }
}

// 2.3 — Messaggio vuoto restituisce Err (non crash)
#[test]
fn client_decode_riga_vuota() {
    let result = ClientMessage::decode("");
    assert!(result.is_err(), "Una riga vuota deve restituire Err");
}

// 2.4 — REGISTER con un solo campo (manca password) restituisce Err
#[test]
fn client_decode_register_campo_mancante() {
    let result = ClientMessage::decode("REGISTER|soloUsername");
    assert!(
        result.is_err(),
        "REGISTER senza password deve restituire Err"
    );
}

// 2.5 — POS con valori non numerici restituisce Err
#[test]
fn client_decode_pos_non_numerica() {
    let result = ClientMessage::decode("POS|abc|xyz|123");
    assert!(
        result.is_err(),
        "POS con lat/lon non numerica deve restituire Err"
    );
}

// POS con timestamp non numerico restituisce Err
#[test]
fn client_decode_pos_timestamp_non_numerico() {
    let result = ClientMessage::decode("POS|45.0|7.0|notanumber");
    assert!(result.is_err());
}

// Round-trip REGISTER
#[test]
fn client_roundtrip_register() {
    let msg = ClientMessage::Register {
        username: "anto".into(),
        password: "1234".into(),
    };
    let encoded = msg.encode();
    assert_eq!(encoded, "REGISTER|anto|1234");
    let decoded = ClientMessage::decode(&encoded).unwrap();
    match decoded {
        ClientMessage::Register { username, password } => {
            assert_eq!(username, "anto");
            assert_eq!(password, "1234");
        }
        other => panic!("Tipo inatteso: {other:?}"),
    }
}

// Round-trip LOGIN
#[test]
fn client_roundtrip_login() {
    let msg = ClientMessage::Login {
        username: "utente".into(),
        password: "pass".into(),
    };
    let encoded = msg.encode();
    let decoded = ClientMessage::decode(&encoded).unwrap();
    match decoded {
        ClientMessage::Login { username, password } => {
            assert_eq!(username, "utente");
            assert_eq!(password, "pass");
        }
        other => panic!("{other:?}"),
    }
}

// Round-trip POS
#[test]
fn client_roundtrip_pos() {
    let msg = ClientMessage::PositionUpdate {
        lat: 45.07,
        lon: 7.68,
        timestamp: 1_700_000_000,
    };
    let encoded = msg.encode();
    let decoded = ClientMessage::decode(&encoded).unwrap();
    match decoded {
        ClientMessage::PositionUpdate {
            lat,
            lon,
            timestamp,
        } => {
            assert!((lat - 45.07).abs() < 1e-9);
            assert!((lon - 7.68).abs() < 1e-9);
            assert_eq!(timestamp, 1_700_000_000);
        }
        other => panic!("{other:?}"),
    }
}

// Round-trip TEXT broadcast (to = None → "ALL")
#[test]
fn client_roundtrip_text_broadcast() {
    let msg = ClientMessage::Text {
        to: None,
        content: "ciao a tutti".into(),
    };
    let encoded = msg.encode();
    assert!(
        encoded.starts_with("TEXT|ALL|"),
        "Broadcast deve usare ALL: {encoded}"
    );
    let decoded = ClientMessage::decode(&encoded).unwrap();
    match decoded {
        ClientMessage::Text { to, content } => {
            assert!(to.is_none(), "to deve essere None per broadcast");
            assert_eq!(content, "ciao a tutti");
        }
        other => panic!("{other:?}"),
    }
}

// Round-trip TEXT privato
#[test]
fn client_roundtrip_text_privato() {
    let msg = ClientMessage::Text {
        to: Some("utente2".into()),
        content: "messaggio privato".into(),
    };
    let encoded = msg.encode();
    let decoded = ClientMessage::decode(&encoded).unwrap();
    match decoded {
        ClientMessage::Text { to, content } => {
            assert_eq!(to.as_deref(), Some("utente2"));
            assert_eq!(content, "messaggio privato");
        }
        other => panic!("{other:?}"),
    }
}

// Round-trip REQANALYSIS per tutti gli intervalli
#[test]
fn client_roundtrip_reqanalysis() {
    for (interval, atteso) in [
        (Interval::Day, "REQANALYSIS|DAY"),
        (Interval::Week, "REQANALYSIS|WEEK"),
        (Interval::Month, "REQANALYSIS|MONTH"),
    ] {
        let msg = ClientMessage::RequestAnalysis {
            interval: interval.clone(),
        };
        let encoded = msg.encode();
        assert_eq!(encoded, atteso, "Encoding errato per {interval:?}");
        let decoded = ClientMessage::decode(&encoded).unwrap();
        match decoded {
            ClientMessage::RequestAnalysis { interval: dec } => {
                assert_eq!(dec, interval)
            }
            other => panic!("{other:?}"),
        }
    }
}

// REQANALYSIS con intervallo sconosciuto restituisce Err
#[test]
fn client_decode_reqanalysis_intervallo_sconosciuto() {
    let result = ClientMessage::decode("REQANALYSIS|QUINDICENNALE");
    assert!(
        result.is_err(),
        "Intervallo sconosciuto deve restituire Err"
    );
}

// ─── ServerMessage encode/decode ─────────────────────────────────────────────

// Round-trip RegisterOk
#[test]
fn server_roundtrip_register_ok() {
    let encoded = ServerMessage::RegisterOk.encode();
    assert_eq!(encoded, "REGISTER_OK");
    assert!(matches!(
        ServerMessage::decode(&encoded).unwrap(),
        ServerMessage::RegisterOk
    ));
}

// Round-trip RegisterErr
#[test]
fn server_roundtrip_register_err() {
    let msg = ServerMessage::RegisterErr("Utente già esistente".into());
    let encoded = msg.encode();
    match ServerMessage::decode(&encoded).unwrap() {
        ServerMessage::RegisterErr(reason) => assert_eq!(reason, "Utente già esistente"),
        other => panic!("{other:?}"),
    }
}

// Round-trip LoginOk / LoginErr
#[test]
fn server_roundtrip_login() {
    assert_eq!(ServerMessage::LoginOk.encode(), "LOGIN_OK");
    let err = ServerMessage::LoginErr("Credenziali non valide".into());
    let enc = err.encode();
    match ServerMessage::decode(&enc).unwrap() {
        ServerMessage::LoginErr(r) => assert_eq!(r, "Credenziali non valide"),
        other => panic!("{other:?}"),
    }
}

// Round-trip Notice
#[test]
fn server_roundtrip_notice() {
    let msg = ServerMessage::Notice("testo avviso".into());
    let enc = msg.encode();
    match ServerMessage::decode(&enc).unwrap() {
        ServerMessage::Notice(t) => assert_eq!(t, "testo avviso"),
        other => panic!("{other:?}"),
    }
}

// Round-trip AnalysisResult per tutti gli intervalli
#[test]
fn server_roundtrip_analysis_result() {
    for intervallo in [Interval::Day, Interval::Week, Interval::Month] {
        let report = ReportAnalisi {
            distanza_totale_km: 12.5,
            velocita_media_kmh: 5.0,
            tempo_in_movimento_sec: 9000,
            tempo_in_pausa_sec: 600,
        };
        let msg = ServerMessage::AnalysisResult {
            utente: "anto".into(),
            intervallo: intervallo.clone(),
            report,
        };
        let encoded = msg.encode();
        match ServerMessage::decode(&encoded).unwrap() {
            ServerMessage::AnalysisResult {
                utente,
                intervallo: int_dec,
                report: r,
            } => {
                assert_eq!(utente, "anto");
                assert_eq!(int_dec, intervallo);
                assert!((r.distanza_totale_km - 12.5).abs() < 1e-4);
                assert!((r.velocita_media_kmh - 5.0).abs() < 1e-4);
                assert_eq!(r.tempo_in_movimento_sec, 9000);
                assert_eq!(r.tempo_in_pausa_sec, 600);
            }
            other => panic!("{other:?}"),
        }
    }
}

// ServerMessage sconosciuto restituisce Err
#[test]
fn server_decode_comando_sconosciuto() {
    let result = ServerMessage::decode("PIPPO|qualcosa");
    assert!(result.is_err());
}

// Sanitizzazione: newline in un campo Notice viene rimpiazzato con spazio
#[test]
fn server_notice_newline_sanitizzato() {
    let msg = ServerMessage::Notice("riga1\nriga2".into());
    let encoded = msg.encode();
    let senza_tag = encoded.trim_start_matches("NOTICE|");
    assert!(
        !senza_tag.contains('\n'),
        "Il campo non deve contenere newline interne: {encoded:?}"
    );
    assert!(
        senza_tag.contains("riga1 riga2"),
        "Newline deve diventare spazio: {encoded}"
    );
}

// ─── coordinate_valide ───────────────────────────────────────────────────────

// 9.1 — Limiti geografici esatti sono validi
#[test]
fn coordinate_valide_limiti_esatti() {
    assert!(coordinate_valide(90.0, 180.0));
    assert!(coordinate_valide(-90.0, -180.0));
    assert!(coordinate_valide(0.0, 0.0));
    assert!(coordinate_valide(45.07, 7.68));
}

// Oltre i limiti → non valide
#[test]
fn coordinate_invalide_oltre_limiti() {
    assert!(!coordinate_valide(90.001, 0.0));
    assert!(!coordinate_valide(-90.001, 0.0));
    assert!(!coordinate_valide(0.0, 180.001));
    assert!(!coordinate_valide(0.0, -180.001));
}

// NaN e infinito → non valide
#[test]
fn coordinate_invalide_nan_infinito() {
    assert!(!coordinate_valide(f64::NAN, 0.0));
    assert!(!coordinate_valide(0.0, f64::NAN));
    assert!(!coordinate_valide(f64::INFINITY, 0.0));
    assert!(!coordinate_valide(0.0, f64::NEG_INFINITY));
}

// ─── distanza_haversine ──────────────────────────────────────────────────────

// Punti identici → distanza zero
#[test]
fn haversine_punti_identici() {
    let p = Posizione {
        lat: 45.07,
        lon: 7.68,
        timestamp: 0,
    };
    let dist = distanza_haversine(&p, &p);
    assert!(
        dist < 1e-9,
        "Distanza da punto a sé stesso deve essere ~0: {dist}"
    );
}

// 9.1 — Poli: distanza polo nord → polo sud ≈ π * R ≈ 20015 km
#[test]
fn haversine_polo_nord_sud() {
    let nord = Posizione {
        lat: 90.0,
        lon: 0.0,
        timestamp: 0,
    };
    let sud = Posizione {
        lat: -90.0,
        lon: 0.0,
        timestamp: 0,
    };
    let dist = distanza_haversine(&nord, &sud);
    assert!(
        (dist - 20015.09).abs() < 1.0,
        "Distanza poli attesa ~20015 km, ottenuta {dist:.2} km"
    );
}

// 5.9 — Coerenza numerica: Torino → Milano ≈ 126 km
#[test]
fn haversine_torino_milano() {
    let torino = Posizione {
        lat: 45.0703,
        lon: 7.6869,
        timestamp: 0,
    };
    let milano = Posizione {
        lat: 45.4654,
        lon: 9.1859,
        timestamp: 0,
    };
    let dist = distanza_haversine(&torino, &milano);
    assert!(
        (dist - 126.0).abs() < 2.0,
        "Distanza Torino-Milano attesa ~126 km, ottenuta {dist:.2} km"
    );
}

// Distanza è simmetrica
#[test]
fn haversine_simmetria() {
    let p1 = Posizione {
        lat: 45.0703,
        lon: 7.6869,
        timestamp: 0,
    };
    let p2 = Posizione {
        lat: 45.4654,
        lon: 9.1859,
        timestamp: 0,
    };
    let d12 = distanza_haversine(&p1, &p2);
    let d21 = distanza_haversine(&p2, &p1);
    assert!(
        (d12 - d21).abs() < 1e-9,
        "Haversine deve essere simmetrica: {d12} vs {d21}"
    );
}

// Spostamento sotto soglia SOGLIA_MOVIMENTO_KM (~1 m < 5 m)
#[test]
fn haversine_sotto_soglia_movimento() {
    let p1 = Posizione {
        lat: 45.0,
        lon: 7.0,
        timestamp: 0,
    };
    let p2 = Posizione {
        lat: 45.000009,
        lon: 7.0,
        timestamp: 0,
    };
    let dist = distanza_haversine(&p1, &p2);
    assert!(
        dist < SOGLIA_MOVIMENTO_KM,
        "1 m deve essere sotto soglia {SOGLIA_MOVIMENTO_KM} km: {dist}"
    );
}

// Spostamento sopra soglia SOGLIA_MOVIMENTO_KM (~111 m > 5 m)
#[test]
fn haversine_sopra_soglia_movimento() {
    let p1 = Posizione {
        lat: 45.0,
        lon: 7.0,
        timestamp: 0,
    };
    let p2 = Posizione {
        lat: 45.001,
        lon: 7.0,
        timestamp: 0,
    };
    let dist = distanza_haversine(&p1, &p2);
    assert!(
        dist > SOGLIA_MOVIMENTO_KM,
        "~111 m deve essere sopra soglia {SOGLIA_MOVIMENTO_KM} km: {dist}"
    );
}

// ─── BroadcastEvent::formatta_per_client ─────────────────────────────────────

#[test]
fn broadcast_event_formato_testo() {
    let ev = BroadcastEvent {
        mittente: "anto".into(),
        tipo: TipoEvento::MessaggioTesto {
            contenuto: "ciao".into(),
        },
    };
    let formatted = ev.formatta_per_client();
    assert!(formatted.contains("anto"), "Deve contenere il mittente");
    assert!(formatted.contains("ciao"), "Deve contenere il contenuto");
}

#[test]
fn broadcast_event_formato_posizione_in_movimento() {
    let ev = BroadcastEvent {
        mittente: "anto".into(),
        tipo: TipoEvento::AggiornamentoPosizione {
            lat: 45.07,
            lon: 7.68,
            stato: UserState::InMovimento,
        },
    };
    let formatted = ev.formatta_per_client();
    assert!(formatted.contains("anto"));
    assert!(
        formatted.contains("IN MOVIMENTO"),
        "Stato IN MOVIMENTO non trovato: {formatted}"
    );
}

#[test]
fn broadcast_event_formato_posizione_fermo() {
    let ev = BroadcastEvent {
        mittente: "luigi".into(),
        tipo: TipoEvento::AggiornamentoPosizione {
            lat: 41.9,
            lon: 12.5,
            stato: UserState::Fermo,
        },
    };
    let formatted = ev.formatta_per_client();
    assert!(formatted.contains("luigi"));
    assert!(
        formatted.contains("FERMO"),
        "Stato FERMO non trovato: {formatted}"
    );
}
