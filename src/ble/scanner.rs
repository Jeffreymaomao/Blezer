//! btleplug 掃描迴圈:訂閱事件、讀 properties、正規化成 Observation、寫 DB。

use crate::ble::parse;
use crate::config::OBSERVATION_THROTTLE_SECS;
use crate::db::{self, Db};
use crate::model::Observation;
use anyhow::{Result, anyhow};
use btleplug::api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::Manager;
use futures::stream::StreamExt;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info, warn};

/// macOS 只給 CoreBluetooth UUID;其他平台(Linux/BlueZ)給 MAC。
#[cfg(target_os = "macos")]
const ADDRESS_TYPE: &str = "uuid";
#[cfg(not(target_os = "macos"))]
const ADDRESS_TYPE: &str = "mac";

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 啟動掃描並持續寫入,直到程序結束。
pub async fn run(db: Db) -> Result<()> {
    let manager = Manager::new().await?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("找不到任何 Bluetooth adapter"))?;

    info!(
        "使用 adapter:{:?}",
        adapter.adapter_info().await.unwrap_or_default()
    );
    adapter.start_scan(ScanFilter::default()).await?;
    info!("開始掃描 BLE 廣播(平台識別碼語意:{ADDRESS_TYPE})");

    let mut events = adapter.events().await?;
    // 每裝置最近一次寫入時間,用來節流密集的 DeviceUpdated。
    let mut last_write: HashMap<String, Instant> = HashMap::new();

    while let Some(event) = events.next().await {
        let id = match event {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => id,
            _ => continue,
        };

        let id_str = id.to_string();
        if let Some(t) = last_write.get(&id_str) {
            if t.elapsed().as_secs() < OBSERVATION_THROTTLE_SECS {
                continue;
            }
        }

        let peripheral = match adapter.peripheral(&id).await {
            Ok(p) => p,
            Err(e) => {
                debug!("取得 peripheral {id_str} 失敗:{e}");
                continue;
            }
        };
        let props = match peripheral.properties().await {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                debug!("讀取 {id_str} properties 失敗:{e}");
                continue;
            }
        };

        // Spike/除錯用:完整 dump 一筆廣播內容,方便驗證 macOS 到底給什麼。
        debug!(
            "廣播 {id_str}: name={:?} rssi={:?} tx={:?} mfg={:?} services={:?} service_data={:?}",
            props.local_name,
            props.rssi,
            props.tx_power_level,
            props.manufacturer_data,
            props.services,
            props.service_data
        );

        let (company_id, vendor, model) =
            match parse::primary_manufacturer(&props.manufacturer_data) {
                Some((cid, payload)) => {
                    let vendor = parse::vendor_name(cid)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| format!("0x{cid:04X}"));
                    let model = parse::best_effort_model(cid, payload);
                    (Some(cid), Some(vendor), model)
                }
                None => (None, None, None),
            };

        // local_name 若存在,通常是最有用的「機型/類別」線索(如 AirPods、耳機)。
        let model = model.or_else(|| props.local_name.clone());

        let raw_mfg = parse::primary_manufacturer(&props.manufacturer_data)
            .map(|(_, payload)| payload.to_vec());

        let service_uuids = props
            .services
            .iter()
            .chain(props.service_data.keys())
            .map(|u| u.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let service_data = props
            .service_data
            .iter()
            .map(|(uuid, bytes)| {
                (
                    uuid.to_string(),
                    bytes.iter().map(|b| format!("{b:02X}")).collect(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let obs = Observation {
            device_id: id_str.clone(),
            address_type: ADDRESS_TYPE,
            ts: now_secs(),
            // 127(0x7F)是「RSSI 不可用」的哨兵值,任何正值也非合法 RSSI,一律視為未知。
            rssi: props.rssi.filter(|&r| r <= 0),
            tx_power: props.tx_power_level,
            company_id,
            vendor,
            model,
            local_name: props.local_name.clone(),
            service_uuids,
            service_data,
            raw_mfg_data: raw_mfg,
        };

        // 短暫持鎖寫入,不在持鎖期間 await。
        {
            let conn = db.lock().expect("db mutex poisoned");
            if let Err(e) = db::record_observation(&conn, &obs) {
                warn!("寫入 observation 失敗:{e}");
            }
        }
        last_write.insert(id_str, Instant::now());
    }

    Ok(())
}
