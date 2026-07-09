<p align="center">
  <img src="/assets/logo.png" width="160" alt="Blezer logo">
</p>

<h1 align="center">Blezer</h1>

<p align="center">
  單一 Rust 執行檔 — 持續掃描附近 BLE 廣播，寫入 SQLite，並在同一個 port 提供內建 Web dashboard。
</p>

<p align="center">
  macOS 開發 · 可攜到 Linux / Raspberry Pi · 零外部相依（單一 binary，dashboard 內嵌）
</p>

<p align="center">
  <a href="README.md">English</a> · <b>繁體中文</b>
</p>

---

## 這是什麼

Blezer 被動監聽周遭的 BLE 廣播封包，把每一筆觀測寫進本機 SQLite，再用內建的 Web dashboard 即時呈現。整個工具就是**一個執行檔**：掃描、儲存、網頁介面全部包在裡面，dashboard 也編進 binary，不需要 CDN 或外部資產。

因為 BLE 廣播只提供有限資訊，且現代手機每 ~15 分鐘就會輪替隨機位址，Blezer 的定位是**群體普查 / 已知裝置在場偵測**，而不是對陌生個體做長期追蹤。

## 能做什麼

實測（macOS）可以拿到：

- **廠牌**（可靠）— 由 manufacturer data 的 SIG company id 解出（Apple / Microsoft / Samsung / Xiaomi …）。
- **Apple 訊息類別** — 如 `Apple Nearby (iPhone/iPad)`、`Apple Find My`。
- **AirPods / Beats 完整明文** — proximity pairing（type `0x07`）未加密，可解出**精確機型 + 左右耳/耳機盒電量 + 充電狀態**，例如 `AirPods Pro · L70% R80% 盒10%`，充電中標 `⚡`。機型代碼對照見 [`src/ble/parse.rs`](src/ble/parse.rs)（`audio_model`，來源：furiousMAC/continuity）。⚠️ 耳機只有在開蓋 / 配對 / 未連線時才發此廣播。
- **周邊裝置名稱** — 有 local name 的裝置（耳機、手環、手錶）會顯示名稱，如 `Mi Smart Band 6`、`Forerunner 935`。
- **RSSI**、出現次數、首見 / 最後出現時間。

## 拿不到什麼

- **一般 iPhone 的精確機型**（如 `iPhone18,2`）— Nearby（`0x10`）廣播不含機型也不含名稱，需連 GATT 或同 Apple ID 才可能。
- **使用者取的名字**（「XXX 的 iPhone / AirPods」）— **不在任何廣播裡**。你自己的 Apple 裝置看得到，是因為同 iCloud 帳號同步、或把訊息裡的 Apple ID 雜湊比對通訊錄、或已配對；第三方被動掃描無法還原。
- **MAC 位址** — macOS 的 CoreBluetooth 只給會輪替的 UUID（`address_type=uuid`）；Linux / BlueZ 才有 raw MAC（`address_type=mac`）。

## 安裝與執行（macOS）

```bash
./scripts/package.sh          # cargo build --release + 組 .app + 簽章 + 產生 LaunchAgent
```

> **需要 Rust 1.85+**（部分相依如 `clap_lex`、`hashlink` 需要 `edition2024`）。

### ⚠️ macOS 藍牙權限的關鍵細節

macOS 用 **responsible process** 決定藍牙授權。若從別的 GUI app（例如某個終端機或 IDE）生出子行程去掃描，授權會被歸責到那個 app；那個 app 沒宣告藍牙用途時，CoreBluetooth 會直接 **SIGABRT**。

解法：由 **launchd** 啟動打包好的 `.app`，讓它成為自己的 responsible process，套用 bundle 內的 `NSBluetoothAlwaysUsageDescription`。

**背景長駐（建議，開機自動跑）：**

```bash
cp "dist/com.blezer.agent.plist" ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.blezer.agent.plist
# 首次執行會跳出藍牙權限對話框 → 按「允許」
```

Dashboard：<http://127.0.0.1:8080/>　日誌：`/tmp/blezer.out.log`

停用：

```bash
launchctl unload ~/Library/LaunchAgents/com.blezer.agent.plist
```

**只跑一次：**

```bash
open "/Applications/Blezer.app" --args run --port 8080
```

**從自己的 Terminal 直接跑**也可以（Terminal 會成為 responsible process，首次授權後即可）：

```bash
cargo run --release -- run
```

> DB 預設寫到 `~/Library/Application Support/Blezer/blezer.db`（自動建立）。
> 要自訂位置才需要 `--db`；launchd / `open` 的工作目錄是 `/`，所以自訂時一定要用絕對路徑。

## CLI

```bash
blezer run     [--port 8080] [--db PATH]      # 掃描 + dashboard，長駐（--db 預設 App Support）
blezer devices [--window 300]                 # 終端列出近期裝置
blezer stats   [--window 3600]                # 終端印出群體普查
```

## Web API

| 端點 | 說明 |
|---|---|
| `GET /` | Dashboard（每 2 秒輪詢下列 API） |
| `GET /api/devices?window=<secs>` | 近期出現的裝置 |
| `GET /api/devices/{id}?window=<secs>` | 單一裝置的 RSSI 時序 |
| `GET /api/stats?window=<secs>` | 群體普查（裝置數、觀測數、RSSI、廠牌分布） |

## 資料庫

SQLite（WAL）。`devices` 保存彙整後的裝置；`observations` 保存每筆觀測，含 `raw_mfg_data` 原始廣播 payload — 刻意保留，供日後做「廣播指紋 / 行為指紋」re-identification。

## Linux / Raspberry Pi

`btleplug` 跨平台（改用 BlueZ），`cargo build --release` 即可。Linux 沒有 macOS 的 responsible-process 限制，但需要藍牙權限（通常 `setcap` 或以有權限的使用者執行）；背景長駐用 systemd service。`address_type` 會是 `mac`。

## 建置

```bash
cargo build --release          # 產生 target/release/blezer
cargo test                     # 單元測試（src/ble/parse.rs）
cargo test decodes_airpods     # 以名稱子字串跑單一測試
```

## 致謝

- Apple continuity / proximity pairing 機型對照：[furiousMAC/continuity](https://github.com/furiousMAC/continuity)
- README 版面參考：[ts1/BLEUnlock](https://github.com/ts1/BLEUnlock)
