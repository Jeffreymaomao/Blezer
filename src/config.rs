//! 集中管理預設值。CLI 參數會覆蓋這些值。

use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 8080;

/// Blezer 的每位使用者資料目錄(DB、設定檔都放這)。
/// macOS:`~/Library/Application Support/Blezer`;其他平台:`<data_dir>/Blezer`。
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Blezer")
}

/// 未指定 `--db` 時的預設 DB 位置(一定可寫、符合 OS 慣例)。
/// 之所以不用相對路徑:launchd 的 cwd 是 `/`,相對路徑會落到 `/blezer.db`(寫不進去)。
pub fn default_db_path() -> PathBuf {
    data_dir().join("blezer.db")
}

/// 同一裝置最短寫入間隔(秒)。DeviceUpdated 事件很密集,
/// 用這個節流避免 observations 表爆量。
pub const OBSERVATION_THROTTLE_SECS: u64 = 2;

/// devices/stats 子命令與 API 的預設時間視窗(秒)。
pub const DEFAULT_DEVICE_WINDOW_SECS: i64 = 300;
pub const DEFAULT_STATS_WINDOW_SECS: i64 = 3600;
