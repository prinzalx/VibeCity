//! VibeCity core: 7-bit codec, SysEx length framing, CRC32 (bulk path),
//! per the clean-room docs/protocol.json. No Casio vendor strings.

pub mod codec;
pub mod protocol;
pub mod transport;

pub use codec::{dec7_5, enc7_2, enc7_5, sysx_length_prefix};

/// Number of addressable parameters for the PX-S family (docs/protocol.json).
pub const PARAM_COUNT: usize = 57;
