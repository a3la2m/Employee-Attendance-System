//! أوامر Tauri — الجسر بين React (frontend) ومنطق Rust (backend).
use crate::attendance::{check_device_clock, compute_days_for_employee, pair_punches};
use crate::db;
use crate::models::*;
use crate::payroll::compute_payroll;
use crate::storage;
use crate::zk::make_backend;
use chrono::{Datelike, NaiveDate};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// حالة التطبيق المشتركة: قاعدة البيانات + مجلد الإعدادات الأساسي.
pub struct AppState {
    pub db: Mutex<Connection>,
    /// مجلد إعدادات التطبيق الثابت (يحوي storage-config.json).
    pub base: PathBuf,
}

fn settings_map(conn: &Connection) -> HashMap<String, String> {
    db::all_settings(conn).unwrap_or_default()
}

fn setting(s: &HashMap<String, String>, k: &str, def: &str) -> String {
    s.get(k).cloned().unwrap_or_else(|| def.to_string())
}

fn setting_f(s: &HashMap<String, String>, k: &str, def: f64) -> f64 {
    s.get(k).and_then(|v| v.parse().ok()).unwrap_or(def)
}

fn setting_i(s: &HashMap<String, String>, k: &str, def: i64) -> i64 {
    s.get(k).and_then(|v| v.parse().ok()).unwrap_or(def)
}

/// يبني كائن PayrollSettings من الإعدادات المخزّنة.
pub fn payroll_settings(s: &HashMap<String, String>) -> PayrollSettings {
    PayrollSettings {
        daily_hours: setting_f(s, "daily_hours", 8.5),
        break_min: setting_i(s, "break_min", 30),
        // زر «لا تخصم الاستراحة» — الافتراضي مفعّل (يُخصم 30 د دائماً)
        deduct_break: s.get("deduct_break").map(|v| v != "0").unwrap_or(true),
        hourly_rate: setting_f(s, "hourly_rate", 10.0),
        meal_allow: setting_f(s, "meal_allow", 7.0),
        transport_allow: setting_f(s, "transport_allow", 5.0),
        overtime_after_hours: setting_f(s, "overtime_after_hours", 8.5),
        overtime_rate_mult: setting_f(s, "overtime_rate_mult", 1.5),
        grace_late_min: setting_i(s, "grace_late_min", 0),
        weekend_allow: setting_f(s, "weekend_allow", 62.0),
        weekend_pay_mode: setting(s, "weekend_pay_mode", "weekly"),
        work_start: setting(s, "work_start", "09:00"),
        work_end: setting(s, "work_end", "17:30"),
    }
}

/// تحقّق أن النص صيغة وقت HH:MM ويعيد (ساعة، دقيقة).
fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    if h < 24 && m < 60 { Some((h, m)) } else { None }
}

/// يطبّع قيمة أي زمن إلى HH:MM صريحة (يقبل "9:5" و"09:05:33").
fn normalize_time(v: &str) -> String {
    match parse_hhmm(v) {
        Some((h, m)) => format!("{h:02}:{m:02}"),
        None => v.trim().to_string(),
    }
}

// ---------------------------------------------------------------- الإعدادات

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    settings_map(&conn).pipe(Ok)
}

/// مفاتيح القيم الزمنية التي تُطبَّع تلقائياً إلى صيغة HH:MM عند الحفظ.
fn is_time_key(k: &str) -> bool {
    matches!(k, "work_start" | "work_end" | "late_notify_time")
}

/// يحفظ/يحدّث إعداداً واحداً (القيم الزمنية تُطبَّع إلى HH:MM).
#[tauri::command]
pub fn save_setting(
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let value = if is_time_key(&key) { normalize_time(&value) } else { value };
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::set_setting(&conn, &key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings_bulk(
    state: tauri::State<'_, AppState>,
    items: HashMap<String, String>,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    for (k, v) in items {
        let v = if is_time_key(&k) { normalize_time(&v) } else { v };
        db::set_setting(&conn, &k, &v).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ------------------------------------------------------- التخزين والنسخ الاحتياطي

/// يعيد معلومات مكان التخزين الحالي (مجلد + مسارات الملفات الإنجليزية).
#[tauri::command]
pub fn get_storage_info(state: tauri::State<'_, AppState>) -> Result<storage::StorageInfo, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let db_path = PathBuf::from(conn.path().ok_or("قاعدة البيانات بلا مسار")?);
    drop(conn);
    Ok(storage::StorageInfo::new(&state.base, db_path))
}

/// يغيّر مجلد التخزين (مثال: مجلد Google Drive) ويعيد فتح القاعدة فيه.
/// يكتب المؤشّر في storage-config.json وينتقل فعلياً بلا إعادة تشغيل.
#[tauri::command]
pub fn set_storage_dir(
    state: tauri::State<'_, AppState>,
    dir: String,
) -> Result<storage::StorageInfo, String> {
    let dir_trim = dir.trim().to_string();
    if dir_trim.is_empty() {
        return Err("مسار المجلد فارغ".into());
    }
    let path = PathBuf::from(&dir_trim);
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| format!("تعذّر إنشاء المجلد: {}", e))?;
    }
    if !path.is_dir() {
        return Err("المسار المختار ليس مجلداً".into());
    }
    // احفظ المؤشّر أولاً
    storage::save(
        &state.base,
        &storage::StorageConfig {
            storage_dir: Some(dir_trim),
        },
    )?;
    // أعد فتح القاعدة في المجلد الجديد (تحت القفل، آمن)
    let new_db = path.join(storage::MAIN_DB);
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let already_open = conn
        .path()
        .map(|p| PathBuf::from(p) == new_db)
        .unwrap_or(false);
    drop(conn);
    if !already_open {
        let mut conn = state.db.lock().map_err(|e| e.to_string())?;
        *conn = db::open(&new_db).map_err(|e| e.to_string())?;
    }
    get_storage_info(state)
}

/// نسخة احتياطية متكاملة في مجلد التخزين النشط:
/// - `attendance.db` → لقطة `attendance-backup-YYYYMMDD-HHMMSS.db` (VACUUM INTO = ثابتة وآمنة)
/// - `attendance-settings.json` → كل الإعدادات بصيغة JSON منظّمة
#[tauri::command]
pub fn backup_now(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let base = &state.base;
    let active = storage::resolve_dir(base);
    std::fs::create_dir_all(&active).map_err(|e| e.to_string())?;

    // 1) لقطة قاعدة البيانات (VACUUM INTO ينتج نسخة متّسقة حتى مع نشاط مستمر)
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let db_dest = active.join(format!("attendance-backup-{}.db", stamp));
    conn.execute("VACUUM INTO ?1", [db_dest.display().to_string()])
        .map_err(|e| format!("فشل نسخ القاعدة: {}", e))?;

    // 2) تصدير الإعدادات إلى JSON منظّم
    let settings = db::all_settings(&conn).unwrap_or_default();
    let doc = json!({
        "app": "Employee-Attendance-System",
        "exported_at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "storage_dir": active.display().to_string(),
        "settings": settings,
    });
    let json_dest = active.join(storage::SETTINGS_JSON);
    let pretty = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&json_dest, pretty).map_err(|e| e.to_string())?;

    Ok(json!({
        "ok": true,
        "db_backup": db_dest.display().to_string(),
        "settings_json": json_dest.display().to_string(),
        "storage_dir": active.display().to_string(),
    }))
}

/// يستعيد الإعدادات من `attendance-settings.json` (يُدمج فوق الإعدادات الحالية).
#[tauri::command]
pub fn restore_settings(state: tauri::State<'_, AppState>, path: Option<String>) -> Result<usize, String> {
    let p = match path {
        Some(p) => PathBuf::from(p),
        None => storage::resolve_dir(&state.base).join(storage::SETTINGS_JSON),
    };
    let raw = std::fs::read_to_string(&p).map_err(|e| format!("تعذّرت قراءة الملف: {}", e))?;
    let doc: Value =
        serde_json::from_str(&raw).map_err(|e| format!("ملف غير صالح: {}", e))?;
    let settings = doc
        .get("settings")
        .and_then(|v| v.as_object())
        .ok_or("الملف لا يحوي قسم settings")?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut n = 0;
    for (k, v) in settings {
        if let Some(val) = v.as_str() {
            db::set_setting(&conn, k, val).map_err(|e| e.to_string())?;
            n += 1;
        }
    }
    Ok(n)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

// ---------------------------------------------------------------- الموظفون

#[tauri::command]
pub fn list_employees(state: tauri::State<'_, AppState>) -> Result<Vec<Employee>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id,pin,name,name_en,job_title,phone,daily_hours,break_min,hourly_rate,
                    meal_allow,transport_allow,active,
                    overtime_enabled,meal_enabled,transport_enabled,weekend_paid,annual_leave_balance,
                    annual_leave_days,leave_accrual_month
             FROM employees ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Employee {
                id: r.get(0)?,
                pin: r.get(1)?,
                name: r.get(2)?,
                name_en: r.get(3)?,
                job_title: r.get(4)?,
                phone: r.get(5)?,
                daily_hours: r.get(6)?,
                break_min: r.get(7)?,
                hourly_rate: r.get(8)?,
                meal_allow: r.get(9)?,
                transport_allow: r.get(10)?,
                active: r.get::<_, i64>(11)? != 0,
                overtime_enabled: r.get::<_, i64>(12)? != 0,
                meal_enabled: r.get::<_, i64>(13)? != 0,
                transport_enabled: r.get::<_, i64>(14)? != 0,
                weekend_paid: r.get::<_, i64>(15)? != 0,
                annual_leave_balance: r.get(16)?,
                annual_leave_days: r.get(17)?,
                leave_accrual_month: r.get(18)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn upsert_employee(state: tauri::State<'_, AppState>, emp: Employee) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    if emp.id > 0 {
        conn.execute(
            "UPDATE employees SET pin=?1,name=?2,job_title=?3,phone=?4,daily_hours=?5,
             break_min=?6,hourly_rate=?7,meal_allow=?8,transport_allow=?9,active=?10,updated_at=?11,
             overtime_enabled=?12,meal_enabled=?13,transport_enabled=?14,name_en=?15,weekend_paid=?16,
             annual_leave_balance=?17,annual_leave_days=?18
             WHERE id=?19",
            rusqlite::params![
                emp.pin,
                emp.name,
                emp.job_title,
                emp.phone,
                emp.daily_hours,
                emp.break_min,
                emp.hourly_rate,
                emp.meal_allow,
                emp.transport_allow,
                emp.active as i64,
                now,
                emp.overtime_enabled as i64,
                emp.meal_enabled as i64,
                emp.transport_enabled as i64,
                emp.name_en,
                emp.weekend_paid as i64,
                emp.annual_leave_balance,
                emp.annual_leave_days,
                emp.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(emp.id)
    } else {
        conn.execute(
            "INSERT INTO employees(pin,name,name_en,job_title,phone,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,overtime_enabled,meal_enabled,transport_enabled,
             weekend_paid,annual_leave_balance,annual_leave_days,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            rusqlite::params![
                emp.pin,
                emp.name,
                emp.name_en,
                emp.job_title,
                emp.phone,
                emp.daily_hours,
                emp.break_min,
                emp.hourly_rate,
                emp.meal_allow,
                emp.transport_allow,
                emp.active as i64,
                emp.overtime_enabled as i64,
                emp.meal_enabled as i64,
                emp.transport_enabled as i64,
                emp.weekend_paid as i64,
                emp.annual_leave_balance,
                emp.annual_leave_days,
                now,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_employee(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM employees WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM punches WHERE employee_id=?1", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM attendance_days WHERE employee_id=?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- الجهاز

#[tauri::command]
pub fn device_info(state: tauri::State<'_, AppState>) -> Result<DeviceInfo, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let s = settings_map(&conn);
    drop(conn);
    let ip = setting(&s, "device_ip", "192.168.1.205");
    let port = setting_i(&s, "device_port", 4370) as u16;
    let key = setting_i(&s, "device_comm_key", 0) as u32;
    let mut b = make_backend(&ip, port, key, true);
    let r = b.connect().map_err(|e| e.to_string());
    b.close();
    r
}

/// سحب سجلات الحضور والموظفين من الجهاز، مزامنتها مع القاعدة، وإعادة احتساب الأيام.
/// نسخة داخلية تعمل من أي مقبض (تُستخدم في الأوامر والمزامنة التلقائية).
/// فحص ساعة الجهاز وتصحيحها تلقائياً إن انحرفت أكثر من 5 دقائق (بوّابة
/// الإعداد auto_fix_clock). تعمل عبر البروتوكول مباشرة لأن قراءة/ضبط
/// الوقت غير متاحة في خلفية DLL. لا تُفشل المزامنة إن تعذّر الاتصال.
fn clock_check_and_fix(ip: &str, port: u16, key: u32, tz: &str, auto_fix: bool) -> Value {
    let mut b = make_backend(ip, port, key, false);
    let dev_t = match b.connect() {
        Ok(_) => match b.get_time_iso() {
            Ok(t) => t,
            Err(e) => {
                b.close();
                return json!({"in_sync": null, "error": e.to_string()});
            }
        },
        Err(e) => return json!({"in_sync": null, "error": e.to_string()}),
    };
    let (local_now, diff) = check_device_clock(&dev_t, tz);
    if diff.abs() <= 5 {
        b.close();
        return json!({"in_sync": true, "diff_min": diff, "device_time": dev_t});
    }
    if auto_fix {
        if b.set_time(&local_now).is_ok() {
            b.close();
            return json!({
                "in_sync": false, "fixed": true, "diff_min": diff,
                "device_time_before": dev_t, "set_to": local_now,
            });
        }
    }
    b.close();
    json!({"in_sync": false, "fixed": false, "diff_min": diff, "device_time": dev_t})
}

pub fn sync_device_internal(state: &AppState) -> Result<Value, String> {
    let (ip, port, key, tz, auto_fix_clock) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
            setting(&s, "timezone", "Asia/Jerusalem"),
            setting(&s, "auto_fix_clock", "1") != "0",
        )
    };
    // الفحص والتصحيح أولاً: بطارية RTC ضعيفة قد تعيد ساعة الجهاز للسنة 2000
    // عند كل قطع كهرباء فتُختم البصمات بتواريخ خاطئة (بصمة موجودة ويظهر غياب).
    let clock = clock_check_and_fix(&ip, port, key, &tz, auto_fix_clock);
    let mut backend = make_backend(&ip, port, key, true);
    let info = backend.connect().map_err(|e| e.to_string())?;
    let users = backend.pull_users().map_err(|e| e.to_string())?;
    let punches = backend.pull_attendance().map_err(|e| e.to_string())?;
    backend.close();

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut synced_punches = 0usize;
    let mut added_users = 0usize;

    // المرحلة الأولى (قصيرة): ربط الأرقام + إضافة مستخدمي الجهاز الجدد —
    // نُطلق القفل بسرعة حتى لا تتجمّد الواجهة أثناء السحب الكبير
    let mut pin_to_id: HashMap<String, i64> = HashMap::new();
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        // اربط pin → employee_id (إن وُجد)
        {
            let mut st = conn
                .prepare("SELECT id, pin FROM employees WHERE pin IS NOT NULL")
                .map_err(|e| e.to_string())?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(0)?)));
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    pin_to_id.insert(row.0, row.1);
                }
            }
        }

        for (pin, name, _card, _priv) in &users {
            if !pin_to_id.contains_key(pin) {
                // موظف على الجهاز غير مسجّل محلياً → أضفه تمهيدياً
                // (بلا اسم على الجهاز؟ نستخدم الرقم الوظيفي كاسم ظاهر)
                let display = if name.trim().is_empty() {
                    pin.clone()
                } else {
                    name.clone()
                };
                conn.execute(
                    "INSERT OR IGNORE INTO employees(pin,name,active,created_at,updated_at)
                     VALUES(?1,?2,1,?3,?3)",
                    rusqlite::params![pin, display, now],
                )
                .map_err(|e| e.to_string())?;
                added_users += 1;
            }
        }
        // حدّث خريطة الربط بعد الإضافات
        if added_users > 0 {
            pin_to_id.clear();
            let mut st = conn
                .prepare("SELECT id, pin FROM employees WHERE pin IS NOT NULL")
                .map_err(|e| e.to_string())?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(0)?)));
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    pin_to_id.insert(row.0, row.1);
                }
            }
        }

        db::set_setting(&conn, "last_sync_at", &now).map_err(|e| e.to_string())?;
    }

    // المرحلة الثانية: إدراج البصمات على دفعات (معاملة واحدة لكل دفعة) مع
    // تحرير القفل بينها — فتبقى الصفحات مستجيبة أثناء سحب آلاف السجلات
    for chunk in punches.chunks(400) {
        let mut conn = state.db.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (pin, dt, punch_type, wc, raw_status) in chunk {
            let emp_id = pin_to_id.get(pin).copied();
            let r = tx.execute(
                "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(pin,device_time) DO NOTHING",
                rusqlite::params![emp_id, pin, dt, punch_type, wc, raw_status, now],
            );
            if let Ok(n) = r {
                synced_punches += n;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    // إعادة احتساب أيام الحضور لآخر 60 يوماً (سريعة وكفيلة)
    let recomputed = recompute_recent(state, 60)?;

    Ok(json!({
        "device": info,
        "new_users_added": added_users,
        "new_punches": synced_punches,
        "days_recomputed": recomputed,
        "last_sync_at": now,
        "clock": clock,
    }))
}

/// أمر Tauri: مزامنة فورية عند طلب الواجهة.
#[tauri::command]
pub fn sync_device(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    sync_device_internal(&state)
}

/// إرسال/تحديث اسم الموظف على جهاز البصمة حسب رقم البصمة (USER_WRQ).
#[tauri::command]
pub fn push_user_to_device(
    state: tauri::State<'_, AppState>,
    pin: String,
    name: String,
) -> Result<String, String> {
    let (ip, port, key) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
        )
    };
    // الكتابة عبر البروتوكول المُثبت تجريبياً (add_user غير متاحة في DLL)
    let mut backend = make_backend(&ip, port, key, false);
    backend.connect().map_err(|e| format!("تعذر الاتصال: {e}"))?;
    backend.add_user(&pin, &name).map_err(|e| e.to_string())?;
    backend.close();
    Ok(format!("أُرسل الاسم إلى الجهاز للرقم {pin}"))
}

/// تعطيل مستخدم على جهاز البصمة (حذف سجله منه) — يُستدعى عند إيقاف موظف:
/// لا يعود الجهاز يقبل بصمته. سجلات حضوره القديمة تبقى كما هي.
#[tauri::command]
pub fn disable_user_on_device(
    state: tauri::State<'_, AppState>,
    pin: String,
) -> Result<String, String> {
    let (ip, port, key) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
        )
    };
    let mut backend = make_backend(&ip, port, key, false); // الكتابة عبر البروتوكول
    backend.connect().map_err(|e| format!("تعذر الاتصال: {e}"))?;
    backend.delete_user(&pin).map_err(|e| e.to_string())?;
    backend.close();
    Ok(format!("عُطّل الرقم {pin} على الجهاز — لم يعد يقبل البصمات"))
}

/// حركات موظف واحد (دخول/خروج/استراحة/إضافي…) مع ترشيح بفترة — لشاشة عرض الموظف.
#[tauri::command]
pub fn employee_punches(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    from_date: Option<String>,
    to_date: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    employee_punches_impl(&conn, employee_id, from_date, to_date, limit)
}

/// النواة القابلة للاختبار: ترشيح حركات موظف بفترة (فارغة = بلا حدّ).
fn employee_punches_impl(
    conn: &Connection,
    employee_id: i64,
    from_date: Option<String>,
    to_date: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<Value>, String> {
    let limit = limit.unwrap_or(1000).clamp(1, 5000);
    let from = from_date.unwrap_or_default();
    // to_date كتاريخ نهاية شامل ليومه الكامل (مقارنة نصية على ISO)؛
    // والفارغ = بلا حدّ أعلى (شرط '<' مع نص فارغ كان يُسقط كل الصفوف!)
    let to_bound = {
        let t = to_date.unwrap_or_default();
        if t.is_empty() {
            "9999-99-99T99:99:99".to_string()
        } else {
            format!("{t}T99:99:99")
        }
    };
    let mut st = conn
        .prepare(
            "SELECT device_time, punch_type, work_code, raw_status FROM punches
             WHERE employee_id=?1 AND device_time >= ?2 AND device_time < ?3
             ORDER BY device_time DESC LIMIT ?4",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map(rusqlite::params![employee_id, from, to_bound, limit], |r| {
            Ok(json!({
                "device_time": r.get::<_, String>(0)?,
                "punch_type": r.get::<_, i64>(1)?,
                "work_code": r.get::<_, i64>(2)?,
                "raw_status": r.get::<_, i64>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// يقرأ ساعة الجهاز ويقارنها بمنطقة Asia/Jerusalem المحددة.
#[tauri::command]
pub fn check_time_sync(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    time_check_internal(&state)
}

/// النواة المشتركة لفحص ساعة الجهاز — تستعمل أيضاً بعد المزامنة التلقائية
/// للتنبيه إن كانت الساعة خاطئة (سبب شائع لاختفاء بصمة اليوم من التقارير).
pub fn time_check_internal(state: &tauri::State<'_, AppState>) -> Result<Value, String> {
    let (ip, port, key, tz) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
            setting(&s, "timezone", "Asia/Jerusalem"),
        )
    };
    // الوقت (get/set) متاح في البروتوكول فقط — لا عبر DLL
    let mut b = make_backend(&ip, port, key, false);
    b.connect().map_err(|e| e.to_string())?;
    let dev_time = b.get_time_iso().map_err(|e| e.to_string())?;
    b.close();
    let (local_now, diff_min) = check_device_clock(&dev_time, &tz);
    Ok(json!({
        "device_time": dev_time,
        "server_time": local_now,
        "diff_minutes": diff_min,
        "timezone": tz,
        "in_sync": diff_min.abs() <= 1,
    }))
}

/// يضبط ساعة الجهاز على ساعة الخادم (حسب المنطقة الزمنية المحددة).
#[tauri::command]
pub fn fix_device_time(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let (ip, port, key, tz) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
            setting(&s, "timezone", "Asia/Jerusalem"),
        )
    };
    let now_local = {
        use chrono_tz::Tz;
        let tzv: Tz = tz.parse().unwrap_or(chrono_tz::Asia::Jerusalem);
        chrono::Utc::now()
            .with_timezone(&tzv)
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    };
    // الوقت (get/set) متاح في البروتوكول فقط — لا عبر DLL
    let mut b = make_backend(&ip, port, key, false);
    b.connect().map_err(|e| e.to_string())?;
    let res = b.set_time(&now_local);
    let after = b.get_time_iso().unwrap_or_default();
    b.close();
    res.map_err(|e| e.to_string())?;
    Ok(json!({ "set_to": now_local, "device_time_after": after }))
}

// ---------------------------------------------------------------- الحضور

/// يعيد احتساب أيام الحضور لعدد أيام معيّن في الماضي.
fn recompute_recent(state: &AppState, days: i64) -> Result<usize, String> {
    // تاريخ اليوم بالمنطقة الزمنية المحددة (لا UTC) حتى يشمل الاحتساب صف اليوم
    // حتى بعد منتصف الليل المحلي — مهم لتحوّل الدوام الجاري إلى «بصمة ناقصة».
    let today = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        today_in_tz(&settings_map(&conn))
    };
    let from = today - chrono::Duration::days(days);
    recompute_range(state, &from, &today)
}

/// يعيد احتساب أيام الحضور لموظف/لكل الموظفين بين تاريخين.
fn recompute_range(
    state: &AppState,
    from: &NaiveDate,
    to: &NaiveDate,
) -> Result<usize, String> {
    let (ps, weekend, employees, today, now) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        let ps = payroll_settings(&s);
        let weekend = crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
        let mut employees: Vec<Employee> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id,pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
                     meal_allow,transport_allow,active,
                     overtime_enabled,meal_enabled,transport_enabled,weekend_paid,annual_leave_balance,
                     annual_leave_days,leave_accrual_month
                     FROM employees",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(Employee {
                        id: r.get(0)?,
                        pin: r.get(1)?,
                        name: r.get(2)?,
                        name_en: String::new(),
                        job_title: r.get(3)?,
                        phone: r.get(4)?,
                        daily_hours: r.get(5)?,
                        break_min: r.get(6)?,
                        hourly_rate: r.get(7)?,
                        meal_allow: r.get(8)?,
                        transport_allow: r.get(9)?,
                        active: r.get::<_, i64>(10)? != 0,
                        overtime_enabled: r.get::<_, i64>(11)? != 0,
                        meal_enabled: r.get::<_, i64>(12)? != 0,
                        transport_enabled: r.get::<_, i64>(13)? != 0,
                        weekend_paid: r.get::<_, i64>(14)? != 0,
                        annual_leave_balance: r.get(15)?,
                        annual_leave_days: r.get(16)?,
                        leave_accrual_month: r.get(17)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                employees.push(row.map_err(|e| e.to_string())?);
            }
        }
        (ps, weekend, employees, today_in_tz(&s), now_in_tz(&s))
    };

    // لا غياب لأيام لم تأتِ بعد: احذف أي صفوف «غياب» مستقبلية كانت تُنشأ
    // سابقاً فتضخّم ملخص الشهر وتُنقص بدل العطلة الأسبوعية.
    let today_s = today.format("%Y-%m-%d").to_string();
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let _ = conn.execute(
            "DELETE FROM attendance_days
             WHERE date > ?1 AND absent = 1 AND manual = 0
               AND leave_type IS NULL AND status = 'absent'",
            rusqlite::params![today_s.clone()],
        );
    }

    // قفل لكل موظف على حدة (يُفلت بين الموظفين) حتى لا يحجب إعادة الاحتساب
    // استعلامات لوحة التحكم أثناء مزامنة الإقلاع الطويلة.
    let mut count = 0usize;
    for emp in employees {
        if !emp.active {
            continue;
        }
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        // جلب بصمات هذا الموظف في المدى (مع نوع كل بصمة للازدواج in/out)
        let mut punches: Vec<(String, i64)> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT device_time, punch_type FROM punches
                     WHERE device_time >= ?1 AND device_time <= ?2
                       AND (employee_id = ?3 OR pin = ?4)
                     ORDER BY device_time",
                )
                .map_err(|e| e.to_string())?;
            let from_s = format!("{}T00:00:00", from);
            let to_s = format!("{}T23:59:59", to);
            let pin = emp.pin.clone().unwrap_or_default();
            let rows = st.query_map(
                rusqlite::params![from_s, to_s, emp.id, pin],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            );
            if let Ok(rows) = rows {
                punches = rows.flatten().collect();
            }
        }
        // جلب الإجازات (مع حالة الدفع)
        let mut leaves: Vec<(NaiveDate, NaiveDate, String, bool)> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT from_date,to_date,leave_type,paid FROM leaves
                     WHERE employee_id=?1 AND to_date>=?2 AND from_date<=?3",
                )
                .map_err(|e| e.to_string())?;
            let rows = st.query_map(rusqlite::params![emp.id, from.to_string(), to.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? != 0,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    if let (Ok(a), Ok(b)) = (
                        NaiveDate::parse_from_str(&row.0, "%Y-%m-%d"),
                        NaiveDate::parse_from_str(&row.1, "%Y-%m-%d"),
                    ) {
                        leaves.push((a, b, row.2.clone(), row.3));
                    }
                }
            }
        }

        // الإجازات الرسمية (الأعياد) — موحّدة لكل الموظفين
        let mut holidays: Vec<NaiveDate> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT date FROM holidays WHERE date>=?1 AND date<=?2",
                )
                .map_err(|e| e.to_string())?;
            let rows = st.query_map(rusqlite::params![from.to_string(), to.to_string()], |r| {
                r.get::<_, String>(0)
            });
            if let Ok(rows) = rows {
                for d in rows.flatten() {
                    if let Ok(dt) = NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
                        holidays.push(dt);
                    }
                }
            }
        }

        // أيام «إلغاء خصم الاستراحة» لهذا الموظف في المدى (زر اليوم في نافذة التعديل)
        let mut no_break_dates: Vec<NaiveDate> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT date FROM attendance_days
                     WHERE employee_id=?1 AND date>=?2 AND date<=?3 AND no_break=1",
                )
                .map_err(|e| e.to_string())?;
            let rows = st.query_map(rusqlite::params![emp.id, from.to_string(), to.to_string()], |r| {
                r.get::<_, String>(0)
            });
            if let Ok(rows) = rows {
                for s in rows.flatten() {
                    if let Ok(dt) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                        no_break_dates.push(dt);
                    }
                }
            }
        }

        let days = compute_days_for_employee(emp.id, emp.pin.as_deref().unwrap_or(""), &punches, &leaves, &holidays, &weekend, &no_break_dates, &ps, *from, *to, now);
        for d in days {
            // الأيام المستقبلية لا تُسجَّل غياباً — تبقى بلا صف حتى يأتي يومها
            if d.date.as_str() > today_s.as_str() && d.absent {
                continue;
            }
            conn.execute(
                "INSERT INTO attendance_days(employee_id,date,first_in,last_out,worked_min,late_min,
                 early_leave_min,overtime_min,absent,leave_type,status,leave_paid)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                 ON CONFLICT(employee_id,date) DO UPDATE SET
                   first_in=excluded.first_in, last_out=excluded.last_out,
                   worked_min=excluded.worked_min, late_min=excluded.late_min,
                   early_leave_min=excluded.early_leave_min, overtime_min=excluded.overtime_min,
                   absent=excluded.absent, leave_type=excluded.leave_type, status=excluded.status,
                   leave_paid=excluded.leave_paid
                 WHERE attendance_days.manual = 0",
                rusqlite::params![
                    d.employee_id,
                    d.date,
                    d.first_in,
                    d.last_out,
                    d.worked_min,
                    d.late_min,
                    d.early_leave_min,
                    d.overtime_min,
                    d.absent as i64,
                    d.leave_type,
                    d.status,
                    d.leave_paid.map(|p| p as i64),
                ],
            )
            .map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    Ok(count)
}

#[tauri::command]
pub fn recompute_days(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<usize, String> {
    let f = NaiveDate::parse_from_str(&from, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let t = NaiveDate::parse_from_str(&to, "%Y-%m-%d").map_err(|e| e.to_string())?;
    recompute_range(&state, &f, &t)
}

fn month_days_impl(conn: &Connection, period: &str) -> Result<Vec<AttendanceDay>, String> {
    // نطاق بالتاريخ (وليس LIKE) ليستفيد من فهرس idx_attendance_days_date
    let (y, m) = parse_ym(period).ok_or("فترة غير صالحة — الصيغة YYYY-MM")?;
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let from = format!("{y:04}-{m:02}");
    let to = format!("{ny:04}-{nm:02}");
    let mut stmt = conn
        .prepare(
            "SELECT id,employee_id,date,first_in,last_out,worked_min,late_min,early_leave_min,
                    overtime_min,absent,leave_type,status,manual,leave_paid
             FROM attendance_days WHERE date >= ?1 AND date < ?2 ORDER BY employee_id, date",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![from, to], |r| {
            Ok(AttendanceDay {
                id: r.get(0)?,
                employee_id: r.get(1)?,
                date: r.get(2)?,
                first_in: r.get(3)?,
                last_out: r.get(4)?,
                worked_min: r.get(5)?,
                late_min: r.get(6)?,
                early_leave_min: r.get(7)?,
                overtime_min: r.get(8)?,
                absent: r.get::<_, i64>(9)? != 0,
                leave_type: r.get(10)?,
                status: r.get(11)?,
                manual: r.get::<_, i64>(12).unwrap_or(0) != 0,
                leave_paid: r.get::<_, Option<i64>>(13).ok().flatten().map(|v| v != 0),
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn get_month_days(
    state: tauri::State<'_, AppState>,
    period: String, // YYYY-MM
) -> Result<Vec<AttendanceDay>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    month_days_impl(&conn, &period)
}

// ---------------------------------------------------------------- الرواتب

#[tauri::command]
pub fn get_payroll(
    state: tauri::State<'_, AppState>,
    period: String,
) -> Result<Vec<PayrollResult>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    payroll_impl(&conn, &period)
}

/// المنطق الداخلي لحساب الرواتب (يُستدعى من الأوامر والأرشفة).
fn payroll_impl(conn: &Connection, period: &str) -> Result<Vec<PayrollResult>, String> {
    // احسب من أيام attendance_days المخزّنة
    let all_days = month_days_impl(conn, period)?;
    let s = settings_map(conn);
    let ps = payroll_settings(&s);
    let weekend = crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));

    let mut stmt = conn
        .prepare(
            "SELECT id,pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,
             overtime_enabled,meal_enabled,transport_enabled,weekend_paid,annual_leave_balance,
             annual_leave_days,leave_accrual_month
             FROM employees WHERE active=1 ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let employees: Vec<Employee> = stmt
        .query_map([], |r| {
            Ok(Employee {
                id: r.get(0)?,
                pin: r.get(1)?,
                name: r.get(2)?,
                name_en: String::new(),
                job_title: r.get(3)?,
                phone: r.get(4)?,
                daily_hours: r.get(5)?,
                break_min: r.get(6)?,
                hourly_rate: r.get(7)?,
                meal_allow: r.get(8)?,
                transport_allow: r.get(9)?,
                active: r.get::<_, i64>(10)? != 0,
                overtime_enabled: r.get::<_, i64>(11)? != 0,
                meal_enabled: r.get::<_, i64>(12)? != 0,
                transport_enabled: r.get::<_, i64>(13)? != 0,
                weekend_paid: r.get::<_, i64>(14)? != 0,
                annual_leave_balance: r.get(15)?,
                annual_leave_days: r.get(16)?,
                leave_accrual_month: r.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut out = Vec::new();
    for emp in employees {
        let emp_days: Vec<AttendanceDay> =
            all_days.iter().filter(|d| d.employee_id == emp.id).cloned().collect();
        let mut ps_emp = ps.clone();
        // أولويّة إعدادات الموظف إن خالفت الافتراضية
        if emp.daily_hours > 0.0 {
            ps_emp.daily_hours = emp.daily_hours;
        }
        if emp.hourly_rate > 0.0 {
            ps_emp.hourly_rate = emp.hourly_rate;
        }
        if emp.break_min >= 0 {
            ps_emp.break_min = emp.break_min;
        }
        // البدلات: مبالغ الموظف (تُشتقّ من الافتراضية عند الإنشاء)، وتُصفَّر عند الاستثناء
        ps_emp.meal_allow = if emp.meal_enabled { emp.meal_allow } else { 0.0 };
        ps_emp.transport_allow = if emp.transport_enabled { emp.transport_allow } else { 0.0 };
        // مضاعف الإضافي: القيمة العامة ما لم يكون الإضافي معطّلاً لهذا الموظف (×1.0)
        ps_emp.overtime_rate_mult = if emp.overtime_enabled {
            ps.overtime_rate_mult
        } else {
            1.0
        };
        let mut res = compute_payroll(
            emp.id,
            &emp.name,
            emp.pin.clone(),
            period,
            &emp_days,
            emp.weekend_paid,
            &weekend,
            &ps_emp,
        );
        // بدل الفطور: الأيام المعلّمة «إزالة بدل الفطور» في نافذة تعديل اليوم
        // لا تدخل في البدل اليومي (وليس في الصافي) — تُطرح قيمة أيامها.
        if ps_emp.meal_allow > 0.0 {
            let (py, pm) = parse_ym(period).unwrap_or((2000, 1));
            let (ny, nm) = if pm == 12 { (py + 1, 1) } else { (py, pm + 1) };
            let no_meal: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM attendance_days
                     WHERE employee_id=?1 AND date >= ?2 AND date < ?3
                       AND no_meal=1 AND status IN ('present','late','incomplete')
                       AND absent=0 AND first_in IS NOT NULL",
                    rusqlite::params![
                        emp.id,
                        format!("{py:04}-{pm:02}"),
                        format!("{ny:04}-{nm:02}")
                    ],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let sub = ps_emp.meal_allow * no_meal as f64;
            if sub > 0.0 {
                res.allowances -= sub;
                res.net_pay -= sub;
            }
        }
        out.push(res);
    }
    Ok(out)
}

/// حفظ كشف رواتب الشهر في payroll_runs (أرشفة).
#[tauri::command]
pub fn archive_payroll(
    state: tauri::State<'_, AppState>,
    period: String,
) -> Result<usize, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let rows = payroll_impl(&conn, &period)?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut n = 0;
    for r in rows {
        conn.execute(
            "INSERT INTO payroll_runs(period,employee_id,days_present,total_worked_hours,
             overtime_hours,base_pay,allowances,weekend_pay,late_deduction,absence_deduction,net_pay,generated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(period,employee_id) DO UPDATE SET
               days_present=excluded.days_present, total_worked_hours=excluded.total_worked_hours,
               overtime_hours=excluded.overtime_hours, base_pay=excluded.base_pay,
               allowances=excluded.allowances, weekend_pay=excluded.weekend_pay,
               late_deduction=excluded.late_deduction,
               absence_deduction=excluded.absence_deduction, net_pay=excluded.net_pay,
               generated_at=excluded.generated_at",
            rusqlite::params![
                period,
                r.employee_id,
                r.days_present,
                r.total_worked_hours,
                r.overtime_hours,
                r.base_pay,
                r.allowances,
                r.weekend_pay,
                r.late_deduction,
                r.absence_deduction,
                r.net_pay,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// الأشهر المؤرشفة في الرواتب (لتنقّل سريع وعرض الأشهر القديمة).
#[tauri::command]
pub fn payroll_months(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT period, COUNT(*), COALESCE(SUM(net_pay),0), MAX(generated_at)
             FROM payroll_runs GROUP BY period ORDER BY period DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "period": r.get::<_, String>(0)?,
                "employees": r.get::<_, i64>(1)?,
                "net_total": r.get::<_, f64>(2)?,
                "generated_at": r.get::<_, String>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// سجل رواتب موظف بعينه في كل الأشهر المؤرشفة (الأحدث أولاً).
#[tauri::command]
pub fn employee_payroll_history(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT period, days_present, total_worked_hours, overtime_hours,
                    base_pay, allowances, weekend_pay, net_pay, generated_at
             FROM payroll_runs WHERE employee_id=?1 ORDER BY period DESC LIMIT 60",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([employee_id], |r| {
            Ok(json!({
                "period": r.get::<_, String>(0)?,
                "days_present": r.get::<_, i64>(1)?,
                "total_worked_hours": r.get::<_, f64>(2)?,
                "overtime_hours": r.get::<_, f64>(3)?,
                "base_pay": r.get::<_, f64>(4)?,
                "allowances": r.get::<_, f64>(5)?,
                "weekend_pay": r.get::<_, f64>(6)?,
                "net_pay": r.get::<_, f64>(7)?,
                "generated_at": r.get::<_, String>(8)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

// ---------------------------------------------------------------- تقرير يومي

#[tauri::command]
pub fn get_daily_report(
    state: tauri::State<'_, AppState>,
    date: String,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    daily_report_impl(&conn, &date)
}

/// النواة المشتركة للتقرير اليومي (تتضمن أعلام اليوم: no_break / no_meal).
fn daily_report_impl(conn: &Connection, date: &str) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.employee_id, e.name, e.pin, a.first_in, a.last_out, a.worked_min,
                    a.late_min, a.early_leave_min, a.overtime_min, a.absent, a.status, a.manual,
                    a.leave_type, a.leave_paid, a.no_break, a.no_meal
             FROM attendance_days a LEFT JOIN employees e ON e.id = a.employee_id
             WHERE a.date = ?1 ORDER BY e.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([date], |r| {
            let mut m = serde_json::Map::new();
            m.insert("id".into(), json!(r.get::<_, i64>(0)?));
            m.insert("employee_id".into(), json!(r.get::<_, i64>(1)?));
            m.insert("name".into(), json!(r.get::<_, Option<String>>(2)?.unwrap_or_default()));
            m.insert("pin".into(), json!(r.get::<_, Option<String>>(3)?.unwrap_or_default()));
            m.insert("first_in".into(), json!(r.get::<_, Option<String>>(4)?));
            m.insert("last_out".into(), json!(r.get::<_, Option<String>>(5)?));
            m.insert("worked_min".into(), json!(r.get::<_, i64>(6)?));
            m.insert("late_min".into(), json!(r.get::<_, i64>(7)?));
            m.insert("early_leave_min".into(), json!(r.get::<_, i64>(8)?));
            m.insert("overtime_min".into(), json!(r.get::<_, i64>(9)?));
            m.insert("absent".into(), json!(r.get::<_, i64>(10)? != 0));
            m.insert("status".into(), json!(r.get::<_, String>(11)?));
            m.insert("manual".into(), json!(r.get::<_, i64>(12).unwrap_or(0) != 0));
            m.insert("leave_type".into(), json!(r.get::<_, Option<String>>(13)?));
            m.insert(
                "leave_paid".into(),
                json!(r.get::<_, Option<i64>>(14)?.map(|v| v != 0)),
            );
            m.insert("no_break".into(), json!(r.get::<_, i64>(15).unwrap_or(0) != 0));
            m.insert("no_meal".into(), json!(r.get::<_, i64>(16).unwrap_or(0) != 0));
            Ok(Value::Object(m))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ------------------------------------------------------- التعديل اليدوي للحضور

/// يقرأ صف يوم واحد كـ AttendanceDay (للإرجاع بعد التعديل).
fn day_row(
    conn: &Connection,
    employee_id: i64,
    date: &str,
) -> Result<AttendanceDay, String> {
    let mut st = conn
        .prepare(
            "SELECT id,employee_id,date,first_in,last_out,worked_min,late_min,early_leave_min,
                    overtime_min,absent,leave_type,status,manual,leave_paid
             FROM attendance_days WHERE employee_id=?1 AND date=?2",
        )
        .map_err(|e| e.to_string())?;
    st.query_row(rusqlite::params![employee_id, date], |r| {
        Ok(AttendanceDay {
            id: r.get(0)?,
            employee_id: r.get(1)?,
            date: r.get(2)?,
            first_in: r.get(3)?,
            last_out: r.get(4)?,
            worked_min: r.get(5)?,
            late_min: r.get(6)?,
            early_leave_min: r.get(7)?,
            overtime_min: r.get(8)?,
            absent: r.get::<_, i64>(9)? != 0,
            leave_type: r.get(10)?,
            status: r.get(11)?,
            manual: r.get::<_, i64>(12).unwrap_or(0) != 0,
            leave_paid: r.get::<_, Option<i64>>(13).ok().flatten().map(|v| v != 0),
        })
    })
    .map_err(|e| e.to_string())
}

/// تعديل يدوي ليوم حضور لموظف معيّن (mode):
/// - "present": يضبط الحضور/الانصراف يدوياً (نسيان تسجيل دخول/خروج، أو عطل الجهاز).
///   إن تُرك الانصراف فارغاً يُؤخذ آخر خروج محسوب لليوم (تعويض نسيان الخروج).
/// - "absent": يعلّم اليوم غياباً مهما تقول البصمات.
/// - "auto": يُلغي التعديل ويعيد اليوم للاحتساب التلقائي من البصمات.
/// - "annual"/"sick"/"unpaid": يحوّل اليوم إلى إجازة من يوم واحد:
///   السنوية تخصم يوماً من رصيد الموظف (ويُستعاد عند التحويل للرجوع) بشرط
///   توفّر الرصيد، المرضية بلا خصم، وبلا أجر بلا خصم؛ ويُرفض اليوم المغطّى
///   بإجازة متعددة الأيام برسالة واضحة (تُعدّل من شاشة الموظفين).
/// أي وضع يزيل إجازة اليوم الواحد السابقة المطابقة ويعيد رصيدها.
/// التعديل يُثبَّت (manual=1) فلا تمسه إعادة الاحتساب التلقائية حتى يُرجَع لـ auto.
#[tauri::command]
pub fn set_day_manual(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    date: String,
    mode: String,
    first_in: Option<String>,
    last_out: Option<String>,
) -> Result<Value, String> {
    let d = NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    match mode.as_str() {
        "annual" | "sick" | "unpaid" => {
            convert_day_to_leave(&state, employee_id, &date, &mode)?;
        }
        "auto" => {
            remove_single_day_leave(&state, employee_id, &date)?;
            {
                let conn = state.db.lock().map_err(|e| e.to_string())?;
                conn.execute(
                    "UPDATE attendance_days SET manual=0 WHERE employee_id=?1 AND date=?2",
                    rusqlite::params![employee_id, date],
                )
                .map_err(|e| e.to_string())?;
            }
            recompute_range(&state, &d, &d)?;
        }
        "absent" => {
            remove_single_day_leave(&state, employee_id, &date)?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO attendance_days(employee_id,date,first_in,last_out,worked_min,late_min,
                 early_leave_min,overtime_min,absent,leave_type,status,manual,leave_paid)
                 VALUES(?1,?2,NULL,NULL,0,0,0,0,1,NULL,'absent',1,NULL)
                 ON CONFLICT(employee_id,date) DO UPDATE SET
                   first_in=NULL, last_out=NULL, worked_min=0, late_min=0,
                   early_leave_min=0, overtime_min=0, absent=1, leave_type=NULL,
                   status='absent', manual=1, leave_paid=NULL",
                rusqlite::params![employee_id, date],
            )
            .map_err(|e| e.to_string())?;
        }
        "present" => {
            remove_single_day_leave(&state, employee_id, &date)?;
            let fin_raw = first_in
                .filter(|s| !s.trim().is_empty())
                .ok_or("أدخل وقت الحضور (HH:MM)")?;
            let (fh, fm) =
                parse_hhmm(&fin_raw).ok_or("صيغة وقت الحضور يجب أن تكون HH:MM")?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            // الانصراف: المُدخل أو آخر خروج محسوب (تعويض نسيان تسجيل الخروج)
            let fout_raw = last_out
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    conn.query_row(
                        "SELECT last_out FROM attendance_days WHERE employee_id=?1 AND date=?2",
                        rusqlite::params![employee_id, date],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten()
                })
                .ok_or("أدخل وقت الانصراف (HH:MM) — لا يوجد خروج محسوب لتعويضه")?;
            let (lh, lm) =
                parse_hhmm(&fout_raw).ok_or("صيغة وقت الانصراف يجب أن تكون HH:MM")?;
            let s = settings_map(&conn);
            let ps = payroll_settings(&s);
            let weekend =
                crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
            let punches = vec![
                (format!("{date}T{fh:02}:{fm:02}:00"), 0i64),
                (format!("{date}T{lh:02}:{lm:02}:00"), 1i64),
            ];
            // احترام علم «إلغاء خصم الاستراحة» لو كان مفعّلاً لهذا اليوم
            let no_break: bool = conn
                .query_row(
                    "SELECT no_break FROM attendance_days WHERE employee_id=?1 AND date=?2",
                    rusqlite::params![employee_id, date],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .unwrap_or(false);
            let no_break_dates = if no_break { vec![d] } else { Vec::new() };
            let days = crate::attendance::compute_days_for_employee(
                employee_id, "", &punches, &[], &[], &weekend, &no_break_dates, &ps, d, d,
                now_in_tz(&s),
            );
            let day = days.into_iter().next().ok_or("تعذر احتساب اليوم")?;
            conn.execute(
                "INSERT INTO attendance_days(employee_id,date,first_in,last_out,worked_min,late_min,
                 early_leave_min,overtime_min,absent,leave_type,status,manual,leave_paid)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1,NULL)
                 ON CONFLICT(employee_id,date) DO UPDATE SET
                   first_in=excluded.first_in, last_out=excluded.last_out,
                   worked_min=excluded.worked_min, late_min=excluded.late_min,
                   early_leave_min=excluded.early_leave_min, overtime_min=excluded.overtime_min,
                   absent=excluded.absent, leave_type=excluded.leave_type, status=excluded.status,
                   manual=1, leave_paid=NULL",
                rusqlite::params![
                    employee_id,
                    date,
                    day.first_in,
                    day.last_out,
                    day.worked_min,
                    day.late_min,
                    day.early_leave_min,
                    day.overtime_min,
                    day.absent as i64,
                    day.leave_type,
                    day.status
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        other => return Err(format!("وضع غير معروف: {other}")),
    }
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let day = day_row(&conn, employee_id, &date)?;
    Ok(serde_json::to_value(day).map_err(|e| e.to_string())?)
}

/// يزيل إجازة اليوم الواحد المطابقة لتاريخ معيّن ويستعيد يومها إلى رصيد
/// الإجازات السنوية إن كانت سنوية مدفوعة (لا يمسّ الإجازات المتعددة الأيام).
fn remove_single_day_leave(
    state: &AppState,
    employee_id: i64,
    date: &str,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let row: Option<(i64, String, i64)> = match conn.query_row(
        "SELECT id,leave_type,paid FROM leaves
         WHERE employee_id=?1 AND from_date=?2 AND to_date=?2 LIMIT 1",
        rusqlite::params![employee_id, date],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.to_string()),
    };
    if let Some((lid, leave_type, paid)) = row {
        if leave_type == "annual" && paid != 0 {
            conn.execute(
                "UPDATE employees SET annual_leave_balance = annual_leave_balance + 1,
                 updated_at=?1 WHERE id=?2",
                rusqlite::params![
                    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                    employee_id
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute("DELETE FROM leaves WHERE id=?1", [lid])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// يحوّل يوماً واحداً إلى إجازة (annual/sick/unpaid) مع احترام الرصيد:
/// - السنوية: تخصم يوماً من `annual_leave_balance` (تُرفض إن الرصيد < 1)،
///   وتُستعاد إذا كان في اليوم إجازة سنوية مدفوعة تُزال قبل الإنشاء.
/// - المرضية: مدفوعة بلا خصم (كما في بطاقة الموظف).
/// - بدون أجر: بلا خصم (تُعرض كإجازة سنوية غير مدفوعة في الرواتب).
/// اليوم المغطّى بإجازة متعددة الأيام يُرفض برسالة تُحيل لشاشة الموظفين.
fn convert_day_to_leave(
    state: &AppState,
    employee_id: i64,
    date: &str,
    kind: &str,
) -> Result<(), String> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let (leave_type, paid, deduct): (&str, bool, bool) = match kind {
        "annual" => ("annual", true, true),
        "sick" => ("sick", true, false),
        _ => ("annual", false, false), // بدون أجر
    };
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        // 1) رفض اليوم المغطّى بإجازة متعددة الأيام
        let multi: Option<(String, String)> = match conn.query_row(
            "SELECT from_date,to_date FROM leaves
             WHERE employee_id=?1 AND from_date<=?2 AND to_date>=?2
               AND from_date<>to_date LIMIT 1",
            rusqlite::params![employee_id, date],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.to_string()),
        };
        if let Some((f, t)) = multi {
            return Err(format!(
                "هذا اليوم ({date}) داخل إجازة متعددة الأيام [{f} → {t}] — عدّلها من بطاقة الموظف في شاشة «الموظفون»"
            ));
        }
        // 2) إزالة إجازة اليوم الواحد السابقة (مع إعادة رصيد السنوية منها)
        let old: Option<(i64, String, i64)> = match conn.query_row(
            "SELECT id,leave_type,paid FROM leaves
             WHERE employee_id=?1 AND from_date=?2 AND to_date=?2 LIMIT 1",
            rusqlite::params![employee_id, date],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.to_string()),
        };
        if let Some((lid, old_type, old_paid)) = old {
            if old_type == "annual" && old_paid != 0 {
                conn.execute(
                    "UPDATE employees SET annual_leave_balance = annual_leave_balance + 1,
                     updated_at=?1 WHERE id=?2",
                    rusqlite::params![
                        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                        employee_id
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
            conn.execute("DELETE FROM leaves WHERE id=?1", [lid])
                .map_err(|e| e.to_string())?;
        }
        // 3) السنوية: التحقق من الرصيد ثم خصم يوم
        if deduct {
            let balance: f64 = conn
                .query_row(
                    "SELECT annual_leave_balance FROM employees WHERE id=?1",
                    [employee_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if balance < 1.0 {
                return Err(format!(
                    "رصيد الإجازات السنوية غير كافٍ: المتاح {balance} يوم — عدّل الاستحقاق أو الرصيد من بطاقة الموظف"
                ));
            }
            conn.execute(
                "UPDATE employees SET annual_leave_balance = annual_leave_balance - 1,
                 updated_at=?1 WHERE id=?2",
                rusqlite::params![
                    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                    employee_id
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        // 4) إنشاء إجازة ليوم واحد
        let note = match kind {
            "annual" => "تحويل يوم إلى إجازة سنوية من شاشة الحضور",
            "sick" => "تحويل يوم إلى إجازة مرضية من شاشة الحضور",
            _ => "تحويل يوم إلى إجازة بدون أجر من شاشة الحضور",
        };
        conn.execute(
            "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                employee_id,
                date,
                date,
                leave_type,
                paid as i64,
                note,
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()
            ],
        )
        .map_err(|e| e.to_string())?;
        // 5) فك تثبيت اليوم (إن كان يدوياً) ليستعيد الاحتساب حالته إجازةً
        conn.execute(
            "UPDATE attendance_days SET manual=0 WHERE employee_id=?1 AND date=?2",
            rusqlite::params![employee_id, date],
        )
        .map_err(|e| e.to_string())?;
    }
    recompute_range(state, &d, &d)?;
    Ok(())
}

/// يقرأ HH:MM من تاريخ/وقت ISO ("…T06:00:00").
fn hhmm_of(iso: &str) -> Option<(&str, &str)> {
    let p = iso.split('T').nth(1)?;
    if p.len() >= 5 && p[0..2].chars().all(|c| c.is_ascii_digit()) && p[3..5].chars().all(|c| c.is_ascii_digit())
    {
        Some((&p[0..2], &p[3..5]))
    } else {
        None
    }
}

/// أزرار نافذة تعديل اليوم: إلغاء خصم الاستراحة / إزالة بدل الفطور.
/// يحفظ العلمين على صف اليوم ثم يعيد اشتقاق المتأثر: الصف المحسوب تلقائياً
/// يمرّ عبر recompute_range (لا يمسّه العمود أيّاً كان) أما الصف اليدوي
/// «حاضر/متأخر» فيُعاد اشتقاقه من الوقتين المخزّنين بعلم الاستراحة الجديد.
#[tauri::command]
pub fn set_day_flags(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    date: String,
    no_break: bool,
    no_meal: bool,
) -> Result<Value, String> {
    set_day_flags_impl(&state, employee_id, &date, no_break, no_meal)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let day = day_row(&conn, employee_id, &date)?;
    Ok(serde_json::to_value(day).map_err(|e| e.to_string())?)
}

fn set_day_flags_impl(
    state: &AppState,
    employee_id: i64,
    date: &str,
    no_break: bool,
    no_meal: bool,
) -> Result<(), String> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let manual: bool;
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        // يجب أن يكون اليوم محسوباً أصلاً (لديه صف)
        let (m, first_in, last_out, status): (bool, Option<String>, Option<String>, String) = conn
            .query_row(
                "SELECT manual, first_in, last_out, status FROM attendance_days
                 WHERE employee_id=?1 AND date=?2",
                rusqlite::params![employee_id, date],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? != 0,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                    ))
                },
            )
            .map_err(|_| {
                "لا يوجد يوم محسوب لهذا التاريخ — اضغط «إعادة احتساب» للشهر أولاً".to_string()
            })?;
        conn.execute(
            "UPDATE attendance_days SET no_break=?1, no_meal=?2
             WHERE employee_id=?3 AND date=?4",
            rusqlite::params![no_break as i64, no_meal as i64, employee_id, date],
        )
        .map_err(|e| e.to_string())?;
        manual = m;
        // إعادة اشتقاق الصف اليدوي الحاضر من الوقتين المخزّنين (العلم الجديد
        // يغيّر خصم الاستراحة) — الغياب/الإجازة بلا ساعات فلا تتأثر.
        if m && matches!(status.as_str(), "present" | "late" | "incomplete") {
            if let (Some(fi), Some(fo)) = (first_in, last_out) {
                if let (Some((fh, fm)), Some((lh, lm))) = (hhmm_of(&fi), hhmm_of(&fo)) {
                    let s = settings_map(&conn);
                    let ps = payroll_settings(&s);
                    let weekend =
                        crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
                    let punches = vec![
                        (format!("{date}T{fh}:{fm}:00"), 0i64),
                        (format!("{date}T{lh}:{lm}:00"), 1i64),
                    ];
                    let no_break_dates = if no_break { vec![d] } else { Vec::new() };
                    let days = compute_days_for_employee(
                        employee_id, "", &punches, &[], &[], &weekend, &no_break_dates, &ps, d, d,
                        now_in_tz(&s),
                    );
                    if let Some(day) = days.into_iter().next() {
                        conn.execute(
                            "UPDATE attendance_days SET worked_min=?1, late_min=?2,
                             early_leave_min=?3, overtime_min=?4, status=?5
                             WHERE employee_id=?6 AND date=?7",
                            rusqlite::params![
                                day.worked_min,
                                day.late_min,
                                day.early_leave_min,
                                day.overtime_min,
                                day.status,
                                employee_id,
                                date
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    if !manual {
        recompute_range(state, &d, &d)?;
    }
    Ok(())
}

// --------------------------------------------------- رسوم لوحة التحكم

/// يحذف صفوف «غياب» المستقبلية (أيام لم تأتِ بعد) التي كانت تُنشأ سابقاً
/// فتضخّم ملخّص الشهر وتُنقص بدل العطلة الأسبوعية. يُعاد بلا خطر.
pub fn cleanup_future_absences(conn: &Connection) -> usize {
    let today = today_in_tz(&settings_map(conn)).format("%Y-%m-%d").to_string();
    conn.execute(
        "DELETE FROM attendance_days
         WHERE date > ?1 AND absent = 1 AND manual = 0
           AND leave_type IS NULL AND status = 'absent'",
        rusqlite::params![today],
    )
    .unwrap_or(0)
}

/// الوقت المحلي الحالي بالمنطقة الزمنية المحددة (للحكم على الدوام الجاري).
fn now_in_tz(s: &HashMap<String, String>) -> chrono::NaiveDateTime {
    use chrono_tz::Tz;
    let tz: Tz = setting(s, "timezone", "Asia/Jerusalem")
        .parse()
        .unwrap_or(chrono_tz::Asia::Jerusalem);
    chrono::Utc::now().with_timezone(&tz).naive_local()
}

/// تاريخ اليوم بالمنطقة الزمنية المحددة (كما يُخزَّن في attendance_days).
fn today_in_tz(s: &HashMap<String, String>) -> chrono::NaiveDate {
    now_in_tz(s).date()
}

/// المتأخرون اليوم (id الموظف، الاسم، دقائق التأخير، وقت الحضور).
pub fn lates_today(conn: &Connection, today: &str) -> Vec<(i64, String, i64, String)> {
    let mut out = Vec::new();
    let mut st = match conn.prepare(
        "SELECT e.id, e.name, a.late_min, COALESCE(a.first_in,'') FROM attendance_days a
         JOIN employees e ON e.id = a.employee_id
         WHERE a.date = ?1 AND a.late_min > 0 ORDER BY a.late_min DESC",
    ) {
        Ok(s) => s,
        Err(_) => return out,
    };
    if let Ok(rows) = st.query_map([today], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    }) {
        out = rows.flatten().collect();
    }
    out
}

// ---------------------------------------------------------------- الإشعارات

/// يحفظ تنبيهاً في السجل — لا يكرّره لنفس (النوع، الموظف، اليوم).
/// يعيد true إن كان تنبيهاً جديداً فعلاً (لإظهار إشعار سطح المكتب له).
pub fn insert_notification(
    conn: &Connection,
    kind: &str,
    employee_id: Option<i64>,
    title: &str,
    body: &str,
    day: &str,
) -> bool {
    let dup: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notifications
             WHERE kind=?1 AND IFNULL(employee_id,-1)=IFNULL(?2,-1) AND day=?3",
            rusqlite::params![kind, employee_id, day],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if dup > 0 {
        return false;
    }
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO notifications(kind, employee_id, title, body, day, created_at, seen)
         VALUES(?1,?2,?3,?4,?5,?6,0)",
        rusqlite::params![kind, employee_id, title, body, day, now],
    )
    .is_ok()
}

/// يكتشف الأحداث الناقصة ويبقيها في سجل التنبيهات (مع منع التكرار).
/// يعيد وصف التنبيهات الجديدة فقط (لإظهارها كإشعار سطح المكتب).
pub fn detect_missing_events(conn: &Connection, today: &str) -> Vec<String> {
    let mut fresh = Vec::new();

    // 1) خروج مفقود: سجّل دخول في يوم سابق ولم يسجّل خروج حتى الآن **وانقضت
    //    مهلته** (status='incomplete'، أي مضى 14 ساعة على الدخول المفتوح) —
    //    فلا نُنبّه على وردية ما زالت جارية (مثل دوام مسائي يمتد بعد منتصف الليل).
    if let Ok(mut st) = conn.prepare(
        "SELECT d.employee_id, e.name, d.date, COALESCE(d.first_in,'')
         FROM attendance_days d JOIN employees e ON e.id=d.employee_id
         WHERE d.date < ?1 AND d.first_in IS NOT NULL AND d.last_out IS NULL
           AND d.status = 'incomplete'
         ORDER BY d.date DESC LIMIT 90",
    ) {
        if let Ok(rows) = st.query_map([today], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        }) {
            for (id, name, date, fin) in rows.flatten() {
                let t = fin.split('T').nth(1).map(|x| x[..5.min(x.len())].to_string()).unwrap_or_default();
                let title = format!("خروج مفقود: {name}");
                let body = format!(
                    "سجّل دخول يوم {date}{} ولم يسجّل خروجه حتى الآن",
                    if t.is_empty() { String::new() } else { format!(" الساعة {t}") }
                );
                if insert_notification(conn, "missing_out", Some(id), &title, &body, &date) {
                    fresh.push(format!("{name} — دخول {date} بدون خروج"));
                }
            }
        }
    }

    // 2) خروج بلا دخول: سجّل خروجاً دون أي تسجيل دخول في ذلك اليوم
    if let Ok(mut st) = conn.prepare(
        "SELECT d.employee_id, e.name, d.date, COALESCE(d.last_out,'')
         FROM attendance_days d JOIN employees e ON e.id=d.employee_id
         WHERE d.date <= ?1 AND d.first_in IS NULL AND d.last_out IS NOT NULL
         ORDER BY d.date DESC LIMIT 90",
    ) {
        if let Ok(rows) = st.query_map([today], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        }) {
            for (id, name, date, lout) in rows.flatten() {
                let t = lout.split('T').nth(1).map(|x| x[..5.min(x.len())].to_string()).unwrap_or_default();
                let title = format!("خروج بلا دخول: {name}");
                let body = format!(
                    "سجّل خروجاً يوم {date}{} دون تسجيل دخول",
                    if t.is_empty() { String::new() } else { format!(" الساعة {t}") }
                );
                if insert_notification(conn, "orphan_out", Some(id), &title, &body, &date) {
                    fresh.push(format!("{name} — خروج {date} بدون دخول"));
                }
            }
        }
    }

    fresh
}

/// سجل التنبيهات (الأحدث أولاً) — يعود لزر الجرس في الشريط الجانبي.
#[tauri::command]
pub fn list_notifications(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<Notification>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id,kind,employee_id,title,body,day,created_at,seen
             FROM notifications ORDER BY id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.unwrap_or(200).clamp(1, 1000)], |r| {
            Ok(Notification {
                id: r.get(0)?,
                kind: r.get(1)?,
                employee_id: r.get(2)?,
                title: r.get(3)?,
                body: r.get(4)?,
                day: r.get(5)?,
                created_at: r.get(6)?,
                seen: r.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// تعليم كل التنبيهات كمقروءة (يصفّر عدّاد الجرس).
#[tauri::command]
pub fn mark_notifications_seen(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("UPDATE notifications SET seen=1", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// مسح سجل التنبيهات بالكامل.
#[tauri::command]
pub fn clear_notifications(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM notifications", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// الحاضرون الآن: آخر بصمة اليوم لهم هي بصمة دخول (0=حضور 3=عودة من استراحة
/// 4=إضافي) — لا يهم إن سجّلوا خروجاً سابقاً خلال اليوم، المهم أنهم لم يخرجوا
/// بعد آخر دخولهم. تظهر أسماؤهم ووقت ذلك الدخول الأخير في لوحة التحكم
/// (زمن الجهاز المحلي — ليس UTC).
#[tauri::command]
pub fn present_now(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let s = settings_map(&conn);
    let today_d = today_in_tz(&s);
    let today = today_d.format("%Y-%m-%d").to_string();
    // نطاق بالتاريخ (يستفيد من idx_punches_time) بدل عملية substr غير الفهرسة
    let tomorrow = (today_d + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let mut stmt = conn
        .prepare(
            "WITH lastp AS (
               SELECT employee_id, MAX(device_time) AS mt
               FROM punches
               WHERE device_time >= ?1 AND device_time < ?2 AND employee_id IS NOT NULL
               GROUP BY employee_id
             )
             SELECT p.employee_id, e.name, COALESCE(e.pin,''), p.device_time
             FROM lastp lp
             JOIN punches p ON p.employee_id = lp.employee_id AND p.device_time = lp.mt
             JOIN employees e ON e.id = p.employee_id
             WHERE p.punch_type IN (0,3,4)
             ORDER BY p.device_time",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![today, tomorrow], |r| {
            Ok(json!({
                "employee_id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "pin": r.get::<_, String>(2)?,
                "last_in": r.get::<_, String>(3)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// رسوم بيانية للوحة التحكم: آخر 7 أيام + ملخص الشهر + الأعلى بالساعات (30 يوماً).
#[tauri::command]
pub fn dashboard_charts(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let s = settings_map(&conn);
    let today = today_in_tz(&s);
    // 0=اثنين … 6=أحد (كما في weekend_days — الافتراضي الجمعة فقط)
    let weekend: Vec<u32> = setting(&s, "weekend_days", "4")
        .split(',')
        .filter_map(|x| x.trim().parse::<u32>().ok())
        .collect();

    // تجميع آخر 7 أيام
    let from7 = (today - chrono::Duration::days(6))
        .format("%Y-%m-%d")
        .to_string();
    let to7 = today.format("%Y-%m-%d").to_string();
    let mut agg: HashMap<String, (i64, i64, i64, i64, i64, i64)> = HashMap::new();
    {
        let mut st = conn
            .prepare(
                "SELECT date,
                        COALESCE(SUM(CASE WHEN status IN ('present','late','incomplete') THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN late_min > 0 THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN absent = 1 THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN status = 'leave' THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(worked_min),0),
                        COALESCE(SUM(CASE WHEN status = 'holiday' THEN 1 ELSE 0 END),0)
                 FROM attendance_days WHERE date >= ?1 AND date <= ?2 GROUP BY date",
            )
            .map_err(|e| e.to_string())?;
        let rows = st.query_map(rusqlite::params![from7, to7], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            agg.insert(row.0, (row.1, row.2, row.3, row.4, row.5, row.6));
        }
    }
    let wd_names = ["اثنين", "ثلاثاء", "أربعاء", "خميس", "جمعة", "سبت", "أحد"];
    let mut last7 = Vec::new();
    for i in (0..7).rev() {
        let date = today - chrono::Duration::days(i);
        let key = date.format("%Y-%m-%d").to_string();
        let wd = date.weekday().num_days_from_monday();
        let (p, late, ab, lv, worked, hol) =
            agg.get(&key).cloned().unwrap_or((0, 0, 0, 0, 0, 0));
        last7.push(json!({
            "date": key,
            "label": wd_names[wd as usize % 7],
            "present": p, "late": late, "absent": ab, "leave": lv,
            "holiday": hol,
            "hours": worked as f64 / 60.0,
            "weekend": weekend.contains(&wd),
        }));
    }

    // ملخص الشهر الحالي
    let month_prefix = format!("{}%", today.format("%Y-%m"));
    let (m_present, m_late, m_absent, m_leave, m_hours, m_holiday, m_weekend) = {
        let mut st = conn
            .prepare(
                "SELECT COALESCE(SUM(CASE WHEN status IN ('present','late','incomplete') THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN late_min > 0 THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN absent = 1 THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN status = 'leave' THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(worked_min),0),
                        COALESCE(SUM(CASE WHEN status = 'holiday' THEN 1 ELSE 0 END),0),
                        COALESCE(SUM(CASE WHEN status = 'weekend' THEN 1 ELSE 0 END),0)
                 FROM attendance_days WHERE date LIKE ?1",
            )
            .map_err(|e| e.to_string())?;
        st.query_row([&month_prefix], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .unwrap_or((0, 0, 0, 0, 0, 0, 0))
    };
    // أيام العمل في الشهر حتى اليوم
    let m_start = today.with_day(1).unwrap();
    let mut workdays = 0i64;
    let mut cur = m_start;
    while cur <= today {
        if !weekend.contains(&cur.weekday().num_days_from_monday()) {
            workdays += 1;
        }
        cur = cur.succ_opt().unwrap();
    }

    // الأعلى بالساعات خلال 30 يوماً
    let from30 = (today - chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let mut top = Vec::new();
    {
        let mut st = conn
            .prepare(
                "SELECT e.name, SUM(a.worked_min) FROM attendance_days a
                 JOIN employees e ON e.id = a.employee_id
                 WHERE a.date >= ?1 AND a.absent = 0
                 GROUP BY a.employee_id HAVING SUM(a.worked_min) > 0
                 ORDER BY SUM(a.worked_min) DESC LIMIT 5",
            )
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([&from30], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
        for (name, mins) in rows.flatten() {
            top.push(json!({"name": name, "hours": mins as f64 / 60.0}));
        }
    }

    Ok(json!({
        "last7": last7,
        "month": {
            "present": m_present, "late": m_late, "absent": m_absent,
            "leave": m_leave, "holiday": m_holiday, "weekend": m_weekend,
            "workdays": workdays,
            "hours": m_hours as f64 / 60.0,
        },
        "top": top,
    }))
}

// ---------------------------------------------------------------- الإجازات

/// عدد أيام المدى [من، إلى] (بما فيه اليومان).
fn range_days(from_date: &str, to_date: &str) -> i64 {
    match (
        NaiveDate::parse_from_str(from_date, "%Y-%m-%d"),
        NaiveDate::parse_from_str(to_date, "%Y-%m-%d"),
    ) {
        (Ok(a), Ok(b)) => (b - a).num_days() + 1,
        _ => 0,
    }
}

/// تسجيل إجازة: النوع (annual/sick) + هل هي مدفوعة. الإجازة السنوية تُخصم من
/// رصيد الموظف (يُرفض إن كان الرصيد غير كافٍ)، وتُعاد عند الحذف.
#[tauri::command]
pub fn add_leave(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    from_date: String,
    to_date: String,
    leave_type: String,
    paid: bool,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let days = range_days(&from_date, &to_date);
    if days <= 0 {
        return Err("تاريخ الانتهاء يجب أن يكون بعد تاريخ البداية".into());
    }
    // الإجازة السنوية المدفوعة تُخصم من الرصيد — غير المدفوعة لا تخصم شيئاً
    if leave_type == "annual" && paid {
        let balance: f64 = conn
            .query_row(
                "SELECT annual_leave_balance FROM employees WHERE id=?1",
                [employee_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if balance < days as f64 {
            return Err(format!(
                "رصيد الإجازات السنوية غير كافٍ: المتاح {} يوم والطلب {} يوم — عدّل الرصيد من بطاقة الموظف",
                balance, days
            ));
        }
        conn.execute(
            "UPDATE employees SET annual_leave_balance = annual_leave_balance - ?1,
             updated_at=?2 WHERE id=?3",
            rusqlite::params![days, chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(), employee_id],
        )
        .map_err(|e| e.to_string())?;
    }
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![employee_id, from_date, to_date, leave_type, paid as i64, note, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// حذف إجازة: تُستعاد أيامها إلى رصيد الإجازات السنوية إن كانت سنوية، ثم
/// يُعاد احتساب الأيام المتأثرة تلقائياً.
#[tauri::command]
pub fn delete_leave(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let (employee_id, from_date, to_date, leave_type, paid): (
        i64,
        String,
        String,
        String,
        bool,
    ) = conn
        .query_row(
            "SELECT employee_id,from_date,to_date,leave_type,paid FROM leaves WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0)),
        )
        .map_err(|_| "الإجازة غير موجودة")?;
    // تستعاد إلى الرصيد الأيام التي خُصم فعلاً (سنوية مدفوعة فقط)
    if leave_type == "annual" && paid {
        let days = range_days(&from_date, &to_date);
        conn.execute(
            "UPDATE employees SET annual_leave_balance = annual_leave_balance + ?1,
             updated_at=?2 WHERE id=?3",
            rusqlite::params![days, chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(), employee_id],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.execute("DELETE FROM leaves WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    drop(conn);
    // إعادة احتساب الأيام المتأثرة (لتصبح غياباً أو حضوراً حسب البصمات)
    if let (Ok(a), Ok(b)) = (
        NaiveDate::parse_from_str(&from_date, "%Y-%m-%d"),
        NaiveDate::parse_from_str(&to_date, "%Y-%m-%d"),
    ) {
        let _ = recompute_range(&state, &a, &b);
    }
    Ok(())
}

/// يحوّل "YYYY-MM" إلى (السنة, الشهر) مع تحقق من النطاق.
fn parse_ym(s: &str) -> Option<(i32, u32)> {
    let mut it = s.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    (1..=12).contains(&m).then_some((y, m))
}

/// الإضافة التلقائية لأرصدة الإجازات السنوية: يُضاف لكل موظف نشِط
/// (استحقاقه السنوي ÷ 12) عن كل شهر عمل مكتمل — **وتُستأنف من حيث توقفت
/// آخر مرة** عبر علامة `leave_accrual_month` فلا يُحتسب شهرٌ مرتين ولا يفوّت
/// شهر إن لم يُفتح التطبيق يوم موعده:
///
/// - لا يُحتسب الشهر الجاري قبل اكتماله (عادل على الشركة)، ولا يُسقط شهر
///   التوظيف كاملاً عند أول احتساب: يُحسب **بنسبة الأيام الباقية** من تاريخ
///   التعيين فقط (عادل على العامل)، فإن كان التعيين في الشهر نفسه بعد آخر
///   شهر مكتمل فلا شيء يُضاف حتى يكتمل الشهر.
/// - يُستدعى مرة واحدة يومياً من الحلقة الخلفية، ومن أمر `accrue_leave_now`.
pub fn accrue_leave_impl(conn: &Connection, today: NaiveDate) -> Result<Value, String> {
    let s = settings_map(conn);
    // الإعداد «الإضافة التلقائية لأرصدة الإجازات» (1 = مفعّلة — الافتراضي).
    if setting(&s, "leave_accrual_enabled", "1") == "0" {
        return Ok(json!({"enabled": false, "accrued": 0, "days": 0.0}));
    }

    // آخر شهر مكتمل قبل اليوم (الشهر الجاري يُحتسب عند اكتماله لا قبله).
    let (ty, tm) = (today.year(), today.month());
    let (ly, lm) = if tm == 1 { (ty - 1, 12) } else { (ty, tm - 1) };
    let last_done = (ly, lm);

    let mut stmt = conn
        .prepare(
            "SELECT id, name, annual_leave_days, annual_leave_balance,
                    leave_accrual_month, created_at
             FROM employees WHERE active = 1 ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String, f64, f64, Option<String>, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut accrued = 0usize;
    let mut total_days = 0.0f64;
    let mut details = Vec::new();

    for (id, name, annual, balance, anchor, created_at) in rows {
        // استحقاق غير مفعّل (0) ← لا إضافة لهذا الموظف إطلاقاً.
        if annual <= 0.0 {
            continue;
        }
        let monthly = annual / 12.0;

        // شهر التعيين ويومه (للأول احتسب — إن لم يُسبقه احتساب).
        let hire_ym = created_at.get(..7).map(|x| x.to_string());
        let hire_day: u32 = created_at
            .get(8..10)
            .and_then(|d| d.parse().ok())
            .unwrap_or(1);

        // نقطة الانطلاق: آخر شهر احتُسب، وإلا شهر التعيين (أول مرة).
        let start = anchor.clone().or(hire_ym);
        let Some(start) = start else { continue };
        let Some((sy, sm)) = parse_ym(&start) else { continue };
        // لم يكتمل الشهر الأول بعد ← لا شيء يُضاف (يبدأ الاحتساب لاحقاً).
        if (sy, sm) > last_done {
            continue;
        }

        // الشهور المكتملة بعد شهر الانطلاق حتى آخر شهر مكتمل.
        let full_months = (last_done.0 - sy) * 12 + (last_done.1 as i32 - sm as i32);
        let mut add = 0.0;
        if anchor.is_none() {
            // أول احتساب: شهر التعيين بالنسبة الباقية من يوم التعيين فقط —
            // لا يُسقط الشهر كاملاً (تأخير التوظيف بعد اليوم 20 مثلنا).
            let dim = NaiveDate::from_ymd_opt(sy, sm, 1)
                .map(|d| d.num_days_in_month() as u32)
                .unwrap_or(28);
            let worked_days = (dim.saturating_sub(hire_day) + 1) as f64;
            add += monthly * worked_days / dim as f64;
        }
        // الشهور الكاملة بعده (وإن غاب التطبيق شهوراً ← يُلحقها كلها دفعة واحدة).
        add += monthly * full_months as f64;
        if add <= 0.0 {
            continue;
        }

        let new_balance = ((balance + add) * 10000.0).round() / 10000.0;
        let new_anchor = format!("{ly:04}-{lm:02}");
        conn.execute(
            "UPDATE employees SET annual_leave_balance = ?1, leave_accrual_month = ?2,
             updated_at = ?3 WHERE id = ?4",
            rusqlite::params![new_balance, new_anchor, now, id],
        )
        .map_err(|e| e.to_string())?;
        accrued += 1;
        total_days += add;
        details.push(json!({"name": name, "days": ((add * 10000.0).round() / 10000.0)}));
    }

    // علامة آخر تشغيل ناجح (لتقليل الاستدعاءات اليومية — الاحتساب بحدّ ذاته
    // آمن للتكرار لأن العلامة تتحرك مع كل إضافة).
    let _ = conn.execute(
        "INSERT INTO settings(key,value) VALUES('leave_accrual_last_run',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [&today.format("%Y-%m-%d").to_string()],
    );

    Ok(json!({
        "enabled": true,
        "accrued": accrued,
        "days": (total_days * 10000.0).round() / 10000.0,
        "month": format!("{ly:04}-{lm:02}"),
        "details": details,
    }))
}

/// أمر يدوي: «احتساب أرصدة الإجازات الآن» من زر في الإعدادات.
#[tauri::command]
pub fn accrue_leave_now(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let today = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        today_in_tz(&settings_map(&conn))
    };
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    accrue_leave_impl(&conn, today)
}

#[tauri::command]
pub fn list_leaves(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.employee_id, e.name, l.from_date, l.to_date, l.leave_type, l.note, l.paid
             FROM leaves l LEFT JOIN employees e ON e.id=l.employee_id
             ORDER BY l.from_date DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "employee_id": r.get::<_, i64>(1)?,
            "name": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            "from_date": r.get::<_, String>(3)?,
            "to_date": r.get::<_, String>(4)?,
            "leave_type": r.get::<_, String>(5)?,
            "note": r.get::<_, String>(6)?,
            "paid": r.get::<_, i64>(7)? != 0,
        }))
    })
    .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// --------------------------------------------------------- الإجازات الرسمية

/// قائمة الإجازات الرسمية (الأعياد) — تُحتسب مدفوعة لكل الموظفين.
#[tauri::command]
pub fn holidays_list(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, date, name FROM holidays ORDER BY date")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "date": r.get::<_, String>(1)?,
            "name": r.get::<_, String>(2)?,
        }))
    })
    .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// إضافة إجازة رسمية (عيد): يُعاد احتساب ذلك اليوم لكل الموظفين فوراً.
#[tauri::command]
pub fn holiday_add(
    state: tauri::State<'_, AppState>,
    date: String,
    name: String,
) -> Result<i64, String> {
    let d = NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|_| "صيغة التاريخ YYYY-MM-DD")?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO holidays(date,name,created_at) VALUES(?1,?2,?3)
             ON CONFLICT(date) DO UPDATE SET name=excluded.name",
            rusqlite::params![date, name, now],
        )
        .map_err(|e| e.to_string())?;
    }
    let _ = recompute_range(&state, &d, &d);
    Ok(1)
}

/// حذف إجازة رسمية: يُعاد احتساب ذلك اليوم لكل الموظفين (يعود غياباً/حضراً).
#[tauri::command]
pub fn holiday_delete(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let date: String = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let date: String = conn
            .query_row("SELECT date FROM holidays WHERE id=?1", [id], |r| r.get(0))
            .map_err(|_| "الإجازة الرسمية غير موجودة")?;
        conn.execute("DELETE FROM holidays WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        date
    };
    if let Ok(d) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
        let _ = recompute_range(&state, &d, &d);
    }
    Ok(())
}

// ---------------------------------------------------------------- استيراد/تصدير

/// يعيد ملخصاً سريعاً للوحة التحكم.
#[tauri::command]
pub fn dashboard(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let s = settings_map(&conn);
    // اليوم بمنطقة زمنية محلية (وليس UTC) حتى لا ينقلب التاريخ قبل منتصف الليل
    let today = today_in_tz(&s).format("%Y-%m-%d").to_string();
    let emp_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM employees WHERE active=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM attendance_days WHERE date=?1 AND status IN ('present','late','incomplete')",
            [&today],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let absent: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM attendance_days WHERE date=?1 AND absent=1",
            [&today],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let punches_total: i64 =
        conn.query_row("SELECT COUNT(*) FROM punches", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
    let last_sync = setting(&settings_map(&conn), "last_sync_at", "-");
    Ok(json!({
        "date": today,
        "employees": emp_count,
        "present": present,
        "absent": absent,
        "total_punches": punches_total,
        "last_sync": last_sync,
    }))
}

/// يبني (أو يحدّث) رابط شعار الشركة من ملف مختار (نص المسار يُخزَّن في الإعدادات).
#[tauri::command]
pub fn set_company_logo(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    save_setting(state, "company_logo".into(), path)
}

/// يحوّل مسار ملف إلى data URI ليُعرض في التقرير المطبوع.
#[tauri::command]
pub fn read_logo_data_uri(path: String) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let mime = match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    Ok(format!("data:{};base64,{}", mime, base64_encode(&buf)))
}

// ترميز base64 بسيط (بدون depend إضافية)
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// مطابقة البصمات (تُستخدم للتحقق من ربط PIN بالموظف).
#[tauri::command]
pub fn pair_punch_pins(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT pin, device_time FROM punches ORDER BY device_time")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let map = pair_punches(&rows);
    Ok(json!({ "pins_with_days": map.len() }))
}

/// (معاينة) يعيد الوقت الحالي حسب المنطقة الزمنية للتطبيق.
#[tauri::command]
pub fn server_time(tz: String) -> Result<String, String> {
    use chrono_tz::Tz;
    let t: Tz = tz.parse().map_err(|e| format!("{e}"))?;
    Ok(chrono::Utc::now().with_timezone(&t).format("%Y-%m-%dT%H:%M:%S").to_string())
}

/// يستورد بصمات من ملف CSV (pin,datetime[,type,workcode,status]).
#[tauri::command]
pub fn import_punches_csv(state: tauri::State<'_, AppState>, path: String) -> Result<usize, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut pin_to_id: HashMap<String, i64> = HashMap::new();
    {
        let mut st = conn
            .prepare("SELECT id,pin FROM employees WHERE pin IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(0)?)));
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                pin_to_id.insert(row.0, row.1);
            }
        }
    }
    let mut n = 0;
    for line in text.lines().skip(1) {
        let p: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if p.len() < 2 || p[0].is_empty() {
            continue;
        }
        let pin = p[0].to_string();
        let dt = p[1].to_string();
        if !dt.contains('T') && dt.contains(' ') {
            continue;
        }
        let punch_type = p.get(2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let wc = p.get(3).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let status = p.get(4).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let emp_id = pin_to_id.get(&pin).copied();
        let r = conn.execute(
            "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(pin,device_time) DO NOTHING",
            rusqlite::params![emp_id, pin, dt, punch_type, wc, status, now],
        );
        if let Ok(x) = r {
            n += x;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// اختبار رجوعي: حدّ «إلى تاريخ» فارغ يجب ألّا يُسقط الحركات.
    /// (كان شرط device_time < '' ينفي كل الصفوف — سبب خلو شاشة العرض.)
    #[test]
    fn employee_punches_empty_bounds_returns_all() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,created_at,updated_at)
             VALUES(9,'9','تسعة','x','x')",
            [],
        )
        .unwrap();
        for (t, pt) in [
            ("2000-02-08T22:21:13", 0),
            ("2000-05-01T08:00:00", 1),
            ("2002-03-10T15:34:41", 0),
        ] {
            conn.execute(
                "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
                 VALUES(9,'9',?1,?2,0,0,'x')",
                rusqlite::params![t, pt],
            )
            .unwrap();
        }
        // بلا فلاتر: كل الحركات
        let all = employee_punches_impl(&conn, 9, None, None, None).unwrap();
        assert_eq!(all.len(), 3, "بلا فلاتر يجب أن تظهر كل الحركات");
        // فترة تشمل 2000-05 وحدها
        let ranged = employee_punches_impl(
            &conn,
            9,
            Some("2000-05-01".into()),
            Some("2000-05-31".into()),
            None,
        )
        .unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0]["device_time"], "2000-05-01T08:00:00");
        // فترة بلا بيانات (مثل 2026 على جهاز بساعة خاطئة): فارغة
        let y2026 = employee_punches_impl(
            &conn,
            9,
            Some("2026-01-01".into()),
            Some("2026-12-31".into()),
            None,
        )
        .unwrap();
        assert!(y2026.is_empty());
    }

    /// اختبار السيناريو الكامل (نهاية إلى نهاية) عبر مسار الإنتاج الحقيقي
    /// (recompute_range ← payroll_impl) الموافق لسيناريو القبول المطلوب:
    /// حسن وخليل (81 — الموظف الجديد) — دوام يومي 06:00–16:30 ما عدا الجمعة
    /// (عطلة أسبوعية تلقائية)، وأكتوبر 2026 (31 يوماً فيها 5 جمع) بدوام كامل.
    /// المتوقّع لكل منهما: أجر اليوم 165 ₪ + بدل فطور 7 ₪، والجمع 5 × 62،
    /// والصافي 4782 ₪ — بالطريقة الأسبوعية الافتراضية (62 ÷ أيام السبت–الخميس).
    #[test]
    fn hassan_october_2026_scenario_end_to_end() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        db::seed_defaults(&conn).unwrap();
        // إعدادات السيناريو: الدوام 06:00–16:30 وعطلة الأسبوع الجمعة فقط
        for (k, v) in [
            ("work_start", "06:00"),
            ("work_end", "16:30"),
            ("weekend_days", "4"),
        ] {
            db::set_setting(&conn, k, v).unwrap();
        }
        let state = AppState {
            db: Mutex::new(conn),
            base: std::env::temp_dir(),
        };

        {
            let conn = state.db.lock().unwrap();
            // الموظفان كما على الإنتاج: 15 ₪/ساعة، بدل فطور 7، بدون بدل مواصلات
            for (id, pin, name, name_en) in [
                (12i64, "80", "حسن", "Hassan"),
                (13i64, "81", "خليل", "Khalil"), // الموظف الجديد رقم 81
            ] {
                conn.execute(
                    "INSERT INTO employees(id,pin,name,name_en,daily_hours,break_min,hourly_rate,
                     meal_allow,transport_allow,active,overtime_enabled,meal_enabled,
                     transport_enabled,weekend_paid,annual_leave_balance,created_at,updated_at)
                     VALUES(?1,?2,?3,?4,8.5,30,15.0,7.0,0.0,1,1,1,0,1,0.0,'x','x')",
                    rusqlite::params![id, pin, name, name_en],
                )
                .unwrap();
            }
            // البصمات: كل يوم عمل حضور 06:00 وانصراف 16:30 — بلا بصمات يوم الجمعة
            let mut d = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
            let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
            while d <= to {
                if d.weekday().num_days_from_monday() != 4 {
                    let day = d.format("%Y-%m-%d").to_string();
                    for (dt, pt) in [
                        (format!("{day}T06:00:00"), 0i64),
                        (format!("{day}T16:30:00"), 1i64),
                    ] {
                        for (emp, pin) in [(12i64, "80"), (13i64, "81")] {
                            conn.execute(
                                "INSERT INTO punches(employee_id,pin,device_time,punch_type,synced_at)
                                 VALUES(?1,?2,?3,?4,'x')",
                                rusqlite::params![emp, pin, dt, pt],
                            )
                            .unwrap();
                        }
                    }
                }
                d = d.succ_opt().unwrap();
            }
        }

        // احتساب حقيقي عبر مسار الإنتاج: إعادة الاحتساب ثم الرواتب
        let from = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        recompute_range(&state, &from, &to).unwrap();

        let conn = state.db.lock().unwrap();
        let mut problems: Vec<String> = Vec::new();
        let days = month_days_impl(&conn, "2026-10").unwrap();
        let rows = payroll_impl(&conn, "2026-10").unwrap();

        for (emp_id, who) in [(12i64, "حسن"), (13i64, "خليل (81)")] {
            // (1) الأيام: أيام الدوام حاضرة 06:00→16:30 بأجر يوم 165، والجمعة عطلة
            let mut workdays_seen = 0;
            let mut fridays_seen = 0;
            for d in days.iter().filter(|d| d.employee_id == emp_id) {
                let is_friday = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d")
                    .unwrap()
                    .weekday()
                    .num_days_from_monday()
                    == 4;
                if is_friday {
                    fridays_seen += 1;
                    if d.absent || d.status == "absent" {
                        problems.push(format!(
                            "{who} · {}: الجمعة مسجّلة «غياب» وليست عطلة أسبوعية",
                            d.date
                        ));
                    }
                } else {
                    workdays_seen += 1;
                    let daily = (d.worked_min - d.overtime_min) as f64 / 60.0 * 15.0
                        + d.overtime_min as f64 / 60.0 * 15.0 * 1.5;
                    if d.status != "present" || d.worked_min != 600 || d.overtime_min != 120 {
                        problems.push(format!(
                            "{who} · {}: الحالة {} وساعات فعلية {} (إضافي {}) — المتوقّع حاضر 600 د بإضافي 120",
                            d.date, d.status, d.worked_min, d.overtime_min
                        ));
                    }
                    if (daily - 165.0).abs() > 1e-9 {
                        problems.push(format!(
                            "{who} · {}: أجر اليوم {:.2} ₪ بدل 165.00 ₪",
                            d.date, daily
                        ));
                    }
                }
            }
            if workdays_seen != 26 {
                problems.push(format!(
                    "{who}: أيام الدوام المحسوبة {} بدل 26",
                    workdays_seen
                ));
            }
            if fridays_seen != 5 {
                problems.push(format!("{who}: عدد الجمعة في الشهر {} بدل 5", fridays_seen));
            }

            // (2) كشف الرواتب: الأجر والبدلات وبدل العطلة والصافي
            let Some(r) = rows.iter().find(|x| x.employee_id == emp_id) else {
                problems.push(format!("{who}: لا يوجد صف راتب لموظف نشط"));
                continue;
            };
            println!("== سيناريو {who} — أكتوبر 2026 ==");
            println!(
                "أيام الدوام: {} · الجمعة: {} · الإضافي: {:.1} سا",
                r.days_present, fridays_seen, r.overtime_hours
            );
            println!(
                "الأجر: {:.2} (26×165=4290) | البدلات: {:.2} (26×7=182) | العطلة: {:.2} (5×62=310) | الصافي: {:.2} (4782)",
                r.base_pay, r.allowances, r.weekend_pay, r.net_pay
            );
            let expectations: Vec<(String, f64, f64)> = vec![
                (format!("{who} — عدد أيام الحضور"), r.days_present as f64, 26.0),
                (format!("{who} — أجر الدوام الأساسي (26 × 165)"), r.base_pay, 4290.0),
                (format!("{who} — بدل الفطور فقط (26 × 7)"), r.allowances, 182.0),
                (format!("{who} — بدل العطلة (5 جمع × 62)"), r.weekend_pay, 310.0),
                (format!("{who} — الصافي النهائي"), r.net_pay, 4782.0),
            ];
            for (label, actual, expected) in expectations {
                if (actual - expected).abs() > 0.01 {
                    problems.push(format!(
                        "{}: الفعلي {:.2} بدل المتوقّع {:.2} (فرق {:+.2})",
                        label,
                        actual,
                        expected,
                        actual - expected
                    ));
                }
            }
        }

        assert!(
            problems.is_empty(),
            "معايير القبول المخالفة ({}):\n{}",
            problems.len(),
            problems.join("\n")
        );
    }

    // ------------------------------------------- الاحتساب الشهري لأرصدة الإجازات

    /// يقرأ رصيد وعلامة احتساب موظف.
    fn balance_and_anchor(conn: &Connection, id: i64) -> (f64, Option<String>) {
        conn.query_row(
            "SELECT annual_leave_balance, leave_accrual_month FROM employees WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    }

    /// موظف مستحق 14 يوماً/سنة (≈ 1.1667 شهرياً) تعيينه 2026-01-15: في فبراير
    /// يُحتسب يناير بنسبة الأيام الباقية (17/31 من الشهر بعد اليوم 15) — لا
    /// كاملاً ولا مسقطاً — وعن كل شهر مكتمل بعده شهر كامل حتى آخر شهر مكتمل.
    #[test]
    fn accrual_prorates_hire_month_then_full_months() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,daily_hours,annual_leave_days,annual_leave_balance,
             active,created_at,updated_at)
             VALUES(1,'1','موظف',8.5,14.0,0.0,1,'2026-01-15T08:00:00','x')",
            [],
        )
        .unwrap();
        // 1 فبراير: آخر شهر مكتمل يناير ← نسبة 17/31 فقط
        let v = accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()).unwrap();
        assert_eq!(v["accrued"], 1);
        let (bal, anchor) = balance_and_anchor(&conn, 1);
        let expected_jan = 14.0 / 12.0 * 17.0 / 31.0;
        assert!((bal - expected_jan).abs() < 1e-4, "يناير بنسبة 17/31: {bal}");
        assert_eq!(anchor.as_deref(), Some("2026-01"));

        // 1 مارس: فبراير مكتمل ← شهر كامل إضافي
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()).unwrap();
        let (bal2, _) = balance_and_anchor(&conn, 1);
        assert!((bal2 - expected_jan - 14.0 / 12.0).abs() < 1e-4, "فبراير كامل: {bal2}");

        // اليوم نفسه يُستدعى مجدداً ← آمن للتكرار (العلامة تمنع إعادة الاحتساب)
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()).unwrap();
        let (bal3, anchor3) = balance_and_anchor(&conn, 1);
        assert!((bal3 - bal2).abs() < 1e-9, "التكرار لا يضيف شيئاً: {bal3}");
        assert_eq!(anchor3.as_deref(), Some("2026-02"));

        // إغلاق التطبيق شهوراً (أبريل–يوليو) ← التشغيل في أغسطس يُلحقها كلها
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 8, 10).unwrap()).unwrap();
        let (bal4, anchor4) = balance_and_anchor(&conn, 1);
        // مارس..يوليو = 5 شهور مكتملة
        assert!(
            (bal4 - expected_jan - 14.0 / 12.0 * 6.0).abs() < 1e-4,
            "استئناف من حيث توقف (5 شهور متأخرة): {bal4}"
        );
        assert_eq!(anchor4.as_deref(), Some("2026-07"));

        // الشهر الجاري (أغسطس) لا يُحتسب قبل اكتماله
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()).unwrap();
        let (bal5, anchor5) = balance_and_anchor(&conn, 1);
        assert!((bal5 - bal4).abs() < 1e-9, "الشهر الجاري يُنتظر اكتماله");
        assert_eq!(anchor5.as_deref(), Some("2026-07"));
    }

    /// استحقاق موظف 0 يعني بلا إجازات ← لا يُضاف شيء ولا تتحرك العلامة.
    #[test]
    fn accrual_zero_entitlement_skipped() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,daily_hours,annual_leave_days,annual_leave_balance,
             active,created_at,updated_at)
             VALUES(1,'1','بلا إجازات',8.5,0.0,0.0,1,'2026-01-15T08:00:00','x')",
            [],
        )
        .unwrap();
        let v = accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()).unwrap();
        assert_eq!(v["accrued"], 0);
        let (bal, anchor) = balance_and_anchor(&conn, 1);
        assert_eq!(bal, 0.0);
        assert!(anchor.is_none());
    }

    /// الموظف غير النشِط لا يحتسب، والإضافة التلقائية المعطّلة (0) لا تعمل.
    #[test]
    fn accrual_inactive_and_disabled_are_respected() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,daily_hours,annual_leave_days,annual_leave_balance,
             active,created_at,updated_at)
             VALUES(1,'1','موقوف',8.5,14.0,0.0,0,'2026-01-15T08:00:00','x'),
                    (2,'2','نشط',8.5,14.0,0.0,1,'2026-01-15T08:00:00','x')",
            [],
        )
        .unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let v = accrue_leave_impl(&conn, today).unwrap();
        assert_eq!(v["accrued"], 1, "النشط فقط");
        assert_eq!(balance_and_anchor(&conn, 1).0, 0.0, "الموقوف بلا إضافة");
        assert!(balance_and_anchor(&conn, 2).0 > 0.0);

        // تعطيل الإضافة التلقائية من الإعدادات ← توقّف فوري
        db::set_setting(&conn, "leave_accrual_enabled", "0").unwrap();
        let v2 = accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()).unwrap();
        assert_eq!(v2["enabled"], false);
        assert_eq!(v2["accrued"], 0);
        let before = balance_and_anchor(&conn, 2).0;
        let v3 = accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()).unwrap();
        assert_eq!(v3["accrued"], 0);
        assert_eq!(balance_and_anchor(&conn, 2).0, before, "لا إضافة بعد التعطيل");
    }

    /// موظف تعيينه الشهر الجاري (بعد آخر شهر مكتمل) ← لا شيء حتى يكتمل الشهر،
    /// ثم يُحتسب بأيامه الباقية فقط (شهر كامل عند التعيين الأول = 14/12).
    #[test]
    fn accrual_hired_this_month_waits_then_counts_partial() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,daily_hours,annual_leave_days,annual_leave_balance,
             active,created_at,updated_at)
             VALUES(1,'1','جديد',8.5,12.0,0.0,1,'2026-02-20T08:00:00','x')",
            [],
        )
        .unwrap();
        // 25 فبراير: آخر شهر مكتمل يناير ← تعيينه فبراير (لم يكتمل) فلا شيء
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 2, 25).unwrap()).unwrap();
        assert_eq!(balance_and_anchor(&conn, 1).0, 0.0, "شهر التعيين لم يكتمل بعد");
        // 1 مارس: فبراير مكتمل ← نسبة الأيام 20..28 = 9/28 من شهر كامل (12÷12=1)
        accrue_leave_impl(&conn, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()).unwrap();
        let (bal, anchor) = balance_and_anchor(&conn, 1);
        assert!((bal - 9.0 / 28.0).abs() < 1e-4, "فبراير 9/28 من الشهر: {bal}");
        assert_eq!(anchor.as_deref(), Some("2026-02"));
    }

    /// أداة تشغيل يدوية (تُستدعى فقط مع `--ignored`): تعيد احتساب فترة على
    /// قاعدة بيانات حقيقية تحدّدها المتغيّرة ATTENDANCE_LIVE_DB وتطبع حالة
    /// أيام الجمعة + صفوف الرواتب. تتجاهل بصمت عند غياب المتغيّرة.
    #[test]
    #[ignore = "أداة يدوية على قاعدة حية — لا تعمل في الاختبار العادي"]
    fn live_recompute_tool() {
        let Ok(path) = std::env::var("ATTENDANCE_LIVE_DB") else {
            return;
        };
        let conn = rusqlite::Connection::open(&path).unwrap();
        // كل النطاق الموجود في attendance_days (يشمل شهوراً سابقة بجمع «غياب»)
        let (from_s, to_s): (String, String) = conn
            .query_row(
                "SELECT MIN(date), MAX(date) FROM attendance_days",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let from = NaiveDate::parse_from_str(&from_s, "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str(&to_s, "%Y-%m-%d").unwrap();
        println!("المدى: {from} ← {to}");
        let state = AppState {
            db: Mutex::new(conn),
            base: std::env::temp_dir(),
        };
        let n = recompute_range(&state, &from, &to).unwrap();
        println!("أُعيد احتساب {} صفاً", n);

        let conn = state.db.lock().unwrap();
        {
            let mut st = conn
                .prepare(
                    "SELECT strftime('%Y-%m', date), status, COUNT(*) FROM attendance_days
                     WHERE strftime('%w', date)='5' GROUP BY 1,2 ORDER BY 1,2",
                )
                .unwrap();
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            });
            for row in rows.unwrap().flatten() {
                println!("جمع {} · {}: {}", row.0, row.1, row.2);
            }
        }
        for r in payroll_impl(&conn, "2026-10").unwrap() {
            if r.pin.as_deref() == Some("80") || r.pin.as_deref() == Some("81") {
                println!(
                    "{} ({}) — أيام {} | أجر {:.2} | بدلات {:.2} | عطلة {:.2} ({}) | صافي {:.2}",
                    r.employee_name,
                    r.pin.clone().unwrap_or_default(),
                    r.days_present,
                    r.base_pay,
                    r.allowances,
                    r.weekend_pay,
                    r.weekend_days_paid,
                    r.net_pay
                );
            }
        }
    }

    // ------------------------------------- تحويل يوم إلى إجازة + أزرار اليوم

    const DAY: &str = "2026-10-19"; // الاثنين — يوم دوام

    /// حالة اختبار: قاعدة في الذاكرة + موظف واحد برصيد وبصمة يوم كامل
    /// (06:00 ← 16:30) لاختبار تحويل اليوم إلى إجازة وأزرار نافذة التعديل.
    fn day_test_state(balance: f64) -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        db::seed_defaults(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,name_en,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,weekend_paid,annual_leave_balance,created_at,updated_at)
             VALUES(1,'1','موظف','Emp',8.5,30,15.0,7.0,0.0,1,1,?1,'x','x')",
            rusqlite::params![balance],
        )
        .unwrap();
        for (dt, pt) in [("2026-10-19T06:00:00", 0i64), ("2026-10-19T16:30:00", 1i64)] {
            conn.execute(
                "INSERT INTO punches(employee_id,pin,device_time,punch_type,synced_at)
                 VALUES(1,'1',?1,?2,'x')",
                rusqlite::params![dt, pt],
            )
            .unwrap();
        }
        AppState {
            db: Mutex::new(conn),
            base: std::env::temp_dir(),
        }
    }

    fn balance_of(state: &AppState) -> f64 {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT annual_leave_balance FROM employees WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn day_status(state: &AppState) -> (String, Option<String>, Option<bool>) {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT status, leave_type, leave_paid FROM attendance_days
             WHERE employee_id=1 AND date=?1",
            [DAY],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, Option<i64>>(2)?.map(|v| v != 0),
                ))
            },
        )
        .unwrap()
    }

    fn worked_of(state: &AppState) -> i64 {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT worked_min FROM attendance_days WHERE employee_id=1 AND date=?1",
            [DAY],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// تحويل يوم دوام إلى إجازة سنوية: خصم يوم من الرصيد، والحالة تصبح
    /// إجازةً رغم وجود البصمات؛ وإزالة الإجازة تُعيد الرصيد والحالة للحضور.
    #[test]
    fn annual_day_conversion_deducts_and_refunds_balance() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(day_status(&state).0, "present", "البصمات قبل التحويل");

        convert_day_to_leave(&state, 1, DAY, "annual").unwrap();
        assert_eq!(balance_of(&state), 4.0, "خصم يوم واحد من الرصيد");
        let (status, lty, paid) = day_status(&state);
        assert_eq!(status, "leave", "الإجازة تتقدّم على البصمات");
        assert_eq!(lty.as_deref(), Some("annual"));
        assert_eq!(paid, Some(true));

        // الرجوع (وضع «تلقائي» في النافذة): إزالة الإجازة + إعادة الاحتساب
        remove_single_day_leave(&state, 1, DAY).unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(balance_of(&state), 5.0, "استعادة الرصيد كاملاً");
        assert_eq!(day_status(&state).0, "present", "البصمات تسترجع الحالة");
    }

    /// رصيد غير كافٍ ← التحويل السنوية يُرفض برسالة واضحة ولا يتغيّر شيء.
    #[test]
    fn annual_day_conversion_refused_without_balance() {
        let state = day_test_state(0.5);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        let err = convert_day_to_leave(&state, 1, DAY, "annual").unwrap_err();
        assert!(err.contains("غير كافٍ"), "رسالة الرصيد: {err}");
        assert_eq!(balance_of(&state), 0.5, "لا خصم عند الرفض");
        let conn = state.db.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM leaves WHERE employee_id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "لا تُنشأ إجازة عند الرفض");
        drop(conn);
        assert_eq!(day_status(&state).0, "present", "اليوم لم يتغيّر");
    }

    /// المرضية بلا خصم، وبلا أجر بلا خصم؛ واستبدال نوع الإجازة يعيد الرصيد
    /// المخصوم للسنوية المدفوعة السابقة فقط.
    #[test]
    fn sick_and_unpaid_conversions_never_touch_balance() {
        let state = day_test_state(5.0);

        convert_day_to_leave(&state, 1, DAY, "sick").unwrap();
        assert_eq!(balance_of(&state), 5.0, "المرضية بلا خصم");
        let (status, lty, paid) = day_status(&state);
        assert_eq!(status, "leave");
        assert_eq!(lty.as_deref(), Some("sick"));
        assert_eq!(paid, Some(true));

        convert_day_to_leave(&state, 1, DAY, "unpaid").unwrap();
        assert_eq!(balance_of(&state), 5.0, "بلا أجر بلا خصم");
        let (status, lty, paid) = day_status(&state);
        assert_eq!(status, "leave");
        assert_eq!(lty.as_deref(), Some("annual"), "تُعرض للرواتب كسنوية");
        assert_eq!(paid, Some(false));

        // إزالتها لا تستعيد شيئاً (لم يُخصم أصلاً)
        remove_single_day_leave(&state, 1, DAY).unwrap();
        assert_eq!(balance_of(&state), 5.0);
    }

    /// يوم داخل إجازة متعددة الأيام ← التحويل يُرفض برسالة تُحيل لشاشة الموظفين.
    #[test]
    fn day_inside_multi_day_leave_conversion_is_refused() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
                 VALUES(1,'2026-10-19','2026-10-21','annual',0,'','x')",
                [],
            )
            .unwrap();
        }
        let err = convert_day_to_leave(&state, 1, "2026-10-20", "annual").unwrap_err();
        assert!(err.contains("متعددة الأيام"), "رسالة الرفض: {err}");
        assert!(err.contains("الموظفون"), "تشير لشاشة تعديل الإجازات");
        assert_eq!(balance_of(&state), 5.0, "لا خصم عند الرفض");
    }

    /// زر «إلغاء خصم الاستراحة»: 630 د بدل 600، يبقى بعد إعادة الاحتساب،
    /// ويعود للخصم عند إلغائه.
    #[test]
    fn no_break_flag_restores_minutes_and_survives_recompute() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(worked_of(&state), 600, "630 − 30 استراحة");

        set_day_flags_impl(&state, 1, DAY, true, false).unwrap();
        assert_eq!(worked_of(&state), 630, "بلا خصم استراحة");

        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(worked_of(&state), 630, "العلم يصمد بعد إعادة الاحتساب");

        set_day_flags_impl(&state, 1, DAY, false, false).unwrap();
        assert_eq!(worked_of(&state), 600, "الرجوع يعيد الخصم");
    }

    /// زر «إزالة بدل الفطور»: يطرح 7 ₪ من البدلات ومن الصافي في الرواتب.
    #[test]
    fn no_meal_day_excluded_from_payroll_allowances() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        let conn = state.db.lock().unwrap();
        let before = payroll_impl(&conn, "2026-10").unwrap();
        conn.execute(
            "UPDATE attendance_days SET no_meal=1 WHERE employee_id=1 AND date=?1",
            [DAY],
        )
        .unwrap();
        let after = payroll_impl(&conn, "2026-10").unwrap();
        assert!((before[0].allowances - after[0].allowances - 7.0).abs() < 1e-9,
            "البدل {} ← {}", before[0].allowances, after[0].allowances);
        assert!((before[0].net_pay - after[0].net_pay - 7.0).abs() < 1e-9,
            "الصافي {} ← {}", before[0].net_pay, after[0].net_pay);
    }

    /// العلمان يظهران في التقرير اليومي (مصدر حالة النافذة في الواجهة).
    #[test]
    fn daily_report_exposes_day_flags() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        set_day_flags_impl(&state, 1, DAY, true, true).unwrap();
        let rows = {
            let conn = state.db.lock().unwrap();
            daily_report_impl(&conn, DAY).unwrap()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["no_break"], true, "إلغاء الاستراحة يظهر");
        assert_eq!(rows[0]["no_meal"], true, "إزالة الفطور يظهر");
        // وقبل التفعيل يظهران false
        set_day_flags_impl(&state, 1, DAY, false, false).unwrap();
        let rows = {
            let conn = state.db.lock().unwrap();
            daily_report_impl(&conn, DAY).unwrap()
        };
        assert_eq!(rows[0]["no_break"], false);
        assert_eq!(rows[0]["no_meal"], false);
    }

    /// تنبيه «خروج مفقود» لا يُطلق على وردية ما زالت جارية (حضور بلا انصراف
    /// بعد) بل فقط بعد أن تنقضي المهلة ويصبح اليوم «بصمة ناقصة».
    #[test]
    fn missing_out_alert_waits_until_day_is_incomplete() {
        let state = day_test_state(5.0);
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO attendance_days(employee_id,date,first_in,worked_min,late_min,
             early_leave_min,overtime_min,absent,status,manual)
             VALUES(1,'2026-10-18','2026-10-18T13:10:00',0,0,0,0,0,'present',0)",
            [],
        )
        .unwrap();
        let fresh = detect_missing_events(&conn, "2026-10-19");
        assert!(fresh.is_empty(), "دوام جارٍ لا يُنبَّه عليه: {fresh:?}");

        conn.execute(
            "UPDATE attendance_days SET status='incomplete' WHERE date='2026-10-18'",
            [],
        )
        .unwrap();
        let fresh = detect_missing_events(&conn, "2026-10-19");
        assert_eq!(fresh.len(), 1, "بعد انتهاء المهلة يُنبَّه: {fresh:?}");
    }

    /// الأيام المستقبلية لا تُسجَّل غياباً: الصفوف المستقبلية القديمة تُنظَّف
    /// عند إعادة الاحتساب ولا يُنشأ غياب بعد اليوم (كان يضخّم ملخّص الشهر).
    #[test]
    fn future_days_are_not_recorded_as_absent() {
        let state = day_test_state(5.0);
        // صف «غياب» مستقبلي قديم — كما كانت تُنشئه إعادة الاحتساب قبل الإصلاح
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO attendance_days(employee_id,date,worked_min,late_min,
                 early_leave_min,overtime_min,absent,status,manual)
                 VALUES(1,'2099-01-05',0,0,0,0,1,'absent',0)",
                [],
            )
            .unwrap();
        }
        let from = NaiveDate::from_ymd_opt(2099, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2099, 1, 31).unwrap();
        recompute_range(&state, &from, &to).unwrap();

        let conn = state.db.lock().unwrap();
        let future_absent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attendance_days
                 WHERE employee_id=1 AND date > '2099-01-04' AND absent=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(future_absent, 0, "لا غياب لأيام لم تأتِ بعد");
        // وعطلة الأسبوع المستقبلية تبقى مسجّلة (معلومة لا غياب)
        let weekend_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attendance_days
                 WHERE employee_id=1 AND date > '2099-01-04' AND status='weekend'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(weekend_rows > 0, "الجمعة المستقبلية تبقى عطلة أسبوعية");
    }
}
