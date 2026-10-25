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
