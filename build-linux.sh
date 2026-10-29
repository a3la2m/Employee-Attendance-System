#!/usr/bin/env bash
# بناء نسخة لينكس (الإصدار) — يبني الواجهة ثم يدمجها داخل الملف التنفيذي.
# مهم: خاصية custom-protocol إلزامية، وبدونها يحاول التطبيق الاتصال بـ localhost:1420
# وتظهر رسالة "Could not connect to localhost: Connection refused".
set -e
cd "$(dirname "$0")"

echo "==> بناء الواجهة (npm run build)"
npm run build

echo "==> بناء الملف التنفيذي (cargo build --release --features custom-protocol)"
# إيقاف أي نسخة تعمل حتى لا يفشل الاستبدال
pkill -f "target/release/employee-attendance-system" 2>/dev/null || true
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml

echo "==> تم: src-tauri/target/release/employee-attendance-system"
