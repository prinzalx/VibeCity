//! Message construction/parsing per docs/protocol.json.
//! Offsets are evidence-based; fields marked [U] (unverified) are constants
//! with placeholder values and must not be sent to real hardware yet.

use crate::codec::{crc32, dec7_2, dec7_5, enc7_2, enc7_5};

/// Common leading bytes (evidence).
pub const HEADER: [u8; 5] = [0xF0, 0x44, 0x17, 0x04, 0x7F];
/// Parameter message total length (evidence).
pub const PARAM_MSG_LEN: usize = 30;
/// Offset of the sub-command 7-bit pair (evidence).
pub const OFF_SUB_CMD: usize = 0x12;
/// Offset of the 5x7-bit payload / reply status byte (evidence).
pub const OFF_PAYLOAD: usize = 0x18;
/// Status byte value meaning "ok" (evidence).
pub const STATUS_OK: u8 = 1;
/// Link-type variant byte index; default 0x04, becomes 0x02 for link type 3/4 (evidence).
pub const OFF_LINK_BYTE: usize = 3;
/// Bulk header (evidence).
pub const BULK_HEADER: [u8; 8] = [0xF0, 0x44, 0x17, 0x04, 0x7F, 0x03, 0x24, 0x01];
/// Bulk payload start offset (evidence).
pub const OFF_BULK_PAYLOAD: usize = 12;

/// Sub-command ids for the "data" and "command" flavours of the parameter
/// message (evidence: link type 3/4 use 0xCE/0xCD, otherwise 0xD0/0xCF).
pub const SUB_CMD_DATA: u16 = 0xD0; // evidence (non-3/4 link variant)
pub const SUB_CMD_COMMAND: u16 = 0xCF; // evidence (non-3/4 link variant)

/// Same ids for the link-type 3/4 variant (evidence).
pub const SUB_CMD_DATA_LINK34: u16 = 0xCE;
pub const SUB_CMD_COMMAND_LINK34: u16 = 0xCD;

/// Session-enable command ids by link type (evidence).
pub fn session_enable_cmd(link_type: u8) -> u8 {
    if link_type == 3 || link_type == 4 {
        0xB6
    } else {
        0xB8
    }
}

/// Build a 30-byte parameter message. `model_id` is written into the header
/// area at offsets 5/6 ([U] — exact field split unverified).
pub fn build_param_msg(sub_cmd: u16, payload: u32, model_id: u16, link_type: u8) -> Vec<u8> {
    let mut m = vec![0u8; PARAM_MSG_LEN];
    m[..5].copy_from_slice(&HEADER);
    if link_type == 3 || link_type == 4 {
        m[OFF_LINK_BYTE] = 0x02;
    }
    let mid = enc7_2(model_id);
    m[5] = mid[0]; // [U]
    m[6] = mid[1]; // [U]
    let cmd = enc7_2(sub_cmd);
    m[OFF_SUB_CMD] = cmd[0];
    m[OFF_SUB_CMD + 1] = cmd[1];
    let pl = enc7_5(payload);
    m[OFF_PAYLOAD..OFF_PAYLOAD + 5].copy_from_slice(&pl);
    m[PARAM_MSG_LEN - 1] = 0xF7;
    m
}

/// A parsed reply to a parameter message.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub sub_cmd: u16,
    pub status: u8,
    pub payload: u32,
    pub ok: bool,
}

/// Parse a reply frame (minimum length PARAM_MSG_LEN, leading F0).
pub fn parse_param_msg(m: &[u8]) -> Option<Reply> {
    if m.len() < PARAM_MSG_LEN || m[0] != 0xF0 || m[PARAM_MSG_LEN - 1] != 0xF7 {
        return None;
    }
    let sub_cmd = dec7_2([m[OFF_SUB_CMD], m[OFF_SUB_CMD + 1]]);
    let status = m[OFF_PAYLOAD];
    let payload = dec7_5(&m[OFF_PAYLOAD..OFF_PAYLOAD + 5]);
    Some(Reply {
        sub_cmd,
        status,
        payload,
        ok: status == STATUS_OK,
    })
}

/// Build the one-way bulk parameter message (evidence layout; CRC32 variant
/// [U] — IEEE default assumed).
pub fn build_bulk_msg(payload: &[u8], selector: u16, _model_id: u16, link_type: u8) -> Vec<u8> {
    let mut m: Vec<u8> = Vec::with_capacity(OFF_BULK_PAYLOAD + payload.len() + 6);
    m.extend_from_slice(&BULK_HEADER);
    // +8/+9 song number, +0x0A/+0x0B selector (overwritten below); the
    // reference zeroes +9..+0xB before filling, payload starts at +0x0C.
    m.extend_from_slice(&[0, 0, 0, 0]);
    m.extend_from_slice(payload);
    // CRC32 over the first 11 + payload_len bytes starting at byte 1 (evidence).
    let crc = crc32(&m[1..]);
    m.extend_from_slice(&enc7_5(crc));
    m.push(0xF7);
    let sel = enc7_2(selector);
    m[0x0A] = sel[0];
    m[0x0B] = sel[1];
    if link_type == 3 || link_type == 4 {
        m[OFF_LINK_BYTE] = 0x02;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_msg_layout() {
        let m = build_param_msg(SUB_CMD_COMMAND, 0x2A, 0, 2);
        assert_eq!(m.len(), PARAM_MSG_LEN);
        assert_eq!(&m[..5], &HEADER);
        assert_eq!(m[OFF_SUB_CMD], 0xCF & 0x7f);
        assert_eq!(m[OFF_PAYLOAD], 0x2A & 0x7f);
        assert_eq!(m[PARAM_MSG_LEN - 1], 0xF7);
        let r = parse_param_msg(&m).unwrap();
        assert_eq!(r.sub_cmd, SUB_CMD_COMMAND);
    }

    #[test]
    fn reply_roundtrip_status_and_value() {
        let mut m = build_param_msg(SUB_CMD_DATA, 0, 0, 2);
        m[OFF_PAYLOAD] = STATUS_OK;
        // value 0x1234 in 7-bit pairs starting at payload offset
        let v = enc7_5(0x1234);
        m[OFF_PAYLOAD..OFF_PAYLOAD + 5].copy_from_slice(&v);
        // careful: status shares the first payload byte region per evidence;
        // the device writes status at OFF_PAYLOAD, value decoded from there.
        let r = parse_param_msg(&m).unwrap();
        assert_eq!(r.status, v[0]);
        assert_eq!(r.payload, 0x1234 & 0x7f | ((0x1234 >> 7 & 0x7f) << 7));
    }

    #[test]
    fn session_enable_ids() {
        assert_eq!(session_enable_cmd(3), 0xB6);
        assert_eq!(session_enable_cmd(4), 0xB6);
        assert_eq!(session_enable_cmd(1), 0xB8);
    }

    #[test]
    fn bulk_layout() {
        let p = vec![0x11, 0x22, 0x33];
        let m = build_bulk_msg(&p, 0, 0, 2);
        assert_eq!(&m[..8], &BULK_HEADER);
        assert_eq!(m[OFF_BULK_PAYLOAD], 0x11);
        assert_eq!(*m.last().unwrap(), 0xF7);
    }
}
