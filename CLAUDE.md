# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A single Rust executable that continuously scans nearby BLE advertisements, stores each observation in SQLite, and serves a self-contained web dashboard — all in one binary. See `README.md` for user-facing capabilities and limits.

## Commands

```bash
cargo build --release          # produces target/release/blezer
cargo test                     # unit tests live in src/ble/parse.rs (#[cfg(test)])
cargo test decodes_airpods     # run a single test by name substring

./scripts/package.sh           # build + assemble macOS .app bundle + codesign + emit LaunchAgent plist

# CLI subcommands
blezer run [--port 8080] [--db PATH] [--no-scan] [--host 127.0.0.1]
blezer devices [--window 300]
blezer stats   [--window 3600]
```

- **Toolchain:** requires Rust **1.85+** (deps like `clap_lex`, `hashlink` need `edition2024`). Older stable fails to parse their manifests.
- `--no-scan` runs web-only (no Bluetooth); `--host 0.0.0.0` is for sandboxed preview. The real app should bind `127.0.0.1` only.

## macOS Bluetooth (the critical, non-obvious part)

CoreBluetooth **SIGABRTs** on first Bluetooth access unless `NSBluetoothAlwaysUsageDescription` is present *and* honored by TCC. Two things must both hold:

1. **The plist must be recognized.** `build.rs` embeds `Info.plist` into the binary's `__TEXT,__info_plist` section, and `package.sh` re-`codesign`s the `.app` so the CodeDirectory hashes it (`Info.plist entries=N`). A linker-signed ad-hoc signature alone is **not** enough.
2. **The process must be its own TCC "responsible process."** macOS attributes the Bluetooth request to the responsible process, not our binary. When spawned from another GUI app (e.g. a terminal, IDE, or Claude), that app is held responsible and crashes us if it lacks the key. **Launch via launchd** (`open "/Applications/Blezer.app" --args run ...` or a LaunchAgent) so the app is responsible for itself.

Consequence: **you cannot run the scanning binary directly from this agent's shell** — it will SIGABRT. To exercise the web UI here, run `--no-scan` (no CoreBluetooth) or launch the `.app` via `open`. DB defaults to `~/Library/Application Support/Blezer/blezer.db` (auto-created); pass `--db` only to override, and use an **absolute** path since launchd cwd is `/`.

## Architecture

Data flows one direction: **scanner → parse → db → web**.

- `src/main.rs` — clap CLI; `run` starts a Tokio runtime with a scanner task + Axum server sharing one `Arc<Mutex<Connection>>`.
- `src/ble/scanner.rs` — btleplug event loop. Reads `PeripheralProperties`, normalizes to `Observation`, throttles per-device writes (`OBSERVATION_THROTTLE_SECS`). `ADDRESS_TYPE` is `"uuid"` on macOS (CoreBluetooth hides MAC) vs `"mac"` elsewhere — this cfg split is deliberate and the DB schema carries `address_type` so both platforms coexist.
- `src/ble/parse.rs` — company-id→vendor lookup + best-effort Apple decode. Apple manufacturer data is TLV; type `0x07` (proximity pairing) is **cleartext** and yields exact AirPods/Beats model + battery + charging (see `audio_model`, `airpods_details`). iPhone `0x10` (Nearby) carries no model or name. **User-facing names are never in advertisements** — they only appear as a BLE Local Name in specific states (e.g. iPhone Settings→Bluetooth open), captured as `local_name`.
- `src/db/mod.rs` — rusqlite (bundled), WAL. `devices` (aggregated, upserted) + `observations` (append-only). `raw_mfg_data` is stored intentionally for future fingerprinting. Queries filter by a `last_seen`/`ts` time window.
- `src/web/mod.rs` + `dashboard.html` — Axum serves the dashboard via `include_str!` (keeps the single-binary property; no external assets/CDN). Endpoints: `/`, `/api/devices`, `/api/devices/{id}` (RSSI history), `/api/stats`.

## Conventions

- **The dashboard is compiled into the binary.** Any `dashboard.html` change requires `cargo build` (and repackage/relaunch) before it's visible — the running server won't hot-reload it.
- Display label priority is **name-first**: `local_name` leads, `model`/category is secondary. This holds in both the web table and the CLI `devices` output — keep them consistent.
- RSSI `127` (and any positive value) is the "unavailable" sentinel; `scanner.rs` maps it to `None`. Don't let it into stats.
- Comments and UI strings are in Traditional Chinese; match that when editing.

## Previewing the dashboard in-agent

`.claude/launch.json` defines a `dashboard` config that runs the release binary with `--no-scan --host 0.0.0.0 --port 8090` against the existing `ble.db`. Use it with the Claude Preview tools. It reads the same DB a launchd scanner writes (WAL allows concurrent read), so it shows live data without needing Bluetooth. The binary must be built first.
