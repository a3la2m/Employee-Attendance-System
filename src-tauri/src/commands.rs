//! أوامر Tauri — الجسر بين React (frontend) ومنطق Rust (backend).
use crate::attendance::{check_device_clock, compute_days_for_employee, pair_punches};
use crate::db;
use crate::models::*;
use crate::payroll::{compute_payroll, expected_workdays_in_month};
use crate::storage;
use crate::zk::make_backend;
use chrono::NaiveDate;
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
        hourly_rate: setting_f(s, "hourly_rate", 10.0),
        meal_allow: setting_f(s, "meal_allow", 7.0),
        transport_allow: setting_f(s, "transport_allow", 5.0),
        overtime_after_hours: setting_f(s, "overtime_after_hours", 8.5),
        overtime_rate_mult: setting_f(s, "overtime_rate_mult", 1.5),
        grace_late_min: setting_i(s, "grace_late_min", 0),
        work_start: setting(s, "work_start", "09:00"),
        work_end: setting(s, "work_end", "17:30"),
    }
}

// ---------------------------------------------------------------- الإعدادات

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    settings_map(&conn).pipe(Ok)
}

#[tauri::command]
pub fn save_setting(
    state: tauri::State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
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
            "SELECT id,pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
                    meal_allow,transport_allow,active,
                    overtime_enabled,meal_enabled,transport_enabled
             FROM employees ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Employee {
                id: r.get(0)?,
                pin: r.get(1)?,
                name: r.get(2)?,
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
             overtime_enabled=?12,meal_enabled=?13,transport_enabled=?14
             WHERE id=?15",
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
                emp.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(emp.id)
    } else {
        conn.execute(
            "INSERT INTO employees(pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,overtime_enabled,meal_enabled,transport_enabled,
             created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
                emp.overtime_enabled as i64,
                emp.meal_enabled as i64,
                emp.transport_enabled as i64,
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
pub fn sync_device_internal(state: &AppState) -> Result<Value, String> {
    let (ip, port, key) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
        )
    };
    let mut backend = make_backend(&ip, port, key, true);
    let info = backend.connect().map_err(|e| e.to_string())?;
    let users = backend.pull_users().map_err(|e| e.to_string())?;
    let punches = backend.pull_attendance().map_err(|e| e.to_string())?;
    backend.close();

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut synced_punches = 0usize;
    let mut added_users = 0usize;
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        // اربط pin → employee_id (إن وُجد)
        let mut pin_to_id: HashMap<String, i64> = HashMap::new();
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

        for (pin, name, _card, _priv) in users {
            if !pin_to_id.contains_key(&pin) {
                // موظف على الجهاز غير مسجّل محلياً → أضفه تمهيدياً
                conn.execute(
                    "INSERT OR IGNORE INTO employees(pin,name,active,created_at,updated_at)
                     VALUES(?1,?2,1,?3,?3)",
                    rusqlite::params![pin, name, now],
                )
                .map_err(|e| e.to_string())?;
                added_users += 1;
            }
        }
        // حدّث خريطة الربط بعد الإضافات
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

        for (pin, dt, punch_type, wc, raw_status) in punches {
            let emp_id = pin_to_id.get(&pin).copied();
            let r = conn.execute(
                "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(pin,device_time) DO NOTHING",
                rusqlite::params![emp_id, pin, dt, punch_type, wc, raw_status, now],
            );
            if let Ok(n) = r {
                synced_punches += n;
            }
        }

        db::set_setting(&conn, "last_sync_at", &now).map_err(|e| e.to_string())?;
    }

    // إعادة احتساب أيام الحضور لآخر 60 يوماً (سريعة وكفيلة)
    let recomputed = recompute_recent(state, 60)?;

    Ok(json!({
        "device": info,
        "new_users_added": added_users,
        "new_punches": synced_punches,
        "days_recomputed": recomputed,
        "last_sync_at": now,
    }))
}

/// أمر Tauri: مزامنة فورية عند طلب الواجهة.
#[tauri::command]
pub fn sync_device(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    sync_device_internal(&state)
}

/// يقرأ ساعة الجهاز ويقارنها بمنطقة Asia/Jerusalem المحددة.
#[tauri::command]
pub fn check_time_sync(state: tauri::State<'_, AppState>) -> Result<Value, String> {
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
    let mut b = make_backend(&ip, port, key, true);
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
    let mut b = make_backend(&ip, port, key, true);
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
    let today = chrono::Utc::now().naive_utc().date();
    let from = today - chrono::Duration::days(days);
    recompute_range(state, &from, &today)
}

/// يعيد احتساب أيام الحضور لموظف/لكل الموظفين بين تاريخين.
fn recompute_range(
    state: &AppState,
    from: &NaiveDate,
    to: &NaiveDate,
) -> Result<usize, String> {
    let (ps, employees) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        let ps = payroll_settings(&s);
        let mut employees: Vec<Employee> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id,pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
                     meal_allow,transport_allow,active,
                     overtime_enabled,meal_enabled,transport_enabled FROM employees",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(Employee {
                        id: r.get(0)?,
                        pin: r.get(1)?,
                        name: r.get(2)?,
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
                    })
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                employees.push(row.map_err(|e| e.to_string())?);
            }
        }
        (ps, employees)
    };

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for emp in employees {
        if !emp.active {
            continue;
        }
        // جلب بصمات هذا الموظف في المدى
        let mut punches: Vec<String> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT device_time FROM punches
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
                |r| r.get::<_, String>(0),
            );
            if let Ok(rows) = rows {
                punches = rows.flatten().collect();
            }
        }
        // جلب الإجازات
        let mut leaves: Vec<(NaiveDate, NaiveDate, String)> = Vec::new();
        {
            let mut st = conn
                .prepare(
                    "SELECT from_date,to_date,leave_type FROM leaves
                     WHERE employee_id=?1 AND to_date>=?2 AND from_date<=?3",
                )
                .map_err(|e| e.to_string())?;
            let rows = st.query_map(rusqlite::params![emp.id, from.to_string(), to.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            });
            if let Ok(rows) = rows {
                for row in rows.flatten() {
                    if let (Ok(a), Ok(b)) = (
                        NaiveDate::parse_from_str(&row.0, "%Y-%m-%d"),
                        NaiveDate::parse_from_str(&row.1, "%Y-%m-%d"),
                    ) {
                        leaves.push((a, b, row.2.clone()));
                    }
                }
            }
        }

        let days = compute_days_for_employee(emp.id, emp.pin.as_deref().unwrap_or(""), &punches, &leaves, &ps, *from, *to);
        for d in days {
            conn.execute(
                "INSERT INTO attendance_days(employee_id,date,first_in,last_out,worked_min,late_min,
                 early_leave_min,overtime_min,absent,leave_type,status)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(employee_id,date) DO UPDATE SET
                   first_in=excluded.first_in, last_out=excluded.last_out,
                   worked_min=excluded.worked_min, late_min=excluded.late_min,
                   early_leave_min=excluded.early_leave_min, overtime_min=excluded.overtime_min,
                   absent=excluded.absent, leave_type=excluded.leave_type, status=excluded.status",
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
                    d.status
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
    let prefix = format!("{}%", period);
    let mut stmt = conn
        .prepare(
            "SELECT id,employee_id,date,first_in,last_out,worked_min,late_min,early_leave_min,
                    overtime_min,absent,leave_type,status
             FROM attendance_days WHERE date LIKE ?1 ORDER BY employee_id, date",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([&prefix], |r| {
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
    let expected = expected_workdays_in_month(period, &weekend);

    let mut stmt = conn
        .prepare(
            "SELECT id,pin,name,job_title,phone,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,
             overtime_enabled,meal_enabled,transport_enabled
             FROM employees WHERE active=1 ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let employees: Vec<Employee> = stmt
        .query_map([], |r| {
            Ok(Employee {
                id: r.get(0)?,
                pin: r.get(1)?,
                name: r.get(2)?,
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
        let res = compute_payroll(
            emp.id,
            &emp.name,
            emp.pin.clone(),
            period,
            &emp_days,
            expected,
            &ps_emp,
        );
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
             overtime_hours,base_pay,allowances,late_deduction,absence_deduction,net_pay,generated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(period,employee_id) DO UPDATE SET
               days_present=excluded.days_present, total_worked_hours=excluded.total_worked_hours,
               overtime_hours=excluded.overtime_hours, base_pay=excluded.base_pay,
               allowances=excluded.allowances, late_deduction=excluded.late_deduction,
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

// ---------------------------------------------------------------- تقرير يومي

#[tauri::command]
pub fn get_daily_report(
    state: tauri::State<'_, AppState>,
    date: String,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.employee_id, e.name, e.pin, a.first_in, a.last_out, a.worked_min,
                    a.late_min, a.early_leave_min, a.overtime_min, a.absent, a.status
             FROM attendance_days a LEFT JOIN employees e ON e.id = a.employee_id
             WHERE a.date = ?1 ORDER BY e.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([&date], |r| {
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
            Ok(Value::Object(m))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------- الإجازات

#[tauri::command]
pub fn add_leave(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    from_date: String,
    to_date: String,
    leave_type: String,
    note: String,
) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,note,created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![employee_id, from_date, to_date, leave_type, note, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn list_leaves(state: tauri::State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT l.id, l.employee_id, e.name, l.from_date, l.to_date, l.leave_type, l.note
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
        }))
    })
    .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ---------------------------------------------------------------- استيراد/تصدير

/// يعيد ملخصاً سريعاً للوحة التحكم.
#[tauri::command]
pub fn dashboard(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let today = chrono::Utc::now().naive_utc().format("%Y-%m-%d").to_string();
    let emp_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM employees WHERE active=1", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM attendance_days WHERE date=?1 AND status IN ('present','late')",
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
