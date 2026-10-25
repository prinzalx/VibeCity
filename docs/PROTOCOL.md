# Piano Parameter Protocol (clean-room description)

Describes how a host application exchanges settings with the Casio PX-S1100
over USB-MIDI, as observed in a shipped Android piano companion app's native
protocol engine (binary analysis only; no vendor SDK or documentation copied).
All byte sequences below are facts taken from decompiled message builders; the
wording and structure are original.

Status legend: **[E]** evidence (observed in the shipped binary), **[U]** unverified / to be confirmed at the device.

## 1. Transport

- MIDI 1.0 over USB (class-compliant device port) **or** BLE-MIDI via the vendor USB dongle on the piano's host port (PC side needs a Bluetooth LE adapter; USB and BLE cannot run simultaneously on the piano). **[E]** for the app path; **[U]** for exact Windows behavior.
- Target OS includes Windows 10 x64: WinMM is the baseline stack; Windows MIDI Services exist on Windows 11 only and are therefore not a requirement. BLE-MIDI endpoints surface as ordinary WinMM ports through the in-box Windows 10 BLE-MIDI driver. **[E: OS docs]** / **[U: to verify on the target PC]**
- All parameter traffic is System Exclusive (`F0 … F7`).
- A framing layer writes messages through a byte-buffered sender. On closing a SysEx message (trailing `F7`) it inserts the payload length after the status byte using 7-bit groups, least significant group first, with the continuation bit (`0x80`) set on all but the final group: 1 byte if < 0x80, 2 if < 0x4000, 3 if < 0x200000, 4 if < 0x10000000. **[E]**
- Channel voice messages pass through unmodified (2-byte forms for `0xC0–0xDF`). **[E]**

## 2. Message families

Common leading bytes of the modern parameter messages: `F0 44 17 04 7F …` **[E]**
(with byte index 3 changed to `02` when the device reports link type 3 or 4). **[E]**
A model identifier is written into the header by a dedicated helper on every message. **[E]**

### 2.1 Parameter request (read) and parameter set (write)

Short fixed-size messages of total length **30 bytes (0x1E)**, ending in `F7` at offset `0x1D`. **[E]**

Layout (offsets from `F0`): **[E]**

| Offset | Content |
|---|---|
| 0x00–0x04 | `F0 44 17 04 7F` (index 3 = `02` for link types 3/4) |
| header area | model identifier (written by helper; exact field split **[U]**) |
| 0x12–0x13 | sub-command identifier, 7-bit pair (two distinct values: one names "data", one names "command") **[E]** |
| 0x18–0x1C | payload, 5 × 7-bit (parameter id **or** value, 16-bit domain observed) **[E]** |
| 0x1D | `F7` |

A write operation emits the pair "value under the *data* sub-command, then id under the *command* sub-command"; a read emits the command part followed by a request under the data sub-command. **[E]**

Legacy short path (device ids `0x19`/`0x1A`, commands `0xC4`/`0xC5`) exists on the same transport. **[E]** Presence/absence of a checksum on this path: **[U]**.

### 2.2 Bulk parameter set (one-way)

Header bytes `F0 44 17 04 7F 03 24 01`, three zero bytes, payload starting at offset `0x0C`. **[E]**

- 7-bit pair at `0x0A` (mode/selector argument), 7-bit pair for song number at `0x08`. **[E]**
- **CRC-32** over the first `11 + payload_len` bytes starting at byte 1, appended after the payload as 5 × 7-bit, terminated by `F7`. **[E]** The implementation is a table-driven CRC-32 with init `0xFFFFFFFF` and final inversion (IEEE/zlib variant). **[E]**
- No response wait for this family (one-way). **[E]**

### 2.3 Session-enable command ids

A per-link-type constant selects the enable/response command id: link type 3 or 4 → `0xB6`, otherwise `0xB8`. **[E]**

## 3. Read/write handshake **[E]**

1. Host sets "session start" state and dispatches the request through the host framework (the native engine hands thread management to the host UI layer).
2. Sender builds and emits the message(s) described above.
3. Receiving thread polls every **2 ms** for an "answer arrived" flag; timeout threshold **0.9** (units interpreted as seconds, **[U]**).
4. On answer: verify status byte at offset `0x18` of the receive buffer (value `1` = success, small integer = retry/legacy status); on success apply value and persist; on timeout mark session error.
5. Session result states observed: set-ok, set-error, get-ok, get-error (numeric codes 2/3/5/6 in the binary).

## 4. Value encoding

- Parameter values are **16-bit** slots in a flat array; count for the PX-S-era family: **57 parameter slots plus 2 tone-selection slots**. **[E]** (A second model family uses 232 slots. **[E]**)
- Wire encoding of ids/values: 7-bit pairs (16-bit) or 7-bit quintets (32-bit), least significant group first. **[E]**
- Read replies carry a value **relative to a device baseline table** maintained by the engine (reply value minus baseline = stored value). **[E]**
- Parameter ids are 16-bit numerals; the human-readable name ↔ id mapping for individual parameters (reverb, chorus, brilliance, transpose, tuning, temperament, layer/split/duet, metronome, touch curve, auto power off, audio/Bluetooth) is **[U]** — to be fixed by a live capture.

## 5. Persistence model

After each successful set/get the engine serializes the full value array as a **comma-separated decimal list** into a per-model file, selected by model index and one of **5 save destinations**, under an application-private directory. **[E]** This mirrors a "snapshot of device settings" strategy rather than journaling.

## 6. Device facts relevant to the protocol

- PX-S1100: no built-in Bluetooth; BLE MIDI only via the vendor USB dongle (which occupies the USB host port, mutually exclusive with wired USB). **[E: vendor spec]**
- The engine recognizes a family of model identifiers `0x1D–0x3B` plus legacy names (MX-944/945/946/947). Which identifier the PX-S1100 uses: **[U]**.

## 7. Open questions for Phase 3 verification

1. Name ↔ id map for the ESSENTIAL parameters (needs live capture or vendor table).
2. ~~CRC-32 variant~~ resolved: table-driven IEEE CRC-32 (evidence).
3. Exact header field split between offsets 0x05 and 0x11.
4. Whether the legacy `0xC4/0xC5` short path needs a checksum.
5. PX-S1100 model id and its persistence-file slot name.
