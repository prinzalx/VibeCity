# Hardware tests (PX-S1100)

Gates per PROMPT_phase3a: R0 ports/identity → R1 single gets → R2 snapshot →
W1 one harmless set → W2 grouped sets. Each row is filled after a logged run;
status values: `open`, `ok`, `failed`, `n/a`.

| Gate | Transport | Date | Result | Notes |
|---|---|---|---|---|
| R0 list-ports | USB | — | open | PX-S1100 endpoint visible |
| R0 list-ports | BLE (WU-BT10) | — | open | visible without pairing on Win10 |
| R1 get <id> | USB | — | open | |
| R1 get <id> | BLE | — | open | |
| R2 dump | USB | — | open | |
| W1 set + readback | USB | — | open | safest parameter pending R1 results |
| W1 set + readback | BLE | — | open | |

Runbook (user, Windows): run `vibecity list-ports`, then per gate the exact
commands listed in the CLI section of the report; send back `logs/session-cli.log`
plus any `monitor` capture. Device stays connected via USB cable for the first
pass; BLE pass afterwards (dongle swap requires replug).

### BLE connect behavior (evidence from the reference app + OS docs)

The reference Android app scans actively for the standard BLE-MIDI GATT service
UUID (`03B80E5A-EDE8-4B33-A751-6CE34EC4C700`) and connects via
`MidiManager.openBluetoothDevice` **without any `createBond()` call** — the
BLE-MIDI spec allows unbonded connections, which is why the phone never asked
for pairing. On Windows the in-box "Bluetooth LE MIDI" driver (Win10 1703+)
performs the same service-UUID discovery at OS level and exposes the dongle as
an ordinary MIDI endpoint; pairing is optional ("do not pair unless the device
requires it"). Our tool therefore contains no pairing logic: if the endpoint
is missing in `list-ports`, suspect the PC Bluetooth adapter/driver first.

## Wizard (dev tool)

`vibecity wizard` runs R0→R1→R2→W1 interactively (port selection, snapshot
file, set+readback, j/n/s per step) and writes `hardware-tests-results.md`
next to the exe. The tables above stay the archive of record; the wizard is
allowed to disappear from the final build.
