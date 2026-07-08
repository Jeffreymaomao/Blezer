//! 跨模組共用的資料結構。

use serde::Serialize;
use std::collections::BTreeMap;

/// Scanner 正規化後、準備寫入 DB 的單筆觀測。
#[derive(Debug, Clone)]
pub struct Observation {
    /// 裝置識別碼。macOS 是 CoreBluetooth UUID(會輪替);Linux 是 MAC。
    pub device_id: String,
    /// 'uuid'(macOS 語意)或 'mac'(Linux 語意)。
    pub address_type: &'static str,
    pub ts: i64,
    pub rssi: Option<i16>,
    pub tx_power: Option<i16>,
    /// manufacturer data 的 SIG company id(廣播中第一筆)。
    pub company_id: Option<u16>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub local_name: Option<String>,
    pub service_uuids: Vec<String>,
    /// 廣播中的 Service Data(UUID → raw payload hex)。macOS/CoreBluetooth 可取得時保留。
    pub service_data: BTreeMap<String, String>,
    /// 原始 manufacturer data payload,保留供日後指紋分析。
    pub raw_mfg_data: Option<Vec<u8>>,
}

/// `/api/devices` 回傳的單筆裝置。
#[derive(Debug, Serialize)]
pub struct DeviceJson {
    pub id: String,
    pub address_type: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub local_name: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub seen_count: i64,
    pub last_rssi: Option<i16>,
    /// 最近一筆帶 manufacturer payload 的 company id / raw evidence。
    pub company_id: Option<u16>,
    pub raw_mfg_hex: Option<String>,
    pub service_uuids: Vec<String>,
    pub service_data: BTreeMap<String, String>,
    /// Find My rolling identity；在 macOS 上可能因輪替而顯示成多筆 UUID。
    pub rotating_id: bool,
}

/// `/api/devices/{id}` 回傳的單一 RSSI 時序點。
#[derive(Debug, Serialize)]
pub struct ObsPoint {
    pub ts: i64,
    pub rssi: Option<i16>,
}

/// `/api/stats` 回傳的群體普查。
#[derive(Debug, Serialize)]
pub struct Stats {
    pub window_secs: i64,
    pub device_count: i64,
    pub observation_count: i64,
    pub rssi_min: Option<i32>,
    pub rssi_max: Option<i32>,
    pub rssi_avg: Option<f64>,
    pub by_vendor: Vec<VendorCount>,
}

#[derive(Debug, Serialize)]
pub struct VendorCount {
    pub vendor: String,
    pub count: i64,
}
