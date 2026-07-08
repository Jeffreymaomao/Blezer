//! 廣播解析:company id → 廠牌、以及 best-effort 機型推估。
//!
//! 廠牌從 manufacturer data 的 SIG company id 解出,可靠。
//! 機型字串是不確定區:掃描階段多半只拿得到「類別」而非精確型號
//! (如 iPhone18,2)。精確型號通常要連 GATT 讀 Device Information
//! Service,不在 v1 主線。這裡盡力給出有意義的字串,拿不到就回 None。

/// 常見 BLE 廠商的 SIG company identifier 小對照表。
/// 完整清單有數千筆;這裡只放常見的,查不到由呼叫端顯示 0xXXXX。
pub fn vendor_name(company_id: u16) -> Option<&'static str> {
    let name = match company_id {
        0x004C => "Apple",
        0x0006 => "Microsoft",
        0x0075 => "Samsung",
        0x00E0 => "Google",
        0x0087 => "Garmin",
        0x0157 => "Huawei",
        0x038F => "Xiaomi",
        0x0499 => "Ruuvi",
        0x0059 => "Nordic Semiconductor",
        0x02E5 => "Espressif",
        0x0001 => "Ericsson",
        0x000D => "Texas Instruments",
        0x004F => "APT (soundcore)",
        0x0110 => "Nippon Seiki",
        _ => return None,
    };
    Some(name)
}

/// 從 (company_id, payload) 盡力推估機型 / 裝置類別字串。
///
/// Apple(0x004C)的 manufacturer data 是一連串 TLV:[type][len][data...]。
/// 常見 type:0x02=iBeacon、0x07=proximity pairing(AirPods 等)、
/// 0x0C=Handoff、0x10=Nearby、0x12=Find My。單一廣播可能串接多筆
/// Continuity 訊息,所以解析時必須走完整包再依資訊量挑最佳結果。
pub fn best_effort_model(company_id: u16, payload: &[u8]) -> Option<String> {
    match company_id {
        0x004C => apple_decode(payload),
        _ => None,
    }
}

/// 此 payload 是否帶 Find My rolling identity。這類識別碼會輪替;macOS 又不提供
/// 廣播 MAC,所以 CoreBluetooth UUID 可能隨輪替變成新的裝置列。
pub fn has_rotating_find_my_identity(company_id: u16, payload: &[u8]) -> bool {
    company_id == 0x004C
        && apple_tlvs(payload)
            .any(|(t, declared_len, _)| t == 0x12 && matches!(declared_len, 0x02 | 0x19))
}

/// 走訪 Apple 的 TLV,回報最能辨識的內容。
///
/// 重點:音訊裝置(AirPods / Beats)的 proximity pairing(type 0x07)是**明文**,
/// 含精確機型;iPhone/iPad 的 Nearby(0x10)則不含機型也不含名稱。
/// 使用者取的名字(「XXX 的 AirPods」)**不在任何廣播裡**,無法被動掃到。
fn apple_decode(payload: &[u8]) -> Option<String> {
    let mut fallback = None;
    for (t, declared_len, val) in apple_tlvs(payload) {
        // 0x07 proximity pairing:val[0]=版本/prefix,val[1..3]=Apple Bluetooth PID
        // (wire order 為 little-endian)。它比同一包裡的 Find My 狀態更具體,優先回傳。
        if t == 0x07 && val.len() >= 3 {
            let pid = u16::from_le_bytes([val[1], val[2]]);
            let name = audio_model(pid)
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("Apple/Beats 音訊裝置 (PID 0x{pid:04X})"));
            return Some(airpods_details(&name, val));
        }

        let label = match t {
            0x02 => Some("Apple iBeacon"),
            0x05 => Some("Apple AirDrop"),
            0x09 => Some("Apple AirPlay"),
            0x0C => Some("Apple Handoff"),
            0x0D => Some("Apple 個人熱點"),
            0x0E | 0x0F => Some("Apple Nearby Action"),
            0x10 => Some("Apple Nearby (iPhone/iPad)"),
            0x12 => {
                if let Some(decoded) = decode_find_my(declared_len, val) {
                    fallback = Some(decoded);
                }
                None
            }
            _ => None,
        };
        if fallback.is_none() {
            fallback = label.map(str::to_string);
        }
    }
    fallback
}

/// 寬容走訪 Apple Continuity TLV。截斷的最後一筆仍回傳現有 bytes,但不越界。
fn apple_tlvs(payload: &[u8]) -> impl Iterator<Item = (u8, usize, &[u8])> {
    let mut i = 0usize;
    std::iter::from_fn(move || {
        if i + 1 >= payload.len() {
            return None;
        }
        let t = payload[i];
        let declared_len = payload[i + 1] as usize;
        let start = i + 2;
        let end = (start + declared_len).min(payload.len());
        i = end;
        Some((t, declared_len, &payload[start..end]))
    })
}

/// Find My payload 0x02=附近(只帶部分 key),0x19=分離/離線(帶完整 rolling key)。
/// status:bits 4-5=裝置類型,bits 6-7=電量,bit 2=本輪 key 期間 owner 曾連線。
fn decode_find_my(declared_len: usize, val: &[u8]) -> Option<String> {
    if !matches!(declared_len, 0x02 | 0x19) || val.is_empty() {
        return None;
    }
    let status = val[0];
    let kind = match (status >> 4) & 0x03 {
        0 => "Apple 裝置",
        1 => "AirTag",
        2 => "Find My 配件（授權第三方）",
        3 => "AirPods",
        _ => unreachable!(),
    };
    let battery = match (status >> 6) & 0x03 {
        0 => "充足",
        1 => "中等",
        2 => "偏低",
        3 => "極低",
        _ => unreachable!(),
    };
    let state = if declared_len == 0x02 {
        "附近"
    } else {
        "分離／離線"
    };
    let maintained = if status & 0x04 != 0 {
        " · 15 分鐘內曾連線擁有者"
    } else {
        ""
    };
    Some(format!(
        "{kind} · 尋找「{state}」 · 電量{battery}{maintained}"
    ))
}

/// AirPods / Beats 的 Apple Bluetooth PID 對照。
/// 來源優先順序:macOS CoreTypes 內建型號資料、Apple 支援文件、furiousMAC。
fn audio_model(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x2002 => "AirPods (1st gen)",
        0x200F => "AirPods (2nd gen)",
        0x2013 => "AirPods (3rd gen)",
        0x2019 | 0x201B | 0x201C | 0x201E | 0x2020 => "AirPods 4",
        0x200E => "AirPods Pro (1st gen)",
        0x2014 => "AirPods Pro (2nd gen, Lightning)",
        0x2024 => "AirPods Pro (2nd gen, USB-C)",
        0x2027 => "AirPods Pro 3",
        0x200A => "AirPods Max (Lightning)",
        0x201F => "AirPods Max (USB-C)",
        0x202D => "AirPods Max 2",
        0x2003 => "Powerbeats 3",
        0x200B => "Powerbeats Pro",
        0x200C => "Beats Solo Pro",
        0x200D => "Powerbeats 4",
        0x2010 => "Beats Flex",
        0x2005 => "BeatsX",
        0x2006 => "Beats Solo 3",
        0x2009 => "Beats Studio 3",
        0x2011 => "Beats Studio Buds",
        0x2012 => "Beats Fit Pro",
        0x2016 => "Beats Studio Buds +",
        0x2017 => "Beats Studio Pro",
        0x201A => "Beats Pill 3",
        0x2025 => "Beats Solo 4",
        0x2026 => "Beats Solo Buds",
        0x2600 => "Beats Pill +",
        _ => return None,
    })
}

/// 從 proximity pairing 明文附加左右耳/耳機盒電量與充電狀態(⚡=充電中)。
///
/// 依 furiousMAC dissector 的欄位定義(val = type/len 之後的位元組):
/// - val[4] 電量:高 4 bits=右耳、低 4 bits=左耳(×10%)
/// - val[5] 充電+盒電量:高 4 bits=充電旗標(bit0 右、bit1 左、bit2 盒)、低 4 bits=盒電量(×10%)
/// - nibble 0xF 表未知
///
/// 註:左右耳有時會因「主廣播的那一顆」而對調(依 val[3] 狀態旗標),
/// 這裡照 furiousMAC 直讀不做翻轉,標示以廣播端為準。
fn airpods_details(name: &str, val: &[u8]) -> String {
    let pct = |nib: u8| -> Option<u8> {
        if nib == 0x0F {
            None
        } else {
            Some((nib as u16 * 10).min(100) as u8)
        }
    };
    let batt = val.get(4).copied();
    let right = batt.and_then(|b| pct((b >> 4) & 0x0F));
    let left = batt.and_then(|b| pct(b & 0x0F));

    let charge = val.get(5).copied();
    let case_batt = charge.and_then(|c| pct(c & 0x0F));
    let flags = charge.map(|c| (c >> 4) & 0x0F).unwrap_or(0);
    let (r_chg, l_chg, c_chg) = (flags & 0x1 != 0, flags & 0x2 != 0, flags & 0x4 != 0);

    let mark = |on: bool| if on { "⚡" } else { "" };
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = left {
        parts.push(format!("L{l}%{}", mark(l_chg)));
    }
    if let Some(r) = right {
        parts.push(format!("R{r}%{}", mark(r_chg)));
    }
    if let Some(c) = case_batt {
        parts.push(format!("盒{c}%{}", mark(c_chg)));
    }

    if parts.is_empty() {
        name.to_string()
    } else {
        format!("{name} · {}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_airpods_pro_with_battery() {
        // AirPods Pro proximity pairing:PID bytes 0E 20 => little-endian 0x200E。
        let mut payload = vec![0x07, 0x19, 0x01, 0x0E, 0x20, 0x55, 0x87, 0x01, 0x02, 0x00];
        payload.extend_from_slice(&[0u8; 16]); // 16-byte 加密段
        let m = best_effort_model(0x004C, &payload).unwrap();
        assert!(m.starts_with("AirPods Pro"), "got {m}");
        assert!(m.contains("L70% R80%"), "got {m}"); // 0x87 → 右8=80% 左7=70%
        assert!(m.contains("盒10%"), "got {m}"); // 0x01 低 nibble=1 → 盒 10%
    }

    #[test]
    fn decodes_charging_flags() {
        // battery=0x99(L/R 90%),charge=0x35 → 旗標 0x3(右+左充電)、盒電量 0x5=50%
        let mut payload = vec![0x07, 0x19, 0x01, 0x0E, 0x20, 0x55, 0x99, 0x35, 0x02, 0x00];
        payload.extend_from_slice(&[0u8; 16]);
        let m = best_effort_model(0x004C, &payload).unwrap();
        assert!(m.contains("L90%⚡"), "got {m}");
        assert!(m.contains("R90%⚡"), "got {m}");
        assert!(m.contains("盒50%"), "got {m}");
        assert!(!m.contains("盒50%⚡"), "case not charging: {m}");
    }

    #[test]
    fn iphone_nearby_has_no_model_or_name() {
        // Nearby(0x10):有活動旗標,但不含機型/名稱。
        let payload = vec![0x10, 0x05, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            best_effort_model(0x004C, &payload).as_deref(),
            Some("Apple Nearby (iPhone/iPad)")
        );
    }

    #[test]
    fn prefers_airpods_details_over_find_my_prefix() {
        // 真實擷取:同一 manufacturer payload 先放 Find My nearby,再放 AirPods 4。
        let payload = [
            0x12, 0x02, 0x6E, 0x02, 0x07, 0x11, 0x06, 0x1C, 0x20, 0x0D, 0x39, 0xE4, 0xE4, 0x51,
            0x0A, 0x00, 0x00, 0x00, 0x00, 0x6F, 0x20, 0x07, 0x00,
        ];
        let m = best_effort_model(0x004C, &payload).unwrap();
        assert!(m.starts_with("AirPods 4"), "got {m}");
        assert!(m.contains("L90%"), "got {m}");
        assert!(has_rotating_find_my_identity(0x004C, &payload));
    }

    #[test]
    fn decodes_find_my_nearby_status() {
        let maintained = best_effort_model(0x004C, &[0x12, 0x02, 0x24, 0x03]).unwrap();
        assert!(maintained.contains("授權第三方"), "got {maintained}");
        assert!(maintained.contains("附近"), "got {maintained}");
        assert!(maintained.contains("電量充足"), "got {maintained}");
        assert!(
            maintained.contains("15 分鐘內曾連線擁有者"),
            "got {maintained}"
        );

        let apple = best_effort_model(0x004C, &[0x12, 0x02, 0x00, 0x01]).unwrap();
        assert!(apple.starts_with("Apple 裝置"), "got {apple}");
    }

    #[test]
    fn decodes_find_my_separated_airpods() {
        let mut payload = vec![0x12, 0x19, 0x79]; // AirPods + medium battery + maintained
        payload.extend_from_slice(&[0xAB; 22]);
        payload.extend_from_slice(&[0x02, 0x00]); // key overflow + hint
        let m = best_effort_model(0x004C, &payload).unwrap();
        assert!(m.starts_with("AirPods"), "got {m}");
        assert!(m.contains("分離／離線"), "got {m}");
        assert!(m.contains("電量中等"), "got {m}");
    }

    #[test]
    fn truncated_apple_tlv_is_safe() {
        assert_eq!(
            best_effort_model(0x004C, &[0x12, 0x19, 0x10]).as_deref(),
            Some("AirTag · 尋找「分離／離線」 · 電量充足")
        );
        assert_eq!(best_effort_model(0x004C, &[0x07]), None);
    }

    #[test]
    fn vendor_lookup() {
        assert_eq!(vendor_name(0x004C), Some("Apple"));
        assert_eq!(vendor_name(0xFFFF), None);
    }
}

/// 取廣播中「第一筆」manufacturer data 作為代表(company_id, payload)。
pub fn primary_manufacturer<'a>(
    mfg: &'a std::collections::HashMap<u16, Vec<u8>>,
) -> Option<(u16, &'a [u8])> {
    // HashMap 無序;若有多筆優先取 Apple,否則取任一筆,結果較穩定。
    if let Some(v) = mfg.get(&0x004C) {
        return Some((0x004C, v.as_slice()));
    }
    mfg.iter().next().map(|(k, v)| (*k, v.as_slice()))
}
