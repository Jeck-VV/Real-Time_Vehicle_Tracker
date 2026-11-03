mod simulator;

use common::{ClientMessage, ServerMessage};
use eframe::egui;
use simulator::{FileSource, PositionSource, RandomWalkSource, SimulationStrategy};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::Duration;

// ---------------------------------------------------------------------------
// Tipi per la chat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum TipoChatMsg {
    Broadcast,
    Privato,
    Server,
}

#[derive(Debug, Clone)]
struct ChatMsg {
    mittente: String,
    contenuto: String,
    tipo: TipoChatMsg,
}

// Tab attiva nella chat
#[derive(Debug, Clone, PartialEq)]
enum ChatTab {
    Broadcast,
    Private(String),
    Server,
}

// Evento inviato dal thread di rete alla GUI
enum GuiEvent {
    Log(String),
    Chat(ChatMsg),
    LoginOk,
}

// ---------------------------------------------------------------------------
// Simulatore
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
enum StrategiaScelta {
    Nessuna,
    RandomWalk,
    DaFile,
}

struct SimulatorState {
    attivo: bool,
    stop_flag: Arc<AtomicBool>,
}
impl SimulatorState {
    fn new() -> Self {
        Self {
            attivo: false,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ---------------------------------------------------------------------------
// Struttura GUI
// ---------------------------------------------------------------------------
struct GeoruggineClient {
    username_input: String,
    password_input: String,
    lat_input: String,
    lon_input: String,
    chat_dest: String,
    chat_msg_input: String,
    is_logged_in: bool,

    // Log di sistema
    server_logs: Vec<String>,

    // Chat organizzata per tab
    chat_broadcast: Vec<ChatMsg>,                // tab "Tutti"
    chat_private: HashMap<String, Vec<ChatMsg>>, // tab per interlocutore
    chat_server: Vec<ChatMsg>,                   // tab "SERVER"
    active_tab: ChatTab,

    net_tx: tokio::sync::mpsc::UnboundedSender<String>,
    gui_rx: mpsc::Receiver<GuiEvent>,

    strategia_scelta: StrategiaScelta,
    file_path_input: String,
    sim_state: SimulatorState,
    rt_handle: tokio::runtime::Handle,

    errore_posizione: Option<String>,
}

impl eframe::App for GeoruggineClient {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut nuovo_messaggio = false;
        while let Ok(event) = self.gui_rx.try_recv() {
            nuovo_messaggio = true;
            match event {
                GuiEvent::Log(msg) => self.server_logs.push(msg),
                GuiEvent::Chat(msg) => self.route_chat_msg(msg),
                GuiEvent::LoginOk => {
                    self.server_logs.push("✅ Login effettuato".into());
                    self.is_logged_in = true;
                }
            }
        }

        // Controlla fine simulatore
        if self.sim_state.attivo && self.sim_state.stop_flag.load(Ordering::Relaxed) {
            self.sim_state.attivo = false;
            self.server_logs
                .push("[Simulatore] Sequenza terminata.".into());
        }

        // Barra superiore
        ui.horizontal(|ui| {
            ui.heading("🌍 Georuggine");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🚪 Esci").clicked() {
                    std::process::exit(0);
                }
                if ui.button("🌙").on_hover_text("Tema Scuro").clicked() {
                    ui.ctx().set_visuals(egui::Visuals::dark());
                }
                if ui.button("☀️").on_hover_text("Tema Chiaro").clicked() {
                    ui.ctx().set_visuals(egui::Visuals::light());
                }
            });
        });
        ui.separator();

        // Layout a due colonne
        ui.columns(2, |cols| {
            // colonna sx — controlli + log di sistema
            let left = &mut cols[0];

            if !self.is_logged_in {
                left.heading("🔑 Autenticazione");
                egui::Grid::new("auth_grid")
                    .num_columns(2)
                    .spacing([6.0, 6.0])
                    .show(left, |ui| {
                        ui.label("Username:");
                        ui.text_edit_singleline(&mut self.username_input);
                        ui.end_row();
                        ui.label("Password:");
                        ui.add(egui::TextEdit::singleline(&mut self.password_input).password(true));
                        ui.end_row();
                    });
                left.horizontal(|ui| {
                    if ui.button("Registrati").clicked() {
                        let _ = self.net_tx.send(format!(
                            "REGISTER|{}|{}",
                            self.username_input, self.password_input
                        ));
                    }
                    if ui.button("Login").clicked() {
                        let _ = self.net_tx.send(format!(
                            "LOGIN|{}|{}",
                            self.username_input, self.password_input
                        ));
                    }
                });
                left.separator();
            }

            left.heading("📍 Posizione (manuale)");
            egui::Grid::new("pos_grid")
                .num_columns(2)
                .spacing([6.0, 6.0])
                .show(left, |ui| {
                    ui.label("Lat:");
                    ui.text_edit_singleline(&mut self.lat_input);
                    ui.end_row();
                    ui.label("Lon:");
                    ui.text_edit_singleline(&mut self.lon_input);
                    ui.end_row();
                });
            if left.button("Invia Posizione").clicked() {
                match (
                    self.lat_input.trim().parse::<f64>(),
                    self.lon_input.trim().parse::<f64>(),
                ) {
                    (Ok(lat), Ok(lon)) => {
                        let ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        let _ = self.net_tx.send(format!("POS|{}|{}|{}", lat, lon, ts));
                        self.lat_input.clear();
                        self.lon_input.clear();
                        self.errore_posizione = None;
                    }
                    _ => {
                        self.errore_posizione = Some(
                            "Latitudine e longitudine devono essere numeri validi (es. 45.07)"
                                .into(),
                        );
                    }
                }
            }
            if let Some(errore) = &self.errore_posizione {
                left.colored_label(egui::Color32::RED, errore);
            }
            left.separator();

            left.heading("🚗 Simulatore");
            left.add_enabled_ui(!self.sim_state.attivo, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.strategia_scelta,
                        StrategiaScelta::Nessuna,
                        "Off",
                    );
                    ui.selectable_value(
                        &mut self.strategia_scelta,
                        StrategiaScelta::RandomWalk,
                        "🚶 Random Walk",
                    );
                    ui.selectable_value(
                        &mut self.strategia_scelta,
                        StrategiaScelta::DaFile,
                        "📂 Da file CSV",
                    );
                });
                if self.strategia_scelta == StrategiaScelta::DaFile {
                    ui.horizontal(|ui| {
                        ui.label("CSV:");
                        ui.text_edit_singleline(&mut self.file_path_input);
                    });
                }
            });
            left.horizontal(|ui| {
                let avvia_ok = self.is_logged_in
                    && !self.sim_state.attivo
                    && self.strategia_scelta != StrategiaScelta::Nessuna
                    && (self.strategia_scelta != StrategiaScelta::DaFile
                        || !self.file_path_input.is_empty());
                if ui
                    .add_enabled(avvia_ok, egui::Button::new("▶ Avvia"))
                    .clicked()
                {
                    self.avvia_simulatore();
                }
                if ui
                    .add_enabled(self.sim_state.attivo, egui::Button::new("⏹ Ferma"))
                    .clicked()
                {
                    self.ferma_simulatore();
                }
                if self.sim_state.attivo {
                    ui.label(egui::RichText::new("● attivo").color(egui::Color32::GREEN));
                }
            });
            left.separator();

            left.heading("📈 Analisi movimento");
            left.horizontal(|ui| {
                if ui.button("Giorno").clicked() {
                    let _ = self.net_tx.send("REQANALYSIS|DAY".into());
                }
                if ui.button("Settimana").clicked() {
                    let _ = self.net_tx.send("REQANALYSIS|WEEK".into());
                }
                if ui.button("Mese").clicked() {
                    let _ = self.net_tx.send("REQANALYSIS|MONTH".into());
                }
            });
            left.separator();

            left.heading("📋 Log di sistema");
            egui::ScrollArea::vertical()
                .id_salt("log_scroll")
                .max_height(180.0)
                .stick_to_bottom(true)
                .show(left, |ui| {
                    for log in &self.server_logs {
                        ui.label(egui::RichText::new(log).size(12.0));
                    }
                });

            // colonna dx — chat con tab
            let right = &mut cols[1];
            right.heading("💬 Chat");
            right.separator();

            // Tab bar
            right.horizontal(|ui| {
                // Tab Broadcast
                let bc_label = format!("📢 Tutti ({})", self.chat_broadcast.len());
                if ui
                    .selectable_label(self.active_tab == ChatTab::Broadcast, &bc_label)
                    .clicked()
                {
                    self.active_tab = ChatTab::Broadcast;
                    self.chat_dest.clear();
                }

                // Tab per ogni interlocutore privato (ordinati per nome)
                let mut interlocutori: Vec<String> = self.chat_private.keys().cloned().collect();
                interlocutori.sort();
                for nome in interlocutori {
                    let count = self.chat_private[&nome].len();
                    let label = format!("🔒 {} ({})", nome, count);
                    let is_active = self.active_tab == ChatTab::Private(nome.clone());
                    if ui.selectable_label(is_active, &label).clicked() {
                        self.active_tab = ChatTab::Private(nome.clone());
                        self.chat_dest = nome.clone();
                    }
                }

                // Tab SERVER
                let sv_label = format!("🖥 Server ({})", self.chat_server.len());
                if ui
                    .selectable_label(self.active_tab == ChatTab::Server, &sv_label)
                    .clicked()
                {
                    self.active_tab = ChatTab::Server;
                    self.chat_dest = "SERVER".to_string();
                }
            });
            right.separator();

            // Area messaggi
            let chat_height = right.available_height() - 58.0;
            let msgs_to_show: Vec<ChatMsg> = match &self.active_tab {
                ChatTab::Broadcast => self.chat_broadcast.clone(),
                ChatTab::Private(nome) => self.chat_private.get(nome).cloned().unwrap_or_default(),
                ChatTab::Server => self.chat_server.clone(),
            };

            egui::ScrollArea::vertical()
                .id_salt("chat_scroll")
                .max_height(chat_height)
                .stick_to_bottom(true)
                .show(right, |ui| {
                    if msgs_to_show.is_empty() {
                        ui.label(
                            egui::RichText::new("Nessun messaggio in questa sezione…")
                                .color(egui::Color32::GRAY)
                                .italics(),
                        );
                    } else {
                        for msg in &msgs_to_show {
                            render_chat_msg(ui, msg);
                        }
                    }
                });

            right.separator();

            // Barra di invio
            right.horizontal(|ui| {
                ui.label("A:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.chat_dest)
                        .hint_text("tutti")
                        .desired_width(70.0),
                );
                ui.add_space(4.0);

                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.chat_msg_input)
                        .hint_text("Scrivi un messaggio…")
                        .desired_width(ui.available_width() - 55.0),
                );
                let invio = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if (ui.button("Invia").clicked() || invio) && !self.chat_msg_input.is_empty() {
                    let dest = if self.chat_dest.is_empty() {
                        "ALL".to_string()
                    } else {
                        self.chat_dest.clone()
                    };
                    let _ = self
                        .net_tx
                        .send(format!("TEXT|{}|{}", dest, self.chat_msg_input));
                    self.chat_msg_input.clear();
                }
            });
        });

        if nuovo_messaggio {
            ui.ctx().request_repaint();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

impl GeoruggineClient {
    // Smista un ChatMsg nella struttura dati giusta in base al tipo.
    fn route_chat_msg(&mut self, msg: ChatMsg) {
        match msg.tipo {
            TipoChatMsg::Broadcast => self.chat_broadcast.push(msg),
            TipoChatMsg::Server => self.chat_server.push(msg),
            TipoChatMsg::Privato => {
                self.chat_private
                    .entry(msg.mittente.clone())
                    .or_default()
                    .push(msg);
            }
        }
    }
}

fn render_chat_msg(ui: &mut egui::Ui, msg: &ChatMsg) {
    let (colore_nome, prefisso) = match msg.tipo {
        TipoChatMsg::Broadcast => (egui::Color32::from_rgb(100, 180, 255), ""),
        TipoChatMsg::Privato => (egui::Color32::from_rgb(200, 120, 255), ""),
        TipoChatMsg::Server => (egui::Color32::from_rgb(255, 180, 50), ""),
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(format!("{}{}", prefisso, msg.mittente))
                .color(colore_nome)
                .strong(),
        );
        ui.label(format!(": {}", msg.contenuto));
    });
}

// ---------------------------------------------------------------------------
// Logica simulatore
// ---------------------------------------------------------------------------
impl GeoruggineClient {
    fn avvia_simulatore(&mut self) {
        let stop_flag = Arc::new(AtomicBool::new(false));
        self.sim_state.stop_flag = stop_flag.clone();
        self.sim_state.attivo = true;

        let strategia = match &self.strategia_scelta {
            StrategiaScelta::RandomWalk => Some(SimulationStrategy::RandomWalk(
                RandomWalkSource::new(45.0628, 7.6623, 0.1),
            )),
            StrategiaScelta::DaFile => match FileSource::new(&self.file_path_input) {
                Ok(s) => Some(SimulationStrategy::File(s)),
                Err(e) => {
                    self.server_logs.push(format!("[Simulatore] Errore: {e}"));
                    self.sim_state.attivo = false;
                    return;
                }
            },
            StrategiaScelta::Nessuna => None,
        };
        let Some(strategia) = strategia else { return };
        let net_tx = self.net_tx.clone();

        self.rt_handle.spawn(async move {
            let mut sim = strategia;
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                match sim.next_position() {
                    Some((lat, lon)) => {
                        let ts = chrono::Utc::now().timestamp() as u64;
                        let _ = net_tx.send(
                            ClientMessage::PositionUpdate {
                                lat,
                                lon,
                                timestamp: ts,
                            }
                            .encode(),
                        );
                    }
                    None => {
                        stop_flag.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                for _ in 0..30 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                }
            }
        });
    }

    fn ferma_simulatore(&mut self) {
        self.sim_state.stop_flag.store(true, Ordering::Relaxed);
        self.sim_state.attivo = false;
    }
}

// ---------------------------------------------------------------------------
// Parsing dei messaggi dal server : GuiEvent
// ---------------------------------------------------------------------------
fn parse_server_line(line: &str) -> GuiEvent {
    match ServerMessage::decode(line) {
        Ok(ServerMessage::RegisterOk) => GuiEvent::Log("✅ Registrazione completata".into()),
        Ok(ServerMessage::RegisterErr(r)) => GuiEvent::Log(format!("❌ Errore registrazione: {r}")),
        Ok(ServerMessage::LoginOk) => GuiEvent::LoginOk,
        Ok(ServerMessage::LoginErr(r)) => GuiEvent::Log(format!("❌ Errore login: {r}")),

        Ok(ServerMessage::Notice(testo)) => {
            if let Some(rest) = testo.strip_prefix("MSG BROADCAST da ") {
                if let Some((mittente, contenuto)) = rest.split_once(": ") {
                    return GuiEvent::Chat(ChatMsg {
                        mittente: mittente.to_string(),
                        contenuto: contenuto.to_string(),
                        tipo: TipoChatMsg::Broadcast,
                    });
                }
            }
            if let Some(rest) = testo.strip_prefix("MSG PRIVATO da ") {
                if let Some((mittente, contenuto)) = rest.split_once(": ") {
                    return GuiEvent::Chat(ChatMsg {
                        mittente: mittente.to_string(),
                        contenuto: contenuto.to_string(),
                        tipo: TipoChatMsg::Privato,
                    });
                }
            }
            if let Some(contenuto) = testo.strip_prefix("[SERVER] ") {
                return GuiEvent::Chat(ChatMsg {
                    mittente: "SERVER".to_string(),
                    contenuto: contenuto.to_string(),
                    tipo: TipoChatMsg::Server,
                });
            }
            GuiEvent::Log(format!("ℹ {testo}"))
        }

        Ok(ServerMessage::Text { from, content }) => GuiEvent::Chat(ChatMsg {
            mittente: from,
            contenuto: content,
            tipo: TipoChatMsg::Broadcast,
        }),

        Ok(ServerMessage::AnalysisResult {
            utente,
            intervallo,
            report,
        }) => {
            let int_str = match intervallo {
                common::Interval::Day => "giorno",
                common::Interval::Week => "settimana",
                common::Interval::Month => "mese",
            };
            GuiEvent::Log(format!(
                "📊 Analisi {} ({}): {:.2} km | {:.2} km/h | mov: {}s | pausa: {}s",
                utente,
                int_str,
                report.distanza_totale_km,
                report.velocita_media_kmh,
                report.tempo_in_movimento_sec,
                report.tempo_in_pausa_sec
            ))
        }

        Err(_) => GuiEvent::Log(format!("Server: {line}")),
    }
}

// ---------------------------------------------------------------------------
// Entry point : main()
// ---------------------------------------------------------------------------
fn main() -> eframe::Result<()> {
    let (net_tx, mut net_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (gui_tx, gui_rx) = mpsc::channel::<GuiEvent>();
    let (handle_tx, handle_rx) = mpsc::channel::<tokio::runtime::Handle>();

    let _ = gui_tx.send(GuiEvent::Log(
        "In attesa di connessione al server...".into(),
    ));

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = handle_tx.send(rt.handle().clone());
        rt.block_on(async {
            let stream = match TcpStream::connect("127.0.0.1:7878").await {
                Ok(s) => s,
                Err(_) => {
                    let _ = gui_tx.send(GuiEvent::Log("ERRORE: Impossibile connettersi al Server!".into()));
                    return;
                }
            };
            let _ = gui_tx.send(GuiEvent::Log("Connesso al server Georuggine!".into()));
            let (reader, mut writer) = stream.into_split();
            let mut server_lines = BufReader::new(reader).lines();
            loop {
                tokio::select! {
                    result = server_lines.next_line() => {
                        match result {
                            Ok(Some(line)) => { let _ = gui_tx.send(parse_server_line(&line)); }
                            _ => { let _ = gui_tx.send(GuiEvent::Log("Connessione chiusa dal server.".into())); break; }
                        }
                    }
                    Some(cmd) = net_rx.recv() => {
                        if !cmd.is_empty() {
                            let _ = writer.write_all(format!("{}\n", cmd).as_bytes()).await;
                        }
                    }
                }
            }
        });
    });

    let rt_handle = handle_rx.recv().expect("Handle Tokio non ricevuto");

    eframe::run_native(
        "Client Georuggine",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([920.0, 650.0]),
            ..Default::default()
        },
        Box::new(|_cc| {
            Ok(Box::new(GeoruggineClient {
                username_input: "".into(),
                password_input: "".into(),
                lat_input: "".into(),
                lon_input: "".into(),
                chat_dest: "".into(),
                chat_msg_input: "".into(),
                is_logged_in: false,
                server_logs: vec![],
                chat_broadcast: vec![],
                chat_private: HashMap::new(),
                chat_server: vec![],
                active_tab: ChatTab::Broadcast,
                net_tx,
                gui_rx,
                strategia_scelta: StrategiaScelta::Nessuna,
                file_path_input: "".into(),
                sim_state: SimulatorState::new(),
                rt_handle,
                errore_posizione: None,
            }))
        }),
    )
}
