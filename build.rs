//! macOS 專用:把 Info.plist 內嵌進 Mach-O 的 __TEXT,__info_plist section。
//!
//! CoreBluetooth 在存取藍牙前會檢查 NSBluetoothAlwaysUsageDescription;
//! 對於非 .app bundle 的 CLI 執行檔,macOS 會讀取這個內嵌 section。
//! 沒有它,首次存取藍牙會直接 SIGABRT(TCC)。這讓我們維持「單一執行檔」。

fn main() {
    #[cfg(target_os = "macos")]
    {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let plist = format!("{manifest_dir}/Info.plist");
        println!("cargo:rerun-if-changed=Info.plist");
        // 只套用到 binary 產物,section 名稱為 CoreBluetooth 會讀取的 __info_plist。
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{plist}"
        );
    }
}
