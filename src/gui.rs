//! VibeCity GUI (Phase 3C light): Geräte-Seite über der bewiesenen Lese-/Schreib-API.
//! UI enthält keine Protokolllogik — sie ruft nur Transport + Frame-Bausteine.

use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};

const STATUS_ID: [u8; 2] = [0x7F, 0x00];

/// Parameter-IDs aus dem Read-All-Beweis (26.10., read_all_beweis_2026-10-26.md).
/// Master-Status zuletzt; `25 14`/`48 00` liegen nicht in der Klavier-Tabelle
/// (Quittung kommt als Master-Status zurück) — trotzdem sichtbar, aber deaktiviert.
const IDS: &[[u8; 2]] = &[
    [0x10, 0x10], [0x04, 0x10], [0x07, 0x01], [0x00, 0x00], [0x10, 0x11], [0x04, 0x11],
    [0x00, 0x04], [0x02, 0x00], [0x00, 0x02], [0x00, 0x03], [0x01, 0x00], [0x01, 0x05],
    [0x01, 0x03], [0x01, 0x04], [0x06, 0x02], [0x06, 0x10], [0x06, 0x11], [0x03, 0x00],
    [0x60, 0x00], [0x61, 0x00], [0x61, 0x01], [0x50, 0x02], [0x50, 0x03], [0x50, 0x04],
    [0x50, 0x08], [0x50, 0x09], [0x52, 0x00], [0x52, 0x01], [0x53, 0x00], [0x5A, 0x00],
    [0x51, 0x00], [0x51, 0x01], [0x52, 0x02], [0x58, 0x01], [0x55, 0x00], [0x3A, 0x00],
    [0x20, 0x04], [0x20, 0x03], [0x25, 0x14], [0x0B, 0x01], [0x0A, 0x00], [0x0B, 0x03],
    [0x6C, 0x00], [0x6C, 0x01], [0x6C, 0x03], [0x48, 0x00], [0x52, 0x03], [0x58, 0x00],
    [0x01, 0x01], [0x05, 0x00], [0x62, 0x00], [0x78, 0x00], STATUS_ID,
];

fn known_bad(id: [u8; 2]) -> bool {
    id == [0x25, 0x14] || id == [0x48, 0x00]
}

fn fmt_id(id: [u8; 2]) -> String {
    format!("{:02X} {:02X}", id[0], id[1])
}

#[derive(Clone)]
struct Row {
    id: [u8; 2],
    typ: String,   // "Wert" | "Flag"
    value: u16,
    edited: String,
}

enum Cmd {
    Connect(String),
    ReadAll,
    Write { id: [u8; 2], value: u16 },
}

enum Ev {
    Ports(Vec<String>),
    Status(String),
    Connected(bool),
    Progress(usize),
    Rows(Vec<(usize, Row)>),
    One(usize, Row),
    Log(String),
}

struct Worker {
    cmd_rx: Receiver<Cmd>,
    ev_tx: Sender<Ev>,
    transport: Option<Box<dyn vibecity::transport::Transport>>,
    port_name: String,
}

fn status_value(f: &[u8]) -> Option<u16> {
    if f.len() >= 15 && f.starts_with(&[0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01]) {
        let hi = if f.len() > 14 { f[14] as u16 } else { 0 };
        Some(f[13] as u16 + hi * 128)
    } else {
        None
    }
}

impl Worker {
    fn run(cmd_rx: Receiver<Cmd>, ev_tx: Sender<Ev>) {
        let mut w = Worker { cmd_rx, ev_tx, transport: None, port_name: String::new() };
        while let Ok(cmd) = w.cmd_rx.recv() {
            match cmd {
                Cmd::Connect(name) => w.connect(name),
                Cmd::ReadAll => w.read_all(),
                Cmd::Write { id, value } => w.write_one(id, value),
            }
        }
    }

    fn list_ports() -> Vec<String> {
        #[cfg(windows)]
        {
            vibecity::transport::winmm::list_ports()
                .unwrap_or_default()
                .into_iter()
                .filter(|p| !p.name.to_lowercase().contains("wavetable") && !p.name.to_lowercase().contains("soundbank"))
                .map(|p| p.name)
                .collect()
        }
        #[cfg(not(windows))]
        { vec!["mock".to_string()] }
    }

    fn connect(&mut self, name: String) {
        self.emit(Ev::Status(format!("Verbinde '{name}' …")));
        #[cfg(windows)]
        match vibecity::transport::winmm::WinMidiTransport::open(&name) {
            Ok(t) => {
                self.transport = Some(Box::new(t));
                self.port_name = name.clone();
                // Eingangswelle leerlaufen lassen.
                if let Some(t) = self.transport.as_mut() {
                    while t.recv(100).is_some() {}
                }
                self.emit(Ev::Status(format!("Verbunden: {name}")));
            }
            Err(e) => {
                self.emit(Ev::Status(format!("Fehler beim Verbinden: {e}")));
                self.emit(Ev::Connected(false));
            }
        }
        #[cfg(not(windows))]
        {
            self.transport = Some(Box::new(vibecity::transport::MockTransport::default()));
            self.port_name = name;
        }
        self.emit(Ev::Status(format!("Verbunden: {name}")));
        self.emit(Ev::Connected(true));
    }

    fn emit(&self, ev: Ev) {
        let _ = self.ev_tx.send(ev);
    }

    fn read_all(&mut self) {
        let mut t = match self.transport.take() {
            Some(t) => t,
            None => { self.emit(Ev::Status("Bitte zuerst verbinden.".into())); return; }
        };
        self.emit(Ev::Status("Lesen läuft …".into()));
        let mut rows: Vec<(usize, Row)> = Vec::new();
        let mut errs: Vec<String> = Vec::new();
        for (i, &id) in IDS.iter().enumerate() {
            let frame: Vec<u8> = vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x00, id[0], id[1], 0x00, 0xF7];
            if t.send(&frame).is_err() {
                errs.push(format!("Send-Fehler bei {}", fmt_id(id)));
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(90));
            let mut answer: Option<Vec<u8>> = None;
            let start = std::time::Instant::now();
            while start.elapsed().as_millis() < 700 {
                match t.recv(60) {
                    Some(f) => {
                        if f.len() >= 9 && (f[7] == id[0] && f[8] == id[1] || (f[7] == STATUS_ID[0] && f[8] == STATUS_ID[1])) {
                            answer = Some(f);
                            break;
                        }
                    }
                    None => break,
                }
            }
            if let Some(f) = answer {
                let typ = if f.len() > 10 && f[10] == 0x02 { "Wert" } else { "Flag" };
                let value = status_value(&f).unwrap_or(0);
                rows.push((i, Row { id, typ: typ.to_string(), value, edited: value.to_string() }));
            }
            self.emit(Ev::Progress(i + 1));
        }
        self.transport = Some(t);
        for e in errs { self.emit(Ev::Log(e)); }
        self.emit(Ev::Rows(rows));
        self.emit(Ev::Status("Lesen fertig.".into()));
    }

    fn write_one(&mut self, id: [u8; 2], value: u16) {
        let mut t = match self.transport.take() {
            Some(t) => t,
            None => { self.emit(Ev::Status("Bitte zuerst verbinden.".into())); return; }
        };
        let frame: Vec<u8> = vec![
            0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, id[0], id[1],
            0x01, 0x00, 0x01, 0x00, (value & 0x7F) as u8, ((value >> 7) & 0x7F) as u8, 0xF7,
        ];
        if let Err(e) = t.send(&frame) {
            self.transport = Some(t);
            self.emit(Ev::Log(format!("Schreibfehler {}: {e}", fmt_id(id))));
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        while t.recv(50).is_some() {}
        // Rücklese-Verifikation.
        let rf: Vec<u8> = vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x00, id[0], id[1], 0x00, 0xF7];
        let _ = t.send(&rf);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut ok = false;
        self.transport = Some(t);
        let mut t = self.transport.as_mut().unwrap();
        let start = std::time::Instant::now();
        while start.elapsed().as_millis() < 900 {
            if let Some(f) = t.recv(100) {
                if f.len() >= 15 && f[7] == id[0] && f[8] == id[1] {
                    if let Some(v) = status_value(&f) {
                        let _ = self.ev_tx.send(Ev::Log(format!("OK {} = {v}", fmt_id(id))));
                        let _ = self.ev_tx.send(Ev::One(
                            IDS.iter().position(|x| *x == id).unwrap_or(usize::MAX),
                            Row { id, typ: "Wert".into(), value: v, edited: v.to_string() },
                        ));
                        ok = true;
                        break;
                    }
                }
            } else {
                break;
            }
        }
        if !ok {
            self.emit(Ev::Log(format!("Keene Rücklese-Antwort bei {}", fmt_id(id))));
        }
    }
}

struct Gui {
    ports: Vec<String>,
    cat: usize,
    port_idx: usize,
    status: String,
    rows: Vec<Option<Row>>,
    log: Vec<String>,
    cmd_tx: Sender<Cmd>,
    ev_rx: Receiver<Ev>,
    busy_read: bool,
    connected: bool,
    progress: usize,
}

impl Gui {
    fn new() -> Self {
        let (ctx, crx) = channel();
        let (etx, erx) = channel();
        std::thread::spawn(move || Worker::run(crx, etx));
        let ports = Worker::list_ports();
        let port_idx = ports.iter().position(|n| n.contains("CASIO")).unwrap_or(0);
        // Auto-Connect: existiert beim Start ein CASIO-Port, sofort verbinden (und danach lesen).
        if let Some(p) = ports.get(port_idx).cloned() {
            if p.contains("CASIO") {
                let _ = ctx.send(Cmd::Connect(p));
            }
        }
        Self {
            ports,
            cat: 0,
            port_idx,
            status: "Nicht verbunden.".into(),
            rows: vec![None; IDS.len()],
            log: Vec::new(),
            cmd_tx: ctx,
            ev_rx: erx,
            busy_read: false,
            connected: false,
            progress: 0,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.ev_rx.try_recv() {
            match ev {
                Ev::Ports(p) => self.port_idx = p.iter().position(|n| n.contains("CASIO")).unwrap_or(0),
                Ev::Status(s) => self.status = s,
                Ev::Connected(c) => {
                    self.connected = c;
                    if c {
                        // Nach erfolgreichem Verbinden automatisch alles lesen.
                        self.busy_read = true;
                        self.progress = 0;
                        let _ = self.cmd_tx.send(Cmd::ReadAll);
                    }
                }
                Ev::Progress(n) => self.progress = n,
                Ev::Rows(rs) => {
                    for (i, r) in rs {
                        if i < self.rows.len() {
                            self.rows[i] = Some(r);
                        }
                    }
                    self.busy_read = false;
                }
                Ev::One(i, r) => {
                    if i < self.rows.len() {
                        self.rows[i] = Some(r);
                    }
                }
                Ev::Log(l) => {
                    self.log.push(l);
                    if self.log.len() > 200 {
                        self.log.remove(0);
                    }
                }
            }
        }
    }
}

/// Kategorien wie im Remote-Controller-Menü der App (Zuordnung nach byte1-Cluster, vorläufig).
const CATS: &[(&str, &[u8])] = &[
    ("Tone", &[0x01, 0x60, 0x61, 0x62]),
    ("Keyboard", &[0x07, 0x20, 0x05, 0x10]),
    ("Pedal & others", &[0x02, 0x03, 0x06, 0x5A]),
    ("Acoustic Simulator", &[0x0A, 0x0B, 0x00]),
    ("Effect", &[0x50, 0x51, 0x52, 0x53, 0x55, 0x58]),
    ("Built-in Songs", &[0x6C, 0x3A]),
    ("MIDI Recorder", &[0x78]),
    ("Metronome", &[0x25, 0x48]),
];

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        let accent = egui::Color32::from_rgb(247, 183, 51);
        {
            let v = ui.visuals_mut();
            v.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(228);
            v.widgets.inactive.bg_fill = egui::Color32::from_gray(228);
            v.widgets.hovered.weak_bg_fill = egui::Color32::from_gray(214);
            v.widgets.hovered.bg_fill = egui::Color32::from_gray(214);
            v.widgets.active.weak_bg_fill = egui::Color32::from_gray(200);
            v.widgets.active.bg_fill = egui::Color32::from_gray(200);
        }

        // Kopfzeile wie in der App: Titel links, Gerät rechts.
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Remote Controller");
                ui.separator();
                ui.strong("PX-S1100");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dot = if self.connected { "●" } else { "○" };
                    ui.colored_label(if self.connected { egui::Color32::from_rgb(40, 160, 80) } else { egui::Color32::GRAY }, format!("{dot} {}", self.status));
                });
            });
        });

        // Werkzeugleiste: Port + Aktionen.
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::new("port_cb", "Port")
                    .selected_text(self.ports.get(self.port_idx).cloned().unwrap_or_else(|| "—".into()))
                    .show_ui(ui, |ui| {
                        for (i, p) in self.ports.clone().iter().enumerate() {
                            ui.selectable_value(&mut self.port_idx, i, p);
                        }
                    });
                if ui.button("Neu laden").clicked() {
                    self.ports = Worker::list_ports();
                }
                if ui.button("Verbinden").clicked() {
                    if let Some(p) = self.ports.get(self.port_idx).cloned() {
                        let _ = self.cmd_tx.send(Cmd::Connect(p));
                    }
                }
                ui.add_enabled_ui(self.connected && !self.busy_read, |ui| {
                    if ui.button("Alle lesen").clicked() {
                        self.busy_read = true;
                        self.progress = 0;
                        let _ = self.cmd_tx.send(Cmd::ReadAll);
                    }
                });
                if self.busy_read {
                    let frac = self.progress as f32 / IDS.len() as f32;
                    ui.add(egui::ProgressBar::new(frac).text(format!("Lese… {}/{}", self.progress, IDS.len())).animate(true));
                }
            });
        });

        // Kategorie-Kacheln wie im App-Screenshot (2×4 + „Alle").
        egui::Panel::top("tiles").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let tile = |ui: &mut egui::Ui, sel: &mut usize, idx: usize, text: &str, accent: egui::Color32| {
                    let active = *sel == idx;
                    let frame = egui::Frame::new()
                        .fill(if active { accent.linear_multiply(0.25) } else { egui::Color32::from_gray(235) })
                        .stroke(egui::Stroke::new(if active { 2.0 } else { 1.0 }, accent))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 8));
                    if frame.show(ui, |ui| ui.selectable_label(active, text)).inner.clicked() {
                        *sel = idx;
                    }
                };
                tile(ui, &mut self.cat, 0, "Alle", accent);
                for (i, (name, _)) in CATS.iter().enumerate() {
                    tile(ui, &mut self.cat, i + 1, name, accent);
                }
            });
        });

        egui::Panel::bottom("log").resizable(true).default_size(90.0).show(ui, |ui| {
            ui.strong("Protokoll");
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for l in &self.log {
                    ui.label(l);
                }
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("params").num_columns(5).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new("ID").strong());
                    ui.label(egui::RichText::new("Typ").strong());
                    ui.label(egui::RichText::new("Wert").strong());
                    ui.label("");
                    ui.label("");
                    for (i, meta) in IDS.iter().enumerate() {
                        // Filter: Kategorie 0 = alle.
                        if self.cat > 0 {
                            let (_, groups) = CATS[self.cat - 1];
                            if !groups.contains(&meta[0]) {
                                continue;
                            }
                        }
                        ui.label(fmt_id(*meta));
                        match &mut self.rows[i] {
                            Some(r) => {
                                ui.label(r.typ.clone());
                                let resp = ui.add_sized(
                                    [70.0, 20.0],
                                    egui::TextEdit::singleline(&mut self.rows[i].as_mut().unwrap().edited)
                                        .desired_width(60.0),
                                );
                                let write = ui.add_enabled_ui(!known_bad(*meta), |ui| {
                                    ui.button("Schreiben")
                                });
                                if write.inner.clicked()
                                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                                {
                                    if let Ok(v) = self.rows[i].as_ref().unwrap().edited.parse::<u16>() {
                                        let _ = self.cmd_tx.send(Cmd::Write { id: *meta, value: v });
                                    } else {
                                        self.log.push("Wert ist keine Zahl.".into());
                                    }
                                }
                            }
                            None => {
                                ui.label("–");
                                ui.label("–");
                                ui.label("");
                            }
                        }
                        ui.label(if known_bad(*meta) { "(nicht in Tabelle)" } else { "" });
                        ui.end_row();
                    }
                });
            });
        });
        let delay = if self.busy_read { 200 } else { 80 };
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(delay));
    }
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 640.0]).with_title("VibeCity"),
        ..Default::default()
    };
    eframe::run_native("VibeCity", opts, Box::new(|cc| {
        let _ = cc;
        Ok(Box::new(Gui::new()))
    }))
}
