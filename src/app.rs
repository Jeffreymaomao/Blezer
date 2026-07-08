//! macOS 選單列(menu bar)app —— Finder 雙擊 .app 時的預設模式。
//!
//! 仿 Ollama:雙擊後不開視窗;預設只在選單列出現一個圖示(不進 Dock),
//! 背景執行緒跑掃描 + web server,點圖示可「打開 Dashboard / 顯示 Dock 圖示 / 結束」。
//!
//! 執行緒分工:
//! - **主執行緒**:tao 事件迴圈 + 托盤圖示(macOS 的 NSApplication 必須在主執行緒跑)。
//! - **背景執行緒**:自己的 tokio runtime,跑 [`crate::run`](掃描 + Axum server)。

use anyhow::Result;
use std::path::PathBuf;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS, EventLoopWindowTargetExtMacOS};
use tracing::{error, info};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

/// 啟動選單列 app。此函式不會返回(事件迴圈接管主執行緒)。
// tray 採 tray-icon 慣用的「Init 事件才建立」寫法:初值 None 會在讀取前被覆寫。
#[allow(unused_assignments)]
pub fn run_menubar(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/");

    // 背景執行緒:各自的 tokio runtime 跑掃描 + web server。
    let db_path = crate::resolve_db(None)?;
    std::thread::Builder::new()
        .name("blezer-server".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    error!("建立 runtime 失敗:{e}");
                    return;
                }
            };
            if let Err(e) = rt.block_on(crate::run("127.0.0.1".into(), port, db_path, false)) {
                error!("server 結束:{e}");
            }
        })?;

    // 主執行緒:macOS 事件迴圈 + 選單列。EventLoop::new() 會初始化 NSApplication。
    let mut event_loop = EventLoop::new();
    // 明確定初始 activation policy —— tao 預設是 Regular(會進 Dock),
    // 這裡依使用者設定覆寫:預設 Accessory(只在選單列)。
    let show_dock = load_show_dock();
    event_loop.set_activation_policy(policy_for(show_dock));

    let menu = Menu::new();
    let open_item = MenuItem::new("打開 Dashboard", true, None);
    let dock_item = CheckMenuItem::new("顯示 Dock 圖示", true, show_dock, None);
    let quit_item = MenuItem::new("結束 Blezer", true, None);
    menu.append(&open_item)?;
    menu.append(&dock_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let menu_channel = MenuEvent::receiver();
    // 托盤圖示要在事件迴圈啟動後、於主執行緒建立;先放著,到 Init 事件才 build。
    let mut tray = None;

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::NewEvents(StartCause::Init) = event {
            tray = Some(
                TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_tooltip("Blezer")
                    .with_icon(app_icon())
                    .with_icon_as_template(true) // 黑色剪影 → 隨深/淺色選單列自動反轉
                    .build()
                    .expect("建立選單列圖示失敗"),
            );
            // tray 必須持續存活,否則圖示會消失;這行只是讓編譯器知道它有被讀取。
            let _ = tray.as_ref();
            info!("選單列已啟動;Dashboard:{url}");
            open_dashboard(&url); // 啟動時自動開一次瀏覽器
        }

        if let Ok(ev) = menu_channel.try_recv() {
            if ev.id == *open_item.id() {
                open_dashboard(&url);
            } else if ev.id == *dock_item.id() {
                // 勾選狀態已由 muda 自動翻轉;讀取後即時套用 + 存檔。
                let show = dock_item.is_checked();
                event_loop.set_activation_policy_at_runtime(policy_for(show));
                save_show_dock(show);
            } else if ev.id == *quit_item.id() {
                // 直接結束整個程序;WAL 模式下背景 DB 寫入是 crash-safe。
                std::process::exit(0);
            }
        }
    });
}

/// show_dock → macOS activation policy。true 進 Dock(Regular),false 只在選單列(Accessory)。
fn policy_for(show_dock: bool) -> ActivationPolicy {
    if show_dock {
        ActivationPolicy::Regular
    } else {
        ActivationPolicy::Accessory
    }
}

/// 用系統預設瀏覽器開 dashboard。
fn open_dashboard(url: &str) {
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        error!("開啟瀏覽器失敗:{e}");
    }
}

// ── 設定持久化(只存一個 show_dock 旗標)──────────────────────────────

fn settings_file() -> PathBuf {
    crate::config::data_dir().join("settings.json")
}

/// 讀取「顯示 Dock 圖示」設定。讀不到 / 壞掉時預設 false(只在選單列)。
fn load_show_dock() -> bool {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("show_dock").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

fn save_show_dock(show_dock: bool) {
    let _ = std::fs::create_dir_all(crate::config::data_dir());
    let body = serde_json::json!({ "show_dock": show_dock }).to_string();
    if let Err(e) = std::fs::write(settings_file(), body) {
        error!("寫入設定失敗:{e}");
    }
}

/// 選單列圖示 —— 內嵌 `assets/menu-icon.png`(黑色剪影)並解碼成 RGBA。
/// 以 `include_bytes!` 編進 binary,維持單一檔案、不外掛資源。
/// 搭配 `with_icon_as_template(true)`:template 模式只看 alpha,顏色由系統依
/// 選單列深/淺色決定,所以這張圖會自動反白。
fn app_icon() -> Icon {
    let bytes = include_bytes!("../assets/menu-icon.png");
    let img = image::load_from_memory(bytes)
        .expect("解碼 menu-icon.png 失敗")
        .to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).expect("建立圖示失敗")
}
