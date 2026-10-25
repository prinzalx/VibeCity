//! 7-bit encodings and SysEx length framing (docs/protocol.json).

/// Encode a 16-bit value as two 7-bit bytes, LSB group first.
pub fn enc7_2(v: u16) -> [u8; 2] {
    [(v & 0x7f) as u8, ((v >> 7) & 0x7f) as u8]
}

/// Decode two 7-bit bytes (LSB first) back to a 16-bit value.
pub fn dec7_2(b: [u8; 2]) -> u16 {
    ((b[0] & 0x7f) as u16) | (((b[1] & 0x7f) as u16) << 7)
}

/// Encode a 32-bit value as five 7-bit bytes, LSB group first.
pub fn enc7_5(v: u32) -> [u8; 5] {
    [
        (v & 0x7f) as u8,
        ((v >> 7) & 0x7f) as u8,
        ((v >> 14) & 0x7f) as u8,
        ((v >> 21) & 0x7f) as u8,
        ((v >> 28) & 0x7f) as u8,
    ]
}

/// Decode five 7-bit bytes (LSB first) back to a 32-bit value.
pub fn dec7_5(b: &[u8]) -> u32 {
    let mut v: u32 = 0;
    for (i, byte) in b.iter().take(5).enumerate() {
        v |= ((byte & 0x7f) as u32) << (7 * i);
    }
    v
}

/// Number of bytes a SysEx payload length requires under the observed framing
/// (docs/protocol.json framing.length_thresholds), LSB 7-bit groups,
/// continuation bit 0x80 on all but the last group.
pub fn sysx_length_bytes(len: usize) -> usize {
    if len < 0x80 {
        1
    } else if len < 0x4000 {
        2
    } else if len < 0x20_0000 {
        3
    } else if len < 0x1000_0000 {
        4
    } else {
        0
    }
}

/// Encode a payload length as framed 7-bit groups (continuation bit set on all
/// but the final byte). Returns empty for out-of-range lengths.
pub fn sysx_length_prefix(len: usize) -> Vec<u8> {
    let n = sysx_length_bytes(len);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let shift = 7 * (n - 1 - i);
        let mut b = ((len >> shift) & 0x7f) as u8;
        if i + 1 < n {
            b |= 0x80;
        }
        out.push(b);
    }
    out
}

/// IEEE / zlib CRC-32 (reflected, poly 0xEDB88320). The bulk family's CRC32
/// variant is marked unverified in protocol.json; this is the common default.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip7_2() {
        for v in [0u16, 1, 0x7f, 0x80, 0x3fff, u16::MAX] {
            assert_eq!(dec7_2(enc7_2(v)), v & 0x3fff);
        }
    }

    #[test]
    fn roundtrip7_5() {
        for v in [0u32, 1, 0x7f, 0x4000, u32::MAX] {
            assert_eq!(dec7_5(&enc7_5(v)), v);
        }
    }

    #[test]
    fn length_prefix_boundaries() {
        assert_eq!(sysx_length_prefix(0x7f), vec![0x7f]);
        assert_eq!(sysx_length_prefix(0x80), vec![0x81, 0x00]);
        assert_eq!(sysx_length_prefix(0x3fff), vec![0xff, 0x7f]);
        assert_eq!(sysx_length_prefix(0x4000), vec![0x81, 0x80, 0x00]);
        assert_eq!(sysx_length_prefix(0x20_0000), vec![0x81, 0x80, 0x80, 0x00]);
        assert!(sysx_length_prefix(0x1000_0000).is_empty());
    }

    #[test]
    fn crc32_known_vector() {
        // Standard CRC-32 of "123456789" == 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
