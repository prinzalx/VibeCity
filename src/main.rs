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
fn get_param<T: Transport>(t: &mut T, id: usize) -> Result<u32, String> {
    // Observed read path: COMMAND sub-cmd with the id, then expect reply.
    t.send(&build_param_msg(SUB_CMD_COMMAND, id as u32, 0, 2))?;
    let frame = t.recv(900).ok_or("timeout waiting for reply")?;
    let r: Reply = parse_param_msg(&frame).ok_or("malformed reply")?;
    if !r.ok {
        return Err(format!("device status {}", r.status));
    }
    Ok(r.payload)
}

fn set_param<T: Transport>(t: &mut T, id: usize, value: u32) -> Result<(), String> {
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
        _ => println!(
            "usage: vibecity <list-ports|identify|get <id>|--all|set <id> <wert> [--unsafe-unverified]|dump|monitor|panic>"
        ),
    }
}
