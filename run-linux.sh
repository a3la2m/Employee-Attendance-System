#!/usr/bin/env bash
# تشغيل نظام الحضور والرواتب على لينكس — بدون أي صلاحيات مدير.
# استخدمه مباشرة: ./run-linux.sh
set -e
cd "$(dirname "$0")"

BIN=./src-tauri/target/release/employee-attendance-system

# إذا لم تكن النسخة مبنية بعد، ابنِها بالطريقة الصحيحة (مع custom-protocol)
if [ ! -x "$BIN" ]; then
  echo "الملف التنفيذي غير موجود — جاري البناء أول مرة..."
  exec ./build-linux.sh
fi

exec "$BIN" "$@"
