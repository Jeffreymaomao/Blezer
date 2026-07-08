//! Blezer — 掃描附近 BLE 廣播、寫入 SQLite、並提供內建 Web dashboard。

mod ble;
mod config;
mod db;
mod model;
mod web;
#[cfg(target_os = "macos")]
mod app;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::{DEFAULT_PORT, DEFAULT_DEVICE_WINDOW_SECS, DEFAULT_STATS_WINDOW_SECS};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "blezer", version, about = "BLE 裝置偵測與分析平台")]
struct Cli {
    /// 省略子命令時進入選單列 app 模式(Finder 雙擊 .app 即走這條)。
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 掃描 + Web dashboard,長駐執行。
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// SQLite 路徑。省略時用預設(macOS:~/Library/Application Support/Blezer/blezer.db)。
        #[arg(long)]
        db: Option<String>,
        /// 只跑 web(不掃描)。用於檢視既有 DB / 在無藍牙授權的環境預覽 dashboard。
        #[arg(long)]
        no_scan: bool,
        /// Web server 綁定位址。預設 127.0.0.1(只限本機);預覽/容器可用 0.0.0.0。
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// 終端列出近期出現的裝置。
    Devices {
        #[arg(long, default_value_t = DEFAULT_DEVICE_WINDOW_SECS)]
        window: i64,
        /// SQLite 路徑。省略時用預設(macOS:~/Library/Application Support/Blezer/blezer.db)。
        #[arg(long)]
        db: Option<String>,
    },
    /// 終端印出群體普查統計。
    Stats {
        #[arg(long, default_value_t = DEFAULT_STATS_WINDOW_SECS)]
        window: i64,
        /// SQLite 路徑。省略時用預設(macOS:~/Library/Application Support/Blezer/blezer.db)。
        #[arg(long)]
        db: Option<String>,
    },
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 把 `--db`(可省略)解析成實際路徑,並確保父目錄存在。
/// 省略時落在 [`config::default_db_path`](使用者 data 目錄下,一定可寫)。
fn resolve_db(db: Option<String>) -> Result<String> {
    let path = db.map(PathBuf::from).unwrap_or_else(config::default_db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path.to_string_lossy().into_owned())
}

/// 建一個 multi-thread tokio runtime。main 不再是 async(選單列模式要把主執行緒
/// 讓給 macOS 事件迴圈),所以各非同步進入點自行 `block_on`。
fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread().enable_all().build()?)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "blezer=info".into()),
        )
        .init();

    // macOS 由 Finder/LaunchServices 雙擊啟動時可能塞進 `-psn_0_xxxxx`,
    // 過濾掉以免 clap 把它當未知參數而報錯退出。
    let args = std::env::args().filter(|a| !a.starts_with("-psn_"));
    let cli = Cli::parse_from(args);

    match cli.command {
        Some(Command::Run { port, db, no_scan, host }) => {
            runtime()?.block_on(run(host, port, resolve_db(db)?, no_scan))
        }
        Some(Command::Devices { window, db }) => list_devices(window, resolve_db(db)?),
        Some(Command::Stats { window, db }) => print_stats(window, resolve_db(db)?),
        // 無子命令(Finder 雙擊 .app)→ 選單列 app。
        None => run_default(),
    }
}

/// 無子命令時的預設行為。macOS:選單列 app;其他平台:退回長駐掃描 + web。
#[cfg(target_os = "macos")]
fn run_default() -> Result<()> {
    app::run_menubar(DEFAULT_PORT)
}

#[cfg(not(target_os = "macos"))]
fn run_default() -> Result<()> {
    runtime()?.block_on(run("127.0.0.1".into(), DEFAULT_PORT, resolve_db(None)?, false))
}

/// 長駐:掃描 task + web server 並行(--no-scan 時只跑 web)。
async fn run(host: String, port: u16, db_path: String, no_scan: bool) -> Result<()> {
    let db = db::open(&db_path)?;
    info!("DB:{db_path}");

    if no_scan {
        info!("web-only 模式(--no-scan):不啟動掃描,僅提供 dashboard 與歷史查詢");
    } else {
        // 掃描在背景 task 跑;掃描出錯時記錄但不讓整個程序倒掉。
        let scan_db = db.clone();
        tokio::spawn(async move {
            if let Err(e) = ble::scanner::run(scan_db).await {
                error!("掃描器結束:{e}(請確認已授權 Bluetooth 權限)");
            }
        });
    }

    web::serve(db, &host, port).await
}

fn list_devices(window: i64, db_path: String) -> Result<()> {
    let db = db::open(&db_path)?;
    let conn = db.lock().expect("db mutex poisoned");
    let cutoff = now_secs() - window;
    let devices = db::recent_devices(&conn, cutoff)?;
    if devices.is_empty() {
        println!("(近 {window}s 內無裝置)");
        return Ok(());
    }
    println!("{:<5} {:<28} {:<22} RSSI  次數  識別碼", "", "廠牌/機型", "");
    for d in &devices {
        // 名字優先當主標,機型/類別當次要細節。
        let detail = d.model.clone().filter(|m| Some(m) != d.local_name.as_ref());
        let label = match (d.local_name.clone(), detail) {
            (Some(n), Some(m)) => format!("{n} · {m}"),
            (Some(n), None) => n,
            (None, Some(m)) => m,
            (None, None) => d.vendor.clone().unwrap_or_else(|| "(未知)".into()),
        };
        let rssi = d.last_rssi.map(|r| r.to_string()).unwrap_or_else(|| "–".into());
        println!(
            "  {:<48} {:>5} {:>5}  {}",
            truncate(&label, 48),
            rssi,
            d.seen_count,
            d.id
        );
    }
    println!("\n共 {} 台裝置。", devices.len());
    Ok(())
}

fn print_stats(window: i64, db_path: String) -> Result<()> {
    let db = db::open(&db_path)?;
    let conn = db.lock().expect("db mutex poisoned");
    let s = db::stats(&conn, window, now_secs())?;
    println!("時間視窗:{} 秒", s.window_secs);
    println!("在場裝置:{}", s.device_count);
    println!("觀測筆數:{}", s.observation_count);
    match (s.rssi_min, s.rssi_max, s.rssi_avg) {
        (Some(mn), Some(mx), Some(avg)) => {
            println!("RSSI:min={mn} max={mx} avg={:.1} dBm", avg)
        }
        _ => println!("RSSI:(無資料)"),
    }
    println!("\n廠牌分布:");
    for v in &s.by_vendor {
        println!("  {:<24} {}", v.vendor, v.count);
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
