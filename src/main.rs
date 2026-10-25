//! VibeCity CLI (Phase 3A). On Windows uses WinMM via midir; elsewhere falls
//! back to the built-in mock device. All device I/O is logged to logs/.

use std::io::Write;
use vibecity::protocol::{build_param_msg, parse_param_msg, Reply, SUB_CMD_COMMAND, SUB_CMD_DATA};
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
fn get_param<T: Transport + ?Sized>(t: &mut T, id: usize) -> Result<u32, String> {
    // Observed read path: COMMAND sub-cmd with the id, then expect reply.
    t.send(&build_param_msg(SUB_CMD_COMMAND, id as u32, 0, 2))?;
    let frame = t.recv(900).ok_or("timeout waiting for reply")?;
    let r: Reply = parse_param_msg(&frame).ok_or("malformed reply")?;
    if !r.ok {
        return Err(format!("device status {}", r.status));
    }
    Ok(r.payload)
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

/// Step-by-step hardware wizard (dev tool, R0..W1). Results land in
/// hardware-tests-results.md next to the exe; send that file back.
fn run_wizard() {
    println!("=== VibeCity Wizard (Entwicklungs-Werkzeug) ===");
    let mut t: Box<dyn Transport>;
    let mut port_name = String::from("mock");

    #[cfg(windows)]
    {
        let ports = match vibecity::transport::winmm::list_ports() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Fehler Portliste: {e}");
                return;
            }
        };
        if ports.is_empty() {
            println!("Keine MIDI-Ports gefunden. USB-Kabel/Dongle prüfen.");
        }
        for (i, p) in ports.iter().enumerate() {
            println!("  [{i}] {p:?}");
        }
        let sel = read_line("Port für den Test wählen (Nummer, Enter = Abbruch): ");
        let Ok(idx) = sel.parse::<usize>() else {
            println!("Abbruch.");
            return;
        };
        let Some(p) = ports.get(idx) else {
            println!("Abbruch.");
            return;
        };
        port_name = p.name.clone();
        match vibecity::transport::winmm::WinMidiTransport::open(&port_name) {
            Ok(w) => t = Box::new(w),
            Err(e) => {
                eprintln!("Verbindung fehlgeschlagen: {e}");
                return;
            }
        }
    }
    #[cfg(not(windows))]
    {
        println!("(Nicht-Windows-Build: Mock-Gerät, nur Demonstration)");
        t = Box::new(MockTransport::default());
    }

    let mut results: Vec<(String, char)> = Vec::new();

    // S1 — R0: Port sichtbar (gerade passiert)
    println!("\nR0 Port: Port '{port_name}' wurde gefunden und verbunden.\nRückweg: App schließen, Piano unverändert.");
    results.push((format!("R0 port visible ({port_name})"), ask("R0 ok?")));

    // S2 — R1: ein Parameter lesbar
    println!("\nR1 Lesen: frage Parameter 0 ab …");
    match get_param(&mut *t, 0) {
        Ok(v) => println!("  Parameter 0 = {v}"),
        Err(e) => println!("  Fehler: {e}"),
    }
    results.push(("R1 get param 0".into(), ask("Antwort plausibel erhalten?")));

    // S3 — R2: dump snapshot
    println!("\nR2 Snapshot: alle 57 Parameter werden abgefragt …");
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
        println!("  Snapshot geschrieben: {file} (Fehler: {})", if dump_ok { "keine" } else { "mind. 1" });
    }
    results.push(("R2 snapshot".into(), ask("Dump ok?")));

    // S4 — W1: harmloser Parameter setzen + readback
    println!("\nW1 Schreiben: Parameter-ID und Zielwert, z. B. '3=7' (Enter = überspringen).");
    let input = read_line("  id=value: ");
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
                        Ok(back) => println!("  Readback Parameter {id} = {back}"),
                        Err(e) => println!("  Readback-Fehler: {e}"),
                    },
                    Err(e) => println!("  Set-Fehler: {e}"),
                }
                println!("  Rückweg: alten Wert mit 'set {id} <alt>' zurückschreiben.");
                results.push((format!("W1 set {id}={val}"), ask("Gerät hat Wert übernommen (gelesen/gehört)?")));
            }
            _ => {
                println!("  ungültige Eingabe, übersprungen");
                results.push((format!("W1 set '{input}'"), 's'));
            }
        }
    }

    // Ergebnis-Datei
    let mut md = format!("# Hardware-Test-Ergebnisse\n\nPort: `{port_name}` · Zeitstempel: {ts}\n\n| Schritt | Ergebnis |\n|---|---|\n");
    for (name, o) in &results {
        let s = match o {
            'j' => "ok",
            'n' => "PROBLEM",
            _ => "skip",
        };
        md.push_str(&format!("| {name} | {s} |\n"));
    }
    if std::fs::write("hardware-tests-results.md", md).is_ok() {
        println!("\nErgebnisse gespeichert: hardware-tests-results.md — diese Datei zurück an Hermes.");
    }
    println!("Wizard fertig. (Entwicklungs-Werkzeug, darf später weg.)");
    log_line(&format!(
        "wizard done on {port_name}: {}",
        results.iter().map(|(_, o)| o).collect::<String>()
    ));
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
        "monitor" => println!("monitor: (mock — nothing to observe without a live port)"),
        "panic" => println!("panic sent: All Sound Off / All Notes Off (CC120/CC123)"),
        "wizard" => run_wizard(),
        _ => println!(
            "usage: vibecity <list-ports|identify|get <id>|--all|set <id> <wert> [--unsafe-unverified]|dump|monitor|panic|wizard>"
        ),
    }
}
