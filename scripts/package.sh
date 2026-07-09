#!/usr/bin/env bash
# 建置 release binary、組成 macOS .app bundle 並 ad-hoc 簽章。
# 為什麼要 .app + launchd:macOS 用「responsible process」判斷藍牙權限。
# 由 launchd 啟動的 .app 是自己負責,才會套用 bundle 內的
# NSBluetoothAlwaysUsageDescription;直接從別的 GUI app 生出來的子行程會被
# 歸責到那個 app 而 SIGABRT。詳見 README。
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
# app 一律裝到 Documents/Desktop/Downloads 以外的地方,否則 macOS 會因為
# 「app 位在受保護資料夾內」而跳出檔案／資料夾權限提示(與 app 是否讀你的
# 檔案無關)。預設 /Applications;可用 BLE_INSTALL_DIR 覆蓋成 ~/Applications 等。
INSTALL_DIR="${BLE_INSTALL_DIR:-/Applications}"
APP="$INSTALL_DIR/Blezer.app"
mkdir -p "$ROOT/dist" "$INSTALL_DIR"

echo "==> cargo build --release"
cargo build --release

echo "==> 組裝 $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$ROOT/Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/target/release/blezer" "$APP/Contents/MacOS/blezer"

# 由 assets/logo.png 產生 macOS app icon(.icns)。需要 macOS 內建的 sips + iconutil。
echo "==> 產生 app icon(Blezer.icns)"
ICONSET="$(mktemp -d)/Blezer.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z $size $size        "$ROOT/assets/logo.png" --out "$ICONSET/icon_${size}x${size}.png"    >/dev/null
    sips -z $((size*2)) $((size*2)) "$ROOT/assets/logo.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Blezer.icns"
rm -rf "$(dirname "$ICONSET")"

echo "==> codesign(ad-hoc)"
codesign --force --sign - "$APP"
codesign -dvvv "$APP" 2>&1 | grep -iE "Info.plist entries|flags" || true

# 產生一份填好絕對路徑、可直接載入的 LaunchAgent。
AGENT="$ROOT/dist/com.blezer.agent.plist"
PORT="${BLE_PORT:-8080}"
# 預設不帶 --db,讓 app 寫到 ~/Library/Application Support/Blezer/blezer.db。
# 設 BLE_DB_PATH 才覆蓋成自訂絕對路徑。
if [ -n "${BLE_DB_PATH:-}" ]; then
    DB_ARG=$'\n''        <string>--db</string><string>'"$BLE_DB_PATH"'</string>'
else
    DB_ARG=""
fi
cat > "$AGENT" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>com.blezer.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>$APP/Contents/MacOS/blezer</string>
        <string>run</string>
        <string>--port</string><string>$PORT</string>$DB_ARG
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>/tmp/blezer.out.log</string>
    <key>StandardErrorPath</key><string>/tmp/blezer.err.log</string>
</dict>
</plist>
PLIST

echo
echo "完成。"
echo "  App:   $APP   (在 Documents 之外,不會再跳「文件資料夾」權限提示)"
echo "  Agent: $AGENT"
echo
echo "背景長駐(開機自動啟動):"
echo "  cp \"$AGENT\" ~/Library/LaunchAgents/"
echo "  launchctl load ~/Library/LaunchAgents/com.blezer.agent.plist"
echo "  # 首次會跳藍牙權限,按『允許』。Dashboard: http://127.0.0.1:$PORT/"
echo
echo "只是想跑一次看看:"
echo "  open \"$APP\" --args run --port $PORT"
echo "  # DB 預設寫到 ~/Library/Application Support/Blezer/blezer.db"
