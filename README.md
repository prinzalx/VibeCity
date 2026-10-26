# VibeCity

Host-Tool für das **Casio PX-S1100** unter **Windows 10 x64** — Piano-Einstellungen
lesen und schreiben über MIDI (USB oder **WU-BT10** BLE-MIDI-Dongle).

Status: **Protokoll gelöst und am Gerät bewiesen (26.10.2026)** · CLI funktionsfähig · GUI im App-Stil (Remote Controller) mit echten Parameternamen, Slidern, Checkboxen & Dropdowns · Version **0.3.0**

## Was es tut

- Alle Piano-Parameter (53 IDs) einzeln oder komplett lesen (`read-all`)
- Einzelne Parameter schreiben — mit automatischer Rücklese-Verifikation (`set`)
- Live-Mitschnitt aller Piano-Meldungen (`monitor`)
- Diagnose-Kommandos für Verbindung/Ports (`list-ports`, `probe`, `sweep`, `status-probe`)

## Schnellanfang (Windows)

Exe aus den [Releases](../../releases) herunterladen (oder bauen, siehe unten):

```bat
vibecity.exe list-ports
vibecity.exe read-all
vibecity.exe set 07 01 42
vibecity-gui.exe        :: grafische Oberfläche; verbindet beim Start automatisch, wenn ein CASIO-Port da ist
```

Ausgabe von `read-all` geht live in die Konsole und als Log + JSON nach `logs\`.

## Protokoll-Kurzfassung (am Gerät bewiesen)

| Richtung | Frame |
|---|---|
| Lesen (11 B) | `F0 44 7E 7E 7F 0F 00 <IDhi> <IDlo> 00 F7` |
| Antwort (17/19 B) | `F0 44 7E 7E 7F 0F 01 <IDhi> <IDlo> 01 <typ> 01 00 <wert…> F7` |
| Schreiben (16 B) | `F0 44 7E 7E 7F 0F 01 <IDhi> <IDlo> 01 00 01 00 <vlo> <vhi> F7` |

Details und Einschränkungen: [docs/PROTOCOL.md](docs/PROTOCOL.md) · ID-Tabelle: [docs/param-ids.md](docs/param-ids.md)

## Bauen (aus WSL oder Windows)

```sh
cargo build --release                                      # Windows-Ziel-PC
cargo build --release --target x86_64-pc-windows-gnu --target-dir tgt-win   # Cross-Build aus WSL
```

Binaries: `vibecity` (CLI) und `vibecity-gui` (GUI im Stil des App-Remote-Controllers: Kategorie-Kacheln, Auto-Connect beim Start, Alle-lesen mit Fortschrittsbalken, Schreiben mit Rücklese).

## Verbindungswege

- **USB:** Kabel von Piano-Buchse *TO HOST (USB-B)* zum PC; Port „CASIO USB-MIDI“.
- **BLE:** WU-BT10-Dongle in *TO DEVICE* (Typ A) am Piano; Windows-10-eigener BLE-MIDI-Treiber; kein Pairing nötig.
- Beide Wege parallel: *TO HOST* (USB) und *TO DEVICE* (Dongle) sind unabhängige Kanäle.

## Repo-Inhalt

- `src/` — Rust-Core (Bibliothek), CLI (`main.rs`), GUI (`gui.rs`)
- `docs/` — Protokoll (Clean-Room), Parameter-IDs, Hardware-Testprotokoll
- `docs/HARDWARE_TESTS.md` — Test-Gates R0–R6 am realen Gerät

## Changelog

- **26.10.2026 (v0.3.0)** — Alle 52 Parameter-IDs nachtennt (statisch via `csUnityOperator`, ohne Gerät); Bereiche/Defaults aus der App-Tabelle (`docs/param-table.md`); GUI mit Slidern/Checkboxen/Dropdowns statt Eingabefeldern; Dark-Mode-Schalter.

- **26.10.2026 (v0.2.0)** — GUI im App-Stil (Kacheln, Kopfzeile, heller Look), Auto-Connect + Auto-Read beim Start mit Fortschrittsbalken, Verbindungsstatus als eigener Zustand behoben.
- **26.10.2026** — Lese-/Schreibpfad am Gerät bewiesen (Read 53/53, Write+Rücklese OK); `read-all` mit JSON-Ausgabe; GUI-Grundlage (`vibecity-gui`) hinzugefügt.
- **25.–26.10.2026** — Phase 3A: Core + CLI, BLE-Analyse, Wizard, Kanal-Findung (7E7E-Statuskanal).
