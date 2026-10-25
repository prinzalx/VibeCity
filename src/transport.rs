//! Transport abstraction. Real WinMM transport via midir on Windows; a
//! simulated device for tests and non-Windows builds.

use crate::codec::{dec7_5, enc7_5};
use crate::protocol::{parse_param_msg, OFF_PAYLOAD, STATUS_OK, SUB_CMD_COMMAND, SUB_CMD_DATA};
use crate::PARAM_COUNT;

/// What a caller can do against the piano or a stand-in.
pub trait Transport {
    /// Send one complete SysEx frame.
    fn send(&mut self, frame: &[u8]) -> Result<(), String>;
    /// Block up to `timeout_ms`, return the next complete frame if any.
    fn recv(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
}

/// Endpoint classification (Amendment: BLE via dongle is a first-class
/// transport; detection by name heuristic on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    Usb,
    Ble,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub name: String,
    pub kind: EndpointKind,
}

/// Classify a WinMM/midir port name. [U] heuristic, refine on the target PC.
pub fn classify_port(name: &str) -> EndpointKind {
    let n = name.to_ascii_lowercase();
    let ble_token = n.split_whitespace().any(|w| w == "ble");
    if ble_token || n.contains("bluetooth") {
        EndpointKind::Ble
    } else if n.contains("usb") {
        EndpointKind::Usb
    } else {
        EndpointKind::Unknown
    }
}

/// A simulated piano: keeps PARAM_COUNT values, answers parameter messages
/// like the observed device (status byte == 1, reply value = baseline + value
/// at the payload offset).
pub struct MockDevice {
    pub values: [u16; PARAM_COUNT],
    pub baseline: [u16; PARAM_COUNT],
    pub link_type: u8,
    pending_value: u16,
    outbox: Vec<Vec<u8>>,
}

impl Default for MockDevice {
    fn default() -> Self {
        Self {
            values: [0; PARAM_COUNT],
            baseline: [0x20; PARAM_COUNT],
            link_type: 2,
            pending_value: 0,
            outbox: Vec::new(),
        }
    }
}

impl MockDevice {
    /// Interpret one frame, per the observed write sequence
    /// (data=value first, then command=id) and queue the reply.
    fn handle(&mut self, frame: &[u8]) {
        let Some(r) = parse_param_msg(frame) else {
            return;
        };
        match r.sub_cmd {
            c if c == SUB_CMD_DATA => self.pending_value = r.payload as u16,
            c if c == SUB_CMD_COMMAND => {
                let id = (r.payload as usize) % PARAM_COUNT;
                self.values[id] = self.pending_value;
            }
            _ => {}
        }
        let mut reply = frame.to_vec();
        // Device replies overlap status and value at OFF_PAYLOAD (evidence,
        // ambiguity [U]): mock encodes (baseline+value)<<7 | STATUS_OK so the
        // low 7-bit group is the status byte, matching parse_param_msg.
        let id = dec7_5(&frame[OFF_PAYLOAD..OFF_PAYLOAD + 5]) as usize % PARAM_COUNT;
        let total = self.baseline[id] as u32 + self.values[id] as u32;
        let enc = enc7_5((total << 7) | STATUS_OK as u32);
        reply[OFF_PAYLOAD..OFF_PAYLOAD + 5].copy_from_slice(&enc);
        self.outbox.push(reply);
    }

    fn take_reply(&mut self) -> Option<Vec<u8>> {
        self.outbox.pop()
    }
}

/// Transport backed by a MockDevice (no real MIDI involved).
pub struct MockTransport {
    pub device: MockDevice,
}

impl Default for MockTransport {
    fn default() -> Self {
        Self {
            device: MockDevice::default(),
        }
    }
}

impl Transport for MockTransport {
    fn send(&mut self, frame: &[u8]) -> Result<(), String> {
        self.device.handle(frame);
        Ok(())
    }
    fn recv(&mut self, _timeout_ms: u32) -> Option<Vec<u8>> {
        self.device.take_reply()
    }
}

#[cfg(windows)]
pub mod winmm {
    use super::{classify_port, PortInfo, Transport};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};

    /// List output ports via midir/WinMM (USB and BLE endpoints both appear
    /// there on Windows 10+, per docs/PROTOCOL.md).
    pub fn list_ports() -> Result<Vec<PortInfo>, String> {
        let io = midir::MidiOutput::new("vibecity").map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for p in io.ports() {
            let name = io.port_name(&p).unwrap_or_else(|_| "?".into());
            out.push(PortInfo {
                kind: classify_port(&name),
                name,
            });
        }
        Ok(out)
    }

    /// Real WinMM transport: SysEx out over one MIDI out port, inbound
    /// SysEx frames reassembled from a MIDI in port of the same name.
    pub struct WinMidiTransport {
        sender: midir::MidiOutputConnection,
        _in_conn: Option<midir::MidiInputConnection<RxState>>,
        rx: Receiver<Vec<u8>>,
    }

    struct RxState {
        tx: mpsc::Sender<Vec<u8>>,
        acc: Vec<u8>,
    }

    impl WinMidiTransport {
        pub fn open(port_name: &str) -> Result<Self, String> {
            let (tx, rx) = mpsc::channel();

            let mut mo = midir::MidiOutput::new("vibecity-out").map_err(|e| e.to_string())?;
            let out_port = mo
                .ports()
                .into_iter()
                .find(|p| mo.port_name(p).map(|n| n == port_name).unwrap_or(false))
                .ok_or_else(|| format!("out port '{port_name}' not found"))?;
            let sender = mo
                .connect(&out_port, "vibecity")
                .map_err(|e| e.to_string())?;

            let mut mi = midir::MidiInput::new("vibecity-in").map_err(|e| e.to_string())?;
            let in_port = mi
                .ports()
                .into_iter()
                .find(|p| mi.port_name(p).map(|n| n == port_name).unwrap_or(false));

            let in_conn = match in_port {
                Some(port) => Some(
                    mi.connect(
                        &port,
                        "vibecity",
                        move |_ts, msg, st: &mut RxState| {
                            // Reassemble SysEx frames across callback chunks;
                            // pass short messages through as one-event frames.
                            if msg[0] == 0xF0 {
                                st.acc.extend_from_slice(msg);
                                if msg.last() == Some(&0xF7) {
                                    let _ = st.tx.send(std::mem::take(&mut st.acc));
                                }
                            } else {
                                let _ = st.tx.send(msg.to_vec());
                            }
                        },
                        RxState {
                            tx: tx.clone(),
                            acc: Vec::new(),
                        },
                    )
                    .map_err(|e| e.to_string())?,
                ),
                None => None,
            };

            Ok(Self {
                sender,
                _in_conn: in_conn,
                rx,
            })
        }
    }

    impl Transport for WinMidiTransport {
        fn send(&mut self, frame: &[u8]) -> Result<(), String> {
            self.sender.send(frame).map_err(|e| e.to_string())
        }
        fn recv(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
            match self.rx.recv_timeout(std::time::Duration::from_millis(timeout_ms as u64)) {
                Ok(v) => Some(v),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::build_param_msg;

    #[test]
    fn mock_write_sequence() {
        let mut t = MockTransport::default();
        // write value 7 then id 3 (observed order)
        t.send(&build_param_msg(SUB_CMD_DATA, 7, 0, 2)).unwrap();
        t.recv(10); // reply consumed
        t.send(&build_param_msg(SUB_CMD_COMMAND, 3, 0, 2)).unwrap();
        t.recv(10);
        assert_eq!(t.device.values[3], 7);
    }

    #[test]
    fn classify_ports() {
        assert_eq!(classify_port("Casio PX-S (USB)"), EndpointKind::Usb);
        assert_eq!(classify_port("BLE MIDI"), EndpointKind::Ble);
        assert_eq!(classify_port("Microsoft GS Wavetable"), EndpointKind::Unknown);
    }
}
