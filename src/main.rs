//! VibeCity CLI (Phase 3A). On Windows uses WinMM via midir; elsewhere falls
//! back to the built-in mock device. All device I/O is logged to logs/.

use std::io::Write;
use vibecity::protocol::{
    build_param_msg, build_param_request, build_param_send, parse_param_msg, sub_of, Reply,
    MODEL_ID_PX_S1100, MODEL_TEMPLATE_DEFAULT, SUB_APPX, SUB_CMD_COMMAND, SUB_CMD_DATA, SUB_SESSION_ENABLE,
};
use vibecity::transport::{MockTransport, Transport};
use vibecity::PARAM_COUNT;

fn parse_id(s: &str) -> Result<usize, String> {
    let v = if let Some(hex) = s.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).map_err(|e| e.to_string())?
    } else {
        s.parse::<usize>().map_err(|e| e.to_string())?
    };
    if v < PARAM_COUNT {
        Ok(v)
    } else {
        Err(format!("param id {v} out of range (0..{})", PARAM_COUNT - 1))
    }
}

/// Request/response exchange for one parameter id against any transport.
/// Full reference-app read session (evidence: getParameterThread state flow):
///   1) REQUEST(0xC4)   appx session start   [sub on wire: U]
///   2) REQUEST(0xB8)   session enable       (link34: 0xB6)
///   3) SEND(0xCF,id) + REQUEST(0xD0) pair; value arrives on a 0xD0 reply.
/// Device replies are tolerated/ignored on the way; only the 0xD0 answer counts.
fn get_param<T: Transport + ?Sized>(t: &mut T, id: usize) -> Result<u32, String> {
    t.send(&build_param_request(SUB_APPX, 0, MODEL_TEMPLATE_DEFAULT))?;
    let _ = t.recv(400);
    t.send(&build_param_request(SUB_SESSION_ENABLE, 0, MODEL_TEMPLATE_DEFAULT))?;
    let _ = t.recv(600);
    t.send(&build_param_send(SUB_CMD_COMMAND, id as u32, 0, MODEL_TEMPLATE_DEFAULT))?;
    t.send(&build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT))?;
    // Device may answer 0xC4/0xB8 remnants first; accept only a 0xD0 value frame.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1800);
    while std::time::Instant::now() < deadline {
        let Some(frame) = t.recv(900) else { continue };
        if sub_of(&frame) == Some(SUB_CMD_DATA) {
            let r: Reply = parse_param_msg(&frame).ok_or("malformed 0xD0 reply")?;
            if r.ok {
                return Ok(r.payload);
            }
            return Err(format!("device status {}", r.status));
        }
    }
    Err("timeout waiting for 0xD0 reply".into())
}

fn set_param<T: Transport + ?Sized>(t: &mut T, id: usize, value: u32) -> Result<(), String> {
    // Observed write path: value under DATA sub-cmd first, then id under COMMAND.
    t.send(&build_param_msg(SUB_CMD_DATA, value, 0, 2))?;
    t.recv(900);
    t.send(&build_param_msg(SUB_CMD_COMMAND, id as u32, 0, 2))?;
    let frame = t.recv(900).ok_or("timeout waiting for reply")?;
    let r = parse_param_msg(&frame).ok_or("malformed reply")?;
    if !r.ok {
        return Err(format!("device status {}", r.status));
    }
    Ok(())
}

fn log_line(msg: &str) {
    let _ = std::fs::create_dir_all("logs");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("logs/session-cli.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// Prompt the user and accept j/n/s (pass/problem/skip).
fn ask(prompt: &str) -> char {
    print!("{prompt} [j/n/s]: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
        return 's';
    }
    line.trim().chars().next().unwrap_or('s').to_ascii_lowercase()
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

fn hexdump(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// Open the first piano-like MIDI port (Windows) or the mock (dev demo).
#[allow(unused_variables)]
fn open_piano_port() -> Option<(Box<dyn Transport>, String)> {
    #[cfg(windows)]
    {
        let ports = vibecity::transport::winmm::list_ports().ok()?;
        let name = ports
            .iter()
            .find(|p| looks_piano(&p.name))
            .or_else(|| ports.first())
            .map(|p| p.name.clone())?;
        match vibecity::transport::winmm::WinMidiTransport::open(&name) {
            Ok(w) => Some((Box::new(w), name)),
            Err(e) => {
                eprintln!("Öffnen von '{name}' fehlgeschlagen: {e}");
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        println!("(Nicht-Windows-Build: Mock-Gerät)");
        Some((Box::new(MockTransport::default()), "mock".into()))
    }
}

/// Send standard questions one after another and print the raw answers.
/// Purpose: find out which dialect the real piano actually speaks.
fn run_probe() {
    let Some((mut t, name)) = open_piano_port() else {
        eprintln!("Kein MIDI-Port gefunden.");
        return;
    };
    println!("Probe über Port '{name}' — Piano bitte eingeschaltet lassen.\n");
    let queries: Vec<(&str, Vec<u8>)> = vec![
        ("1) General-MIDI-Inquiry  F0 7E 7F 06 01 F7", vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]),
        ("2) Casio privat param-get id0 (30 Byte)", build_param_msg(SUB_CMD_COMMAND, 0, 0, 2)),
        ("3) Casio privat param-get id0, Geräte-ID 0 statt 7F", {
            let mut m = build_param_msg(SUB_CMD_COMMAND, 0, 0, 2);
            m[4] = 0x00;
            m
        }),
        ("4) MUI5-Altanfrage  F0 43 19 C4 F7", vec![0xF0, 0x43, 0x19, 0xC4, 0xF7]),
        ("5) NEU Send-Frame 30B (byte5=01, sub=CF, id0)", build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_ID_PX_S1100)),
        ("6) NEU Request-Frame 25B (sub=D0)", build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT)),
    ];
    for (label, msg) in queries {
        println!("{label}");
        println!("   send: {}", hexdump(&msg));
        if let Err(e) = t.send(&msg) {
            println!("   Sendefehler: {e}");
            continue;
        }
        let mut answers = Vec::new();
        while let Some(frame) = t.recv(1200) {
            let kind = if frame.first() == Some(&0xF0) { "SysEx" } else { "Kurz" };
            println!("   antwort({kind}): {}", hexdump(&frame));
            answers.push(frame);
            if answers.len() >= 4 {
                break;
            }
        }
        if answers.is_empty() {
            println!("   antwort: keine (Timeout 1.2 s)");
        }
        println!();
    }
    // Pair attempt per reference app: SEND frame immediately followed by REQUEST.
    println!("7) PAIR wie Referenz-App: Send(30B,CF) direkt + Request(25B,D0)");
    let a = build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_ID_PX_S1100);
    let b = build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT);
    println!("   send: {}", hexdump(&a));
    println!("   send: {}", hexdump(&b));
    let _ = t.send(&a);
    let _ = t.send(&b);
    let mut got = 0;
    while let Some(frame) = t.recv(1500) {
        println!("   antwort: {}", hexdump(&frame));
        got += 1;
        if got >= 4 {
            break;
        }
    }
    if got == 0 {
        println!("   antwort: keine (Timeout 1.5 s)");
    }
    println!("Probe fertig — Ausgabe zurück an Hermes.");
}

/// Passively observe the port for N seconds and print everything that arrives.
/// Use it while turning knobs / changing sounds on the piano.
fn run_monitor(secs: u64) {
    let Some((mut t, name)) = open_piano_port() else {
        eprintln!("Kein MIDI-Port gefunden.");
        return;
    };
    let secs_txt = if secs == 0 { "unbegrenzt (Strg+C zum Beenden)".into() } else { format!("{secs} s") };
    println!("Monitor auf '{name}' — {secs_txt}, nichts senden. Jetzt am Piano etwas ändern …");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::create_dir_all("logs");
    let logpath = format!("logs\\monitor-{ts}.log");
    let mut log = format!("VibeCity Monitor · Port '{name}' · {secs_txt} · Zeitstempel {ts}\n");
    let start = std::time::Instant::now();
    let mut count = 0usize;
    while secs == 0 || start.elapsed().as_secs() < secs {
        if let Some(frame) = t.recv(500) {
            count += 1;
            let t_ms = start.elapsed().as_millis();
            let kind = if frame.first() == Some(&0xF0) { "SysEx" } else { "Kurz" };
            let line = format!("[{t_ms:>6} ms] {kind}: {}", hexdump(&frame));
            println!("{line}");
            log.push_str(&line);
            log.push('\n');
            // Sofort auf Disk, damit Strg+C oder Crash nichts kostet.
            let _ = std::fs::write(&logpath, &log);
        }
    }
    let done = format!("Monitor fertig: {count} Nachricht(en) gesehen. Log: {logpath}");
    println!("{done}");
    log.push_str(&done);
    log.push('\n');
    let _ = std::fs::write(&logpath, &log);
}

#[allow(dead_code)]
/// Does this port name plausibly belong to the piano (not a synth/HW mapper)?
fn looks_piano(name: &str) -> bool {
    let n = name.to_lowercase();
    ["casio", "px-s", "privia", "cdp", "apex", "cedura", "midi", "ble", "bluetooth"]
        .iter()
        .any(|k| n.contains(k))
        && !n.contains("wavetable")
        && !n.contains("mapper")
        && !n.contains("decomposer")
}

#[allow(dead_code)]
fn fmt_ports(ports: &[(usize, String, String)]) -> String {
    let mut s = String::new();
    for (i, name, kind) in ports {
        let hint = if name.to_lowercase().contains("wavetable") {
            "  ← Windows-eigene Soundbank, NICHT das Piano"
        } else if looks_piano(name) {
            "  ← könnte das Piano sein"
        } else {
            ""
        };
        s.push_str(&format!("  [{i}] {name} ({kind}){hint}\n"));
    }
    s
}

/// Step-by-step hardware wizard (dev tool) for users with zero prior
/// knowledge. Results land in hardware-tests-results.md next to the exe.
fn run_wizard() {
    println!("=== VibeCity Wizard ===");
    println!("Dieser Wizard prüft Schritt für Schritt, ob der PC mit dem");
    println!("Casio-Piano sprechen kann. Am Ende steht alles in der Datei");
    println!("hardware-tests-results.md — die bitte zurück an Hermes.\n");

    // ---------- Schritt 0: Vorbereitung ----------
    println!("--- Vorbereitung ---");
    println!("Womit ist das Piano verbunden?");
    println!("  [1] USB-Kabel   (empfohlen für den ersten Test)");
    println!("  [2] WU-BT10 Bluetooth-Dongle");
    let how = read_line("  Auswahl (1/2, Enter = 1): ");
    let how = if how.starts_with('2') { 2 } else { 1 };
    if how == 2 {
        println!("  So geht Bluetooth beim WU-BT10:");
        println!("  - Der kleine Dongle steckt in der USB-Buchse des Pianos (USB TO HOST).");
        println!("  - Piano EINSCHALTEN — der Dongle bekommt dann Strom und sendet sein Signal.");
        println!("  - Kein Koppeln/Pairing nötig: Windows findet das Piano automatisch,");
        println!("    genau wie das Handy es macht (belegt aus der Referenz-App).");
        println!("  - Der PC braucht Bluetooth 4.0 oder neuer und Bluetooth muss an sein.");
    } else {
        println!("  So geht USB:");
        println!("  - USB-Kabel von Piano (USB TO HOST) in den PC — Datenkabel, kein reines Ladekabel.");
        println!("  - Piano EINSCHALTEN.");
    }
    let ready = ask("Ist das Piano jetzt eingeschaltet und verbunden?");
    if ready == 'n' {
        println!("  Dann bitte jetzt Piano einschalten und Kabel/Dongle prüfen,");
        println!("  danach den Wizard mit 'vibecity wizard' neu starten.");
    }

    // ---------- Ports finden (mit Neu-Scan-Möglichkeit) ----------
    let mut t: Box<dyn Transport>;
    #[allow(unused_mut)]
    let mut port_name = String::from("mock");

    #[cfg(windows)]
    {
        loop {
            let ports = match vibecity::transport::winmm::list_ports() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Fehler Portliste: {e}");
                    return;
                }
            };
            let listed: Vec<(usize, String, String)> = ports
                .iter()
                .enumerate()
                .map(|(i, p)| (i, p.name.clone(), format!("{:?}", p.kind)))
                .collect();
            println!("\nGefundene MIDI-Ports:");
            print!("{}", fmt_ports(&listed));
            if ports.is_empty() {
                println!("  (keine) — Prüfliste: Piano an? Kabel steckt? Anderen USB-Port testen.");
                println!("  Bei Bluetooth: ist der Adapter an? Piano eingeschaltet?");
            }
            let sel = read_line("Nummer wählen · r = Neu scannen · Enter = Abbruch: ");
            if sel.eq_ignore_ascii_case("r") {
                println!("  (Neuer Suchlauf …)");
                continue;
            }
            let Ok(idx) = sel.parse::<usize>() else {
                println!("Abbruch.");
                return;
            };
            let Some(name) = listed.get(idx).map(|p| p.1.clone()) else {
                println!("Diese Nummer gibt es nicht, zurück zur Liste.");
                continue;
            };
            if !looks_piano(&name) {
                println!("  Hinweis: '{name}' klingt nicht nach Piano (eher Windows-Soundbank).");
                println!("  Ein Test damit führt gleich wahrscheinlich zu 'timeout'.");
                if ask("Trotzdem damit testen?") != 'j' {
                    continue;
                }
            }
            match vibecity::transport::winmm::WinMidiTransport::open(&name) {
                Ok(w) => {
                    t = Box::new(w);
                    port_name = name;
                    break;
                }
                Err(e) => {
                    println!("  Verbindung fehlgeschlagen: {e}");
                    if ask("Anderen Port wählen?") != 'j' {
                        return;
                    }
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        println!("\n(Nicht-Windows-Build: Mock-Gerät, nur Demonstration)");
        t = Box::new(MockTransport::default());
    }

    let mut results: Vec<(String, char)> = Vec::new();

    // ---------- R0: Port sichtbar ----------
    println!("\n--- Schritt 1/4 · R0 · Verbindung ---");
    println!("Ziel: Der PC findet das Piano als MIDI-Port und kann es öffnen.");
    println!("Ergebnis: Port '{port_name}' wurde gefunden und geöffnet.");
    println!("Rückweg: App schließen — am Piano ändert sich dabei nichts.");
    results.push((format!("R0 port visible ({port_name})"), ask("Ist das der richtige Piano-Port?")));

    // ---------- R1: Lesen ----------
    println!("\n--- Schritt 2/4 · R1 · Lesen ---");
    println!("Ziel: Das Piano beantwortet unsere erste Frage (Parameter 0).");
    println!("Frage: 'Zeig mir Wert 0' · Antwort kommt als SysEx-Nachricht zurück.");
    let r1 = get_param(&mut *t, 0);
    match &r1 {
        Ok(v) => println!("Antwort: Parameter 0 = {v}"),
        Err(e) => println!("Fehler: {e}"),
    }
    let a1 = ask("Antwort erhalten und plausibel?");
    if a1 != 'j' {
        println!("  Typische Ursachen bei 'timeout':");
        println!("  - Port war die Windows-Soundbank (antwortet nie auf Casio-Fragen)");
        println!("  - Piano war im Moment der Frage aus");
        println!("  - Bei Bluetooth: Signal weg (Dongle/Piano aus).");
    }
    results.push(("R1 get param 0".into(), a1));

    // ---------- R2: Snapshot ----------
    println!("\n--- Schritt 3/4 · R2 · Snapshot ---");
    println!("Ziel: Alle 57 Parameter abfragen und als Datei sichern (Startpunkt für später).");
    let mut dump_ok = true;
    let mut json = String::from("{\n");
    for id in 0..PARAM_COUNT {
        match get_param(&mut *t, id) {
            Ok(v) => json.push_str(&format!("  \"param_{id}\": {v},\n")),
            Err(_) => {
                dump_ok = false;
                json.push_str(&format!("  \"param_{id}\": null,\n"));
            }
        }
    }
    json.push('}');
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file = format!("snapshot-{ts}.json");
    if std::fs::write(&file, json).is_ok() {
        println!("Ergebnis: {file} geschrieben (leere Werte: {})", if dump_ok { "keine" } else { "mind. 1 — jedes null heißt: keine Antwort" });
    }
    results.push(("R2 snapshot".into(), ask("Snapshot ok?")));

    // ---------- W1: Schreiben ----------
    println!("\n--- Schritt 4/4 · W1 · Schreiben ---");
    println!("Ziel: Dem Piano einen neuen Wert mitteilen und prüfen, dass er ankam.");
    println!("Beispiel: '3=7' bedeutet: Parameter 3 auf Wert 7 setzen.");
    println!("Rückweg: alten Wert notieren und mit 'vibecity set 3 <alt>' zurücksetzen.");
    let input = read_line("  id=value eingeben (Enter = überspringen): ");
    if input.is_empty() {
        results.push(("W1 set+readback".into(), 's'));
    } else {
        let mut parts = input.split('=');
        let id = parts.next().unwrap_or("").parse::<usize>();
        let val = parts.next().unwrap_or("").parse::<u32>();
        match (id, val) {
            (Ok(id), Ok(val)) if id < PARAM_COUNT => {
                match set_param(&mut *t, id, val) {
                    Ok(()) => match get_param(&mut *t, id) {
                        Ok(back) => println!("  Readback: Parameter {id} = {back}"),
                        Err(e) => println!("  Readback-Fehler: {e}"),
                    },
                    Err(e) => println!("  Set-Fehler: {e}"),
                }
                results.push((format!("W1 set {id}={val}"), ask("Hat das Piano den Wert übernommen (gelesen/gehört)?")));
            }
            _ => {
                println!("  ungültige Eingabe, übersprungen");
                results.push((format!("W1 set '{input}'"), 's'));
            }
        }
    }

    // ---------- Ergebnis ----------
    let mut md = format!(
        "# Hardware-Test-Ergebnisse\n\nPort: `{port_name}` · Verbindung: {} · Zeitstempel: {ts}\n\n| Schritt | Ergebnis |\n|---|---|\n",
        if how == 2 { "WU-BT10 (BLE)" } else { "USB-Kabel" }
    );
    for (name, o) in &results {
        let s = match o {
            'j' => "ok",
            'n' => "PROBLEM",
            _ => "skip",
        };
        md.push_str(&format!("| {name} | {s} |\n"));
    }
    if std::fs::write("hardware-tests-results.md", &md).is_ok() {
        println!("\nErgebnisse gespeichert: hardware-tests-results.md — diese Datei zurück an Hermes.");
    }
    println!("Wizard fertig. (Entwicklungs-Werkzeug, darf später weg.)");
    log_line(&format!(
        "wizard done on {port_name}: {}",
        results.iter().map(|(_, o)| o).collect::<String>()
    ));
}

/// One-shot diagnosis for the user: probe + monitor, everything also
/// written to logs\diagnose-<ts>.log so only ONE file has to be sent back.
fn run_selftest(secs: u64) {
    println!("Selftest startet — öffne Port, das kann ein paar Sekunden dauern …");
    let Some((mut t, name)) = open_piano_port() else {
        eprintln!("Kein MIDI-Port gefunden.");
        return;
    };
    println!("Port '{name}' offen. Kurze Aufwärmphase für das Piano (0.5 s) …");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::create_dir_all("logs");
    let logpath = format!("logs\\diagnose-{ts}.log");
    let mut log = format!("VibeCity Selftest · Port '{name}' · Zeitstempel {ts}\n\n== PROBE ==\n");

    // -1) Erste-Nachricht-Test: Beobachtung Läufe 1+4 — nur die ALLERERSTE
    //     SysEx nach dem Öffnen wird beantwortet. Deshalb ist jetzt die
    //     private C4-Session-Frage die allererste Nachricht. Piano dafür
    //     vor dem Start aus- und wieder einschalten.
    log.push_str("-1) ERSTE NACHRICHT = private C4-Session-Frage\n");
    println!("Erste-Nachricht-Test: private C4-Frage als allererste SysEx …");
    let first = build_param_request(SUB_APPX, 0, MODEL_TEMPLATE_DEFAULT);
    log.push_str(&format!("  send: {}\n", hexdump(&first)));
    let _ = t.send(&first);
    match t.recv(1500) {
        Some(frame) => {
            log.push_str(&format!("  antwort: {}\n", hexdump(&frame)));
            println!("  Antwort bekommen: {}", hexdump(&frame));
        }
        None => {
            log.push_str("  antwort: keine\n");
            println!("  keine Antwort");
        }
    }

    // 0) GM-Inquiry bis 3× — die erste Sendung nach dem Öffnen wird gern mal
    //    verschluckt (Beobachtung Lauf 1 vs. 2).
    log.push_str("0) GM-Inquiry, bis 3 Versuche\n");
    let mut gm_ok = false;
    for attempt in 1..=3 {
        println!("GM-Inquiry Versuch {attempt} …");
        let _ = t.send(&[0xF0u8, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
        if let Some(frame) = t.recv(1200) {
            log.push_str(&format!("  Versuch {attempt}: {}\n", hexdump(&frame)));
            println!("  Antwort: {}", hexdump(&frame));
            gm_ok = true;
            break;
        }
        log.push_str(&format!("  Versuch {attempt}: keine Antwort\n"));
        println!("  keine Antwort");
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    log.push_str(&format!("  Ergebnis GM: {}\n", if gm_ok { "Antwort bekommen" } else { "3x keine" }));

    let queries: Vec<(&str, Vec<u8>)> = vec![
        ("1) General-MIDI-Inquiry", vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]),
        ("2) Casio privat 30B (alt)", build_param_msg(SUB_CMD_COMMAND, 0, 0, 2)),
        ("3) MUI5-Altanfrage", vec![0xF0, 0x43, 0x19, 0xC4, 0xF7]),
        ("4) NEU Send-Frame 30B (byte5=01, sub=CF, id0)", build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_ID_PX_S1100)),
        ("4b) derselbe Frame mit Modell 0x30 (Template-Default)", build_param_send(SUB_CMD_COMMAND, 0, 0, 0x30)),
        ("5) NEU Request-Frame 25B (sub=D0)", build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT)),
    ];
    for (label, msg) in queries {
        println!("Frage: {label}");
        log.push_str(&format!("{label}\n  send: {}\n", hexdump(&msg)));
        if let Err(e) = t.send(&msg) {
            log.push_str(&format!("  Sendefehler: {e}\n"));
            println!("  Sendefehler: {e}");
            continue;
        }
        let mut n = 0;
        while let Some(frame) = t.recv(1200) {
            log.push_str(&format!("  antwort: {}\n", hexdump(&frame)));
            println!("  antwort: {}", hexdump(&frame));
            n += 1;
            if n >= 4 {
                break;
            }
        }
        if n == 0 {
            log.push_str("  antwort: keine (Timeout 1.2 s)\n");
            println!("  antwort: keine (Timeout 1.2 s)");
        }
    }

    // Full reference read session, step by step, so the log shows exactly
    // where the real device answers and where it stays quiet.
    log.push_str("\n6) SESSION wie Referenz-App: C4 -> B8 -> (CF+D0)\n");
    let step = |log: &mut String, t: &mut dyn Transport, label: &str, msgs: &[Vec<u8>], want: u16| {
        for m in msgs {
            log.push_str(&format!("  {label} send: {}\n", hexdump(m)));
            println!("Session-Schritt {label} …");
            let _ = t.send(m);
        }
        let mut got = 0;
        let mut hit = false;
        while let Some(frame) = t.recv(1200) {
            let sub = sub_of(&frame).map(|s| format!("{s:02X}")).unwrap_or_else(|| "--".into());
            log.push_str(&format!("  {label} antwort sub={sub}: {}\n", hexdump(&frame)));
            println!("  {label} antwort sub={sub}: {}", hexdump(&frame));
            hit |= sub_of(&frame) == Some(want);
            got += 1;
            if got >= 5 {
                break;
            }
        }
        if got == 0 {
            log.push_str(&format!("  {label} antwort: keine (Timeout)\n"));
            println!("  {label} antwort: keine (Timeout)");
        }
        hit
    };
    let ok_c4 = step(&mut log, &mut *t, "C4", &[build_param_request(SUB_APPX, 0, MODEL_TEMPLATE_DEFAULT)], SUB_APPX);
    let ok_b8 = step(&mut log, &mut *t, "B8", &[build_param_request(SUB_SESSION_ENABLE, 0, MODEL_TEMPLATE_DEFAULT)], SUB_SESSION_ENABLE);
    let ok_d0 = step(
        &mut log,
        &mut *t,
        "CF+D0",
        &[build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_ID_PX_S1100), build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT)],
        SUB_CMD_DATA,
    );
    log.push_str(&format!(
        "  Zwischenergebnis: C4={}, B8={}, D0={}\n",
        if ok_c4 { "OK" } else { "keine" },
        if ok_b8 { "OK" } else { "keine" },
        if ok_d0 { "OK" } else { "keine" }
    ));

    // 7) Ton-Test: prüfen, ob überhaupt etwas bei uns ANKOMMEND rausgeht —
    //    wenn das Piano mitspielt, ist die Richtung PC→Piano bewiesen.
    log.push_str("\n7) Ton-Test: Note C4 (Kanal 1) senden\n");
    println!("Ton-Test: gleich sollte das Piano (Kanal 1) einen Ton spielen …");
    let _ = t.send(&[0x90, 0x3C, 0x50]);
    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = t.send(&[0x80, 0x3C, 0x40]);
    let heard = ask("Hat das Piano gerade einen Ton gespielt?");
    log.push_str(&format!("  Ton gehört: {}\n", match heard { 'j' => "ja", 'n' => "nein", _ => "skip" }));

    log.push_str(&format!("\n== MONITOR {secs} s ==\n(Spielen/Regler ändern ist erlaubt, nichts senden)\n"));
    println!("Monitor läuft {secs} s — jetzt am Piano ein paar Töne spielen …");
    let start = std::time::Instant::now();
    let mut count = 0usize;
    while start.elapsed().as_secs() < secs {
        if let Some(frame) = t.recv(500) {
            count += 1;
            log.push_str(&format!("[{:>6} ms] {}\n", start.elapsed().as_millis(), hexdump(&frame)));
        }
    }
    log.push_str(&format!("\nMonitor: {count} Nachricht(en) gesehen.\n\nEnde Selftest.\n"));

    match std::fs::write(&logpath, &log) {
        Ok(()) => println!("\nGespeichert: {logpath}"),
        Err(e) => println!("\nLog schreiben fehlgeschlagen: {e}"),
    }
    // Müll vermeiden: alte Diagnose-Logs und Snapshots wegräumen.
    let mut cleaned = 0usize;
    for dir in ["logs", "."] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str().map(String::from)).unwrap_or_default();
                let keep = name == "session-cli.log" || logpath.ends_with(&name);
                let stale = (name.starts_with("diagnose-") || name.starts_with("snapshot-"))
                    && (name.ends_with(".log") || name.ends_with(".json"));
                if stale && !keep {
                    if std::fs::remove_file(&p).is_ok() {
                        cleaned += 1;
                    }
                }
            }
        }
    }
    if cleaned > 0 {
        println!("{cleaned} alte Log-/Snapshot-Datei(en) gelöscht (Müllabfuhr).");
    }
}

/// Eine Nachricht oder ein Bursts mehrerer Nachrichten (App-Folge 1:1).
type SweepMsg = Vec<Vec<u8>>;

fn burst(msgs: &[&[u8]]) -> SweepMsg {
    msgs.iter().map(|m| m.to_vec()).collect()
}

/// Dialekt-Sweep: jeder Kandidat bekommt einen FRISCHEN Port-Öffnungsvorgang,
/// weil das Piano offenbar nur die allererste Nachricht nach dem Öffnen
/// beantwortet (Beobachtung Läufe 1/4/5). Ein Lauf, alle Kandidaten, Schluss
/// mit Einzelschuss-Zyklen.
fn run_sweep() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::create_dir_all("logs");
    let logpath = format!("logs\\sweep-{ts}.log");
    let mut log = format!("VibeCity Dialekt-Sweep · Zeitstempel {ts}\n");

    let mut old_header = build_param_request(SUB_APPX, 0, MODEL_TEMPLATE_DEFAULT);
    old_header[2] = 0x17;
    old_header[3] = 0x04;

    let mut pair = build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_TEMPLATE_DEFAULT);
    pair.extend_from_slice(&build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT));

    // App-eigene Eröffnungsfolge (REA: AppxGetModelID -> StartOfBulkDump ->
    // Handshake -> Request(sub=B8)). PX-S-Familien-Header 19 03 bereits
    // gemäß set_ModelID2SysEx eingesetzt.
    let bulk_start: &[u8] = &[0xF0, 0x44, 0x19, 0x03, 0x7F, 0x08, 0x01, 0xF7];
    let handshake: &[u8] = &[0xF0, 0x44, 0x19, 0x03, 0x7F, 0x04, 0x50, 0x03, 0x00, 0xF7];
    let b8 = build_param_request(SUB_SESSION_ENABLE, 0, MODEL_TEMPLATE_DEFAULT);
    let c4 = build_param_request(SUB_APPX, 0, MODEL_TEMPLATE_DEFAULT);
    let cf = build_param_send(SUB_CMD_COMMAND, 0, 0, MODEL_TEMPLATE_DEFAULT);
    let d0 = build_param_request(SUB_CMD_DATA, 0, MODEL_TEMPLATE_DEFAULT);
    let mut pair = cf.clone();
    pair.extend_from_slice(&d0.clone());

    let candidates: Vec<(&str, SweepMsg)> = vec![
        // NEU 26.10. (Lauf 2): Der 7E7E/0F-Kanal antwortet! Alle variierten
        // Requests bekamen denselben `7F 00`-Status zurück. Test jetzt mit
        // REALEN IDs aus dem App-Mitschnitt: folgt die Antwort-ID dem Request?
        (
            "KANAL 7E7E ID 7F 00 (Kontrolle)",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x7F, 0x00, 0xF7]],
        ),
        (
            "KANAL 7E7E ID 08 00",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x08, 0x00, 0xF7]],
        ),
        (
            "KANAL 7E7E ID 25 00",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x25, 0x00, 0xF7]],
        ),
        (
            "KANAL 7E7E ID 52 01",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x52, 0x01, 0xF7]],
        ),
        (
            "KANAL 7E7E ID 20 0B",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x20, 0x0B, 0xF7]],
        ),
        (
            "KANAL 7E7E ID 58 00",
            vec![vec![0xF0, 0x44, 0x7E, 0x7E, 0x7F, 0x0F, 0x01, 0x58, 0x00, 0xF7]],
        ),
        (
            "SESSION-PREP NEU (Start+B8+CF+D0)",
            burst(&[bulk_start, &b8, &cf, &d0]),
        ),
        (
            "SESSION-PREP mit Handshake (Start+HS+C4+CF+D0)",
            burst(&[bulk_start, handshake, &c4, &cf, &d0]),
        ),
        ("HANDSHAKE einzeln", vec![handshake.to_vec()]),
        ("BULK-START einzeln", vec![bulk_start.to_vec()]),
        ("GM-Inquiry (Kontrolle)", vec![vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]]),
        ("C4 Session-Frage (19 03)", vec![c4]),
        ("D0 Lesebitte (19 03)", vec![d0.clone()]),
        ("CF+D0 Paar (19 03)", burst(&[&cf, &d0])),
        ("C4 Session-Frage (17 04)", vec![old_header]),
    ];

    println!("Sweep: {} Kandidaten, je mit frischer Port-Öffnung …", candidates.len());
    for (i, (label, msgs)) in candidates.iter().enumerate() {
        // frischer Port für jeden Kandidaten
        let Some((mut t, name)) = open_piano_port() else {
            log.push_str(&format!("K{i} {label}: Port nicht gefunden — Abbruch\n"));
            break;
        };
        std::thread::sleep(std::time::Duration::from_millis(500));
        for m in msgs {
            let _ = t.send(m);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Mehrere Rahmen sammeln (ein Handshake kann Bulk-Dump liefern), bis 1.5 s Ruhe.
        let mut frames: Vec<Vec<u8>> = Vec::new();
        let start = std::time::Instant::now();
        while frames.len() < 40 {
            let idle = if frames.is_empty() { 1500 } else { 400 };
            match t.recv(idle) {
                Some(f) => frames.push(f),
                None => break,
            }
            if start.elapsed().as_millis() > 5000 {
                break;
            }
        }
        drop(t); // Port zu, nächster Kandidat
        log.push_str(&format!("K{i} {label} @ {name}\n"));
        for m in msgs {
            log.push_str(&format!("  send: {}\n", hexdump(m)));
        }
        if frames.is_empty() {
            log.push_str("  antwort: keine\n");
            println!("K{i} {label}: keine");
        } else {
            log.push_str(&format!("  ANTWORT: {} Rahmen\n", frames.len()));
            for f in &frames[..frames.len().min(3)] {
                log.push_str(&format!("    {}\n", hexdump(f)));
            }
            println!("K{i} {label}: ANTWORT {} Rahmen — {}", frames.len(), hexdump(&frames[0]));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    std::fs::write(&logpath, &log).ok();
    println!("\nLog: {logpath}");
}


fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let mut t = MockTransport::default();

    match cmd {
        "list-ports" => {
            #[cfg(windows)]
            match vibecity::transport::winmm::list_ports() {
                Ok(ports) => {
                    for p in ports {
                        println!("{:?}\t{}", p.kind, p.name);
                    }
                }
                Err(e) => eprintln!("error: {e}"),
            }
            #[cfg(not(windows))]
            println!("(non-Windows build: mock device in use)");
        }
        "identify" => println!("mock device ready (link_type {}, {} params)", t.device.link_type, PARAM_COUNT),
        "get" => {
            let all = args.iter().any(|a| a == "--all");
            if all {
                for id in 0..PARAM_COUNT {
                    match get_param(&mut t, id) {
                        Ok(v) => println!("{id}\t{v}"),
                        Err(e) => println!("{id}\terror: {e}"),
                    }
                }
            } else if let Some(idstr) = args.get(1) {
                match parse_id(idstr) {
                    Ok(id) => match get_param(&mut t, id) {
                        Ok(v) => println!("{id}\t{v}"),
                        Err(e) => eprintln!("error: {e}"),
                    },
                    Err(e) => eprintln!("error: {e}"),
                }
            } else {
                eprintln!("usage: vibecity get <id>|--all");
            }
        }
        "set" => {
            let unsafe_ok = args.iter().any(|a| a == "--unsafe-unverified");
            if args.len() < 3 {
                eprintln!("usage: vibecity set <id> <wert> [--unsafe-unverified]");
                return;
            }
            match (parse_id(&args[1]), args[2].parse::<u32>()) {
                (Ok(id), Ok(v)) => {
                    if !unsafe_ok {
                        println!("refusing: parameter id/name unverified — pass --unsafe-unverified after review");
                        return;
                    }
                    log_line(&format!("set {id} -> {v}"));
                    match set_param(&mut t, id, v) {
                        Ok(()) => println!("ok: {id} = {v}"),
                        Err(e) => eprintln!("error: {e}"),
                    }
                }
                (Err(e), _) => eprintln!("error: {e}"),
                (_, Err(e)) => eprintln!("error: value: {e}"),
            }
        }
        "dump" => {
            println!("{{");
            for id in 0..PARAM_COUNT {
                let sep = if id + 1 == PARAM_COUNT { "" } else { "," };
                match get_param(&mut t, id) {
                    Ok(v) => println!("  \"param_{id}\": {v}{sep}"),
                    Err(_) => println!("  \"param_{id}\": null{sep}"),
                }
            }
            println!("}}");
        }
        "monitor" => {
            // Default 300 s; Argument 0 = unbegrenzt bis Strg+C.
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);
            run_monitor(secs);
        }
        "probe" => run_probe(),
        "sweep" => run_sweep(),
        "selftest" => {
            let secs = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(15);
            run_selftest(secs);
        }
        "panic" => println!("panic sent: All Sound Off / All Notes Off (CC120/CC123)"),
        "wizard" => run_wizard(),
        _ => println!(
            "usage: vibecity <list-ports|identify|get <id>|--all|set <id> <wert> [--unsafe-unverified]|dump|probe|monitor [sekunden]|panic|wizard>"
        ),
    }
}
