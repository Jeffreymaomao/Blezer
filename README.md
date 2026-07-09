<p align="center">
  <img src="/assets/logo.png" width="160" alt="Blezer logo">
</p>

<h1 align="center">Blezer</h1>

<p align="center">
  A single Rust binary that continuously scans nearby BLE advertisements, stores them in SQLite, and serves a built-in web dashboard on the same port.
</p>

<p align="center">
  Built on macOS · portable to Linux / Raspberry Pi · zero external dependencies (single binary, dashboard embedded)
</p>

<p align="center">
  <b>English</b> · <a href="README.zh.md">繁體中文</a>
</p>

---

## What it is

Blezer passively listens to the BLE advertisement packets around you, writes every observation into a local SQLite database, and renders them live through a built-in web dashboard. The whole tool is **one executable**: scanning, storage, and the web UI are all bundled inside — the dashboard is compiled into the binary too, so no CDN or external assets are needed.

Because a BLE advertisement carries only limited information, and modern phones rotate their random address every ~15 minutes, Blezer is designed for **census / known-device-presence detection**, not long-term tracking of unknown individuals.

## What you can get

Verified on macOS:

- **Vendor** (reliable) — resolved from the SIG company id in the manufacturer data (Apple / Microsoft / Samsung / Xiaomi …).
- **Apple message category** — e.g. `Apple Nearby (iPhone/iPad)`, `Apple Find My`.
- **Full cleartext for AirPods / Beats** — proximity pairing (type `0x07`) is unencrypted, so you get the **exact model + left/right/case battery + charging state**, e.g. `AirPods Pro · L70% R80% case10%`, with `⚡` when charging. Model-code table lives in [`src/ble/parse.rs`](src/ble/parse.rs) (`audio_model`, sourced from furiousMAC/continuity). ⚠️ Earbuds only emit this advertisement when the case is open / pairing / disconnected.
- **Peripheral device names** — devices that broadcast a local name (earbuds, bands, watches) show it, e.g. `Mi Smart Band 6`, `Forerunner 935`.
- **RSSI**, hit count, first-seen / last-seen timestamps.

## What you cannot get

- **The exact model of a plain iPhone** (e.g. `iPhone18,2`) — the Nearby (`0x10`) advertisement carries neither the model nor a name; you'd need a GATT connection or the same Apple ID.
- **User-assigned names** ("XXX's iPhone / AirPods") — these are **not in any advertisement**. Your own Apple devices see them only because of iCloud account sync, hash-matching the Apple ID in the message against your contacts, or an existing pairing; third-party passive scanning cannot recover them.
- **MAC addresses** — macOS CoreBluetooth only exposes a rotating UUID (`address_type=uuid`); only Linux / BlueZ gives you the raw MAC (`address_type=mac`).

## Install & run (macOS)

```bash
./scripts/package.sh          # cargo build --release + assemble .app + codesign + emit LaunchAgent
```

> **Requires Rust 1.85+** (some deps like `clap_lex`, `hashlink` need `edition2024`).

### ⚠️ The critical detail about macOS Bluetooth permission

macOS decides Bluetooth authorization by the **responsible process**. If another GUI app (a terminal, an IDE) spawns the scanner as a child process, the request is attributed to *that* app; if it hasn't declared a Bluetooth usage description, CoreBluetooth **SIGABRTs** immediately.

The fix: launch the packaged `.app` via **launchd** so it becomes its own responsible process and applies the `NSBluetoothAlwaysUsageDescription` bundled inside it.

**Run in the background (recommended, auto-starts at login):**

```bash
cp "dist/com.blezer.agent.plist" ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.blezer.agent.plist
# The first run pops the Bluetooth permission dialog → click "Allow"
```

Dashboard: <http://127.0.0.1:8080/>　Logs: `/tmp/blezer.out.log`

Disable:

```bash
launchctl unload ~/Library/LaunchAgents/com.blezer.agent.plist
```

**Run once:**

```bash
open "/Applications/Blezer.app" --args run --port 8080
```

**Running directly from your own Terminal** works too (Terminal becomes the responsible process; fine after the first authorization):

```bash
cargo run --release -- run
```

> The DB defaults to `~/Library/Application Support/Blezer/blezer.db` (auto-created).
> Pass `--db` only to override the location; since launchd / `open` run with a working directory of `/`, always use an absolute path when overriding.

## CLI

```bash
blezer run     [--port 8080] [--db PATH]      # scan + dashboard, long-running (--db defaults to App Support)
blezer devices [--window 300]                 # list recent devices in the terminal
blezer stats   [--window 3600]                # print a census in the terminal
```

## Web API

| Endpoint | Description |
|---|---|
| `GET /` | Dashboard (polls the APIs below every 2s) |
| `GET /api/devices?window=<secs>` | Devices seen recently |
| `GET /api/devices/{id}?window=<secs>` | RSSI time series for a single device |
| `GET /api/stats?window=<secs>` | Census (device count, observation count, RSSI, vendor distribution) |

## Database

SQLite (WAL). `devices` holds the aggregated devices; `observations` holds every observation, including the raw advertisement payload in `raw_mfg_data` — kept intentionally for future "advertisement fingerprint / behavior fingerprint" re-identification.

## Linux / Raspberry Pi

`btleplug` is cross-platform (it switches to BlueZ), so `cargo build --release` is all you need. Linux has no macOS responsible-process restriction, but it does need Bluetooth permission (usually `setcap` or running as a privileged user); use a systemd service to run it in the background. `address_type` will be `mac`.

## Building

```bash
cargo build --release          # produces target/release/blezer
cargo test                     # unit tests (src/ble/parse.rs)
cargo test decodes_airpods     # run a single test by name substring
```

## Credits

- Apple continuity / proximity pairing model table: [furiousMAC/continuity](https://github.com/furiousMAC/continuity)
- README layout inspired by: [ts1/BLEUnlock](https://github.com/ts1/BLEUnlock)
