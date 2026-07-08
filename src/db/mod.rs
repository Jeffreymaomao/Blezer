//! SQLite 存取層(rusqlite,bundled)。單一連線以 Arc<Mutex> 共享。

use crate::ble::parse;
use crate::model::{DeviceJson, ObsPoint, Observation, Stats, VendorCount};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type Db = Arc<Mutex<Connection>>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS devices (
  id            TEXT PRIMARY KEY,
  address_type  TEXT,
  vendor        TEXT,
  model         TEXT,
  local_name    TEXT,
  first_seen    INTEGER NOT NULL,
  last_seen     INTEGER NOT NULL,
  seen_count    INTEGER NOT NULL DEFAULT 1,
  last_rssi     INTEGER
);

CREATE TABLE IF NOT EXISTS observations (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  device_id      TEXT NOT NULL REFERENCES devices(id),
  ts             INTEGER NOT NULL,
  rssi           INTEGER,
  tx_power       INTEGER,
  company_id     INTEGER,
  service_uuids  TEXT,
  service_data   TEXT,
  raw_mfg_data   BLOB
);

CREATE INDEX IF NOT EXISTS idx_obs_ts ON observations(ts);
CREATE INDEX IF NOT EXISTS idx_obs_dev ON observations(device_id);
"#;

/// 開啟(必要時建立)DB,設定 WAL 並套用 schema。
pub fn open(path: impl AsRef<Path>) -> Result<Db> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)?;
    ensure_column(&conn, "observations", "service_data", "TEXT")?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// 現有 DB 的輕量冪等 schema 演進；新 DB 已由 SCHEMA 直接建立完整欄位。
fn ensure_column(conn: &Connection, table: &str, column: &str, declaration: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
        ))?;
    }
    Ok(())
}

/// upsert device + 插入一筆 observation。呼叫端保證不在持鎖時 await。
pub fn record_observation(conn: &Connection, o: &Observation) -> Result<()> {
    conn.execute(
        r#"INSERT INTO devices
             (id, address_type, vendor, model, local_name, first_seen, last_seen, seen_count, last_rssi)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1, ?7)
           ON CONFLICT(id) DO UPDATE SET
             last_seen  = ?6,
             seen_count = seen_count + 1,
             last_rssi  = ?7,
             -- 只在新值非 NULL 時覆蓋,避免偶爾缺欄位把既有資訊清掉
             vendor     = COALESCE(?3, vendor),
             model      = COALESCE(?4, model),
             local_name = COALESCE(?5, local_name)"#,
        params![
            o.device_id,
            o.address_type,
            o.vendor,
            o.model,
            o.local_name,
            o.ts,
            o.rssi,
        ],
    )?;

    let services = serde_json::to_string(&o.service_uuids).unwrap_or_else(|_| "[]".into());
    let service_data = serde_json::to_string(&o.service_data).unwrap_or_else(|_| "{}".into());
    conn.execute(
        r#"INSERT INTO observations
             (device_id, ts, rssi, tx_power, company_id, service_uuids, service_data, raw_mfg_data)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            o.device_id,
            o.ts,
            o.rssi,
            o.tx_power,
            o.company_id,
            services,
            service_data,
            o.raw_mfg_data,
        ],
    )?;
    Ok(())
}

/// last_seen >= cutoff 的裝置,最近出現優先。
pub fn recent_devices(conn: &Connection, cutoff: i64) -> Result<Vec<DeviceJson>> {
    let mut stmt = conn.prepare(
        r#"SELECT d.id, d.address_type, d.vendor, d.model, d.local_name,
                  d.first_seen, d.last_seen, d.seen_count, d.last_rssi,
                  o.company_id, o.raw_mfg_data, o.service_uuids, o.service_data
             FROM devices d
             LEFT JOIN observations o ON o.id = (
                 SELECT newest.id FROM observations newest
                  WHERE newest.device_id = d.id AND newest.raw_mfg_data IS NOT NULL
                  ORDER BY newest.ts DESC, newest.id DESC LIMIT 1
             )
            WHERE d.last_seen >= ?1
            ORDER BY d.last_seen DESC"#,
    )?;
    let rows = stmt
        .query_map(params![cutoff], |r| {
            let stored_model: Option<String> = r.get(3)?;
            let company_id: Option<u16> = r.get(9)?;
            let raw_mfg: Option<Vec<u8>> = r.get(10)?;
            let decoded_model = company_id
                .zip(raw_mfg.as_deref())
                .and_then(|(cid, raw)| parse::best_effort_model(cid, raw));
            let service_uuids_json = r.get::<_, Option<String>>(11)?.unwrap_or_default();
            let service_data_json = r.get::<_, Option<String>>(12)?.unwrap_or_default();
            Ok(DeviceJson {
                id: r.get(0)?,
                address_type: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                vendor: r.get(2)?,
                model: decoded_model.or(stored_model),
                local_name: r.get(4)?,
                first_seen: r.get(5)?,
                last_seen: r.get(6)?,
                seen_count: r.get(7)?,
                last_rssi: r.get(8)?,
                company_id,
                raw_mfg_hex: raw_mfg.as_deref().map(hex_upper),
                service_uuids: serde_json::from_str(&service_uuids_json).unwrap_or_default(),
                service_data: serde_json::from_str(&service_data_json)
                    .unwrap_or_else(|_| BTreeMap::new()),
                rotating_id: company_id
                    .zip(raw_mfg.as_deref())
                    .is_some_and(|(cid, raw)| parse::has_rotating_find_my_identity(cid, raw)),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// 單一裝置在時間視窗內的 RSSI 時序。
pub fn device_history(conn: &Connection, id: &str, cutoff: i64) -> Result<Vec<ObsPoint>> {
    let mut stmt = conn.prepare(
        r#"SELECT ts, rssi FROM observations
            WHERE device_id = ?1 AND ts >= ?2
            ORDER BY ts ASC"#,
    )?;
    let rows = stmt
        .query_map(params![id, cutoff], |r| {
            Ok(ObsPoint {
                ts: r.get(0)?,
                rssi: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 時間視窗內的群體普查統計。
pub fn stats(conn: &Connection, window_secs: i64, now: i64) -> Result<Stats> {
    let cutoff = now - window_secs;

    let device_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM devices WHERE last_seen >= ?1",
        params![cutoff],
        |r| r.get(0),
    )?;

    let observation_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations WHERE ts >= ?1",
        params![cutoff],
        |r| r.get(0),
    )?;

    let (rssi_min, rssi_max, rssi_avg): (Option<i32>, Option<i32>, Option<f64>) = conn.query_row(
        "SELECT MIN(rssi), MAX(rssi), AVG(rssi) FROM observations WHERE ts >= ?1 AND rssi IS NOT NULL",
        params![cutoff],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;

    let mut stmt = conn.prepare(
        r#"SELECT COALESCE(vendor, 'Unknown') AS v, COUNT(*) AS c
             FROM devices
            WHERE last_seen >= ?1
            GROUP BY v
            ORDER BY c DESC"#,
    )?;
    let by_vendor = stmt
        .query_map(params![cutoff], |r| {
            Ok(VendorCount {
                vendor: r.get(0)?,
                count: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Stats {
        window_secs,
        device_count,
        observation_count,
        rssi_min,
        rssi_max,
        rssi_avg,
        by_vendor,
    })
}
