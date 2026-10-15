#!/usr/bin/env bash
# تشغيل نظام الحضور والرواتب على لينكس — بدون أي صلاحيات مدير.
# استخدمه مباشرة: ./run-linux.sh
set -e
cd "$(dirname "$0")"
exec ./src-tauri/target/release/employee-attendance-system "$@"
