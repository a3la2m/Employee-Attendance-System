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

/// صف وردية كما تُقرأ من قاعدة البيانات (الحقول المستخدمة في الاحتساب).
struct ShiftRow {
    work_start: String,
    work_end: String,
    daily_hours: f64,
    break_min: i64,
}

fn read_shift_row(conn: &Connection, id: i64) -> Option<ShiftRow> {
    conn.query_row(
        "SELECT work_start,work_end,daily_hours,break_min FROM shifts WHERE id=?1",
        [id],
        |r| {
            Ok(ShiftRow {
                work_start: r.get(0)?,
                work_end: r.get(1)?,
                daily_hours: r.get(2)?,
                break_min: r.get(3)?,
            })
        },
    )
    .ok()
}

/// إعدادات الاحتساب لموظف واحد: ورديته (أو الوردية الرئيسية) فوق الإعداد
/// العام. الوردية هي مرجع ساعات الدوام والاستراحة (تضبط حساب دوام الموظف).
fn employee_settings(conn: &Connection, base: &PayrollSettings, employee_id: i64) -> PayrollSettings {
    let shift_id: Option<i64> = conn
        .query_row("SELECT shift_id FROM employees WHERE id=?1", [employee_id], |r| {
            r.get(0)
        })
        .ok()
        .flatten();
    shift_settings(conn, base, shift_id)
}

/// يعيد معرّف الوردية التي يعمل عليها الموظف: ورديته المسنَدة، وإلا الوردية
/// الرئيسية (is_default=1)، وإلا أول وردية مفعّلة. None إن لم توجد أي وردية
/// (حينها يبقى الإعداد العام هو المرجع).
fn effective_shift_id(conn: &Connection, employee_shift: Option<i64>) -> Option<i64> {
    if let Some(id) = employee_shift {
        let exists: bool = conn
            .query_row("SELECT COUNT(*) FROM shifts WHERE id=?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n > 0)
            .unwrap_or(false);
        if exists {
            return Some(id);
        }
    }
    conn.query_row(
        "SELECT id FROM shifts WHERE is_default=1 ORDER BY id LIMIT 1",
        [],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .or_else(|| {
        conn.query_row(
            "SELECT id FROM shifts WHERE active=1 ORDER BY id LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .ok()
    })
}

/// يطبّق وردية الموظف على إعدادات الأساس (الساعات/الاستراحة/الوقتين)، مع
/// احترام التجاوزات الفردية (ساعات/استراحة الموظف إن كانت محدّدة > 0).
pub fn shift_settings(
    conn: &Connection,
    base: &PayrollSettings,
    employee_shift: Option<i64>,
) -> PayrollSettings {
    let mut s = base.clone();
    if let Some(id) = effective_shift_id(conn, employee_shift) {
        if let Some(sh) = read_shift_row(conn, id) {
            s.work_start = sh.work_start;
            s.work_end = sh.work_end;
            s.daily_hours = sh.daily_hours;
            s.break_min = sh.break_min;
        }
    }
    s
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

/// ينقل (يُنسخ) ملفات القاعدة والإعدادات إلى مجلد يختاره المستخدم.
/// يُرجع قائمة الملفات المنسوخة، ويطلب من الواجهة إعادة تشغيل التطبيق للتأكد من العمل بعد النقل.
#[tauri::command]
pub fn export_storage_files(state: tauri::State<'_, AppState>, target_dir: String) -> Result<Vec<String>, String> {
    let t = PathBuf::from(target_dir.trim());
    if t.as_os_str().is_empty() {
        return Err("المسار فارغ".into());
    }
    if !t.exists() {
        std::fs::create_dir_all(&t).map_err(|e| format!("تعذّر إنشاء المجلد: {}", e))?;
    }
    let base = &state.base;
    let active = storage::resolve_dir(base);
    let mut copied: Vec<String> = Vec::new();
    // نسخ قاعدة البيانات النشطة
    if let Ok(conn) = state.db.lock() {
        if let Some(p) = conn.path() {
            let src = PathBuf::from(p);
            if src.exists() {
                let dst = t.join(storage::MAIN_DB);
                std::fs::copy(&src, &dst).map_err(|e| format!("تعذّر نسخ القاعدة: {}", e))?;
                copied.push(dst.display().to_string());
            }
        }
    }
    // نسخ attendance-settings.json إن وُجد
    let sjson_src = active.join(storage::SETTINGS_JSON);
    if sjson_src.exists() {
        let sjson_dst = t.join(storage::SETTINGS_JSON);
        std::fs::copy(&sjson_src, &sjson_dst).map_err(|e| format!("تعذّر نسخ الإعدادات: {}", e))?;
        copied.push(sjson_dst.display().to_string());
    }
    // نسخ storage-config.json إن وُجد
    let cfg_src = storage::config_file(base);
    if cfg_src.exists() {
        let cfg_dst = t.join(storage::STORAGE_CONFIG);
        std::fs::copy(&cfg_src, &cfg_dst).map_err(|e| format!("تعذّر نسخ التهيئة: {}", e))?;
        copied.push(cfg_dst.display().to_string());
    }
    Ok(copied)
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
                    annual_leave_days,leave_accrual_month,shift_id,device_privilege
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
                shift_id: r.get::<_, Option<i64>>(19).ok().flatten(),
                device_privilege: r.get::<_, i64>(20).unwrap_or(0),
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
             annual_leave_balance=?17,annual_leave_days=?18,shift_id=?19,device_privilege=?20
             WHERE id=?21",
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
                emp.shift_id,
                emp.device_privilege,
                emp.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(emp.id)
    } else {
        // موظف جديد بلا وردية → الوردية الرئيسية تلقائياً
        let shift_id = emp.shift_id.or_else(|| {
            conn.query_row("SELECT id FROM shifts WHERE is_default=1 ORDER BY id LIMIT 1", [], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
        });
        conn.execute(
            "INSERT INTO employees(pin,name,name_en,job_title,phone,daily_hours,break_min,hourly_rate,
             meal_allow,transport_allow,active,overtime_enabled,meal_enabled,transport_enabled,
             weekend_paid,annual_leave_balance,annual_leave_days,shift_id,device_privilege,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
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
                shift_id,
                emp.device_privilege,
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

// ---------------------------------------------------------------- الورديات

/// يسرد كل الورديات مع عدد الموظفين المسنَدين لكل وردية (رقم إعلامي).
pub fn list_shifts_impl(conn: &Connection) -> Result<Vec<Shift>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id,s.name,s.work_start,s.work_end,s.daily_hours,s.break_min,
                    s.is_default,s.active,
                    (SELECT COUNT(*) FROM employees e WHERE e.shift_id=s.id)
             FROM shifts s ORDER BY s.is_default DESC, s.active DESC, s.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Shift {
                id: r.get(0)?,
                name: r.get(1)?,
                work_start: r.get(2)?,
                work_end: r.get(3)?,
                daily_hours: r.get(4)?,
                break_min: r.get(5)?,
                is_default: r.get::<_, i64>(6)? != 0,
                active: r.get::<_, i64>(7)? != 0,
                employee_count: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[tauri::command]
pub fn list_shifts(state: tauri::State<'_, AppState>) -> Result<Vec<Shift>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    list_shifts_impl(&conn)
}

/// ينشئ/يحدّث وردية. اسم الوردية إلزامي، والأوقات تُطبَّع إلى HH:MM،
/// وتُتحقق صحّتها. لا يمكن تعطيل/حذف الوردية الرئيسية إن كانت الافتراضية.
pub fn upsert_shift_impl(conn: &Connection, shift: Shift) -> Result<i64, String> {
    let name = shift.name.trim().to_string();
    if name.is_empty() {
        return Err("اسم الوردية مطلوب".into());
    }
    if parse_hhmm(&shift.work_start).is_none() {
        return Err("بداية الدوام غير صالحة (HH:MM)".into());
    }
    if parse_hhmm(&shift.work_end).is_none() {
        return Err("نهاية الدوام غير صالحة (HH:MM)".into());
    }
    if shift.work_start.trim() == shift.work_end.trim() {
        return Err("بداية الدوام ونهايته لا يتطابقان".into());
    }
    if shift.daily_hours <= 0.0 {
        return Err("ساعات الدوام الأساسية يجب أن تكون أكبر من صفر".into());
    }
    if shift.break_min < 0 {
        return Err("الاستراحة لا تكون سالبة".into());
    }
    let ws = normalize_time(&shift.work_start);
    let we = normalize_time(&shift.work_end);
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    if shift.id > 0 {
        conn.execute(
            "UPDATE shifts SET name=?1,work_start=?2,work_end=?3,daily_hours=?4,break_min=?5,
             active=?6,updated_at=?7 WHERE id=?8",
            rusqlite::params![
                name,
                ws,
                we,
                shift.daily_hours,
                shift.break_min,
                shift.active as i64,
                now,
                shift.id
            ],
        )
        .map_err(|e| e.to_string())?;
        // لا بدّ من وردية رئيسية واحدة دائماً: إن كان هذا الافتراضية فلا تُعطَّل
        if shift.is_default {
            conn.execute("UPDATE shifts SET is_default=0 WHERE id<>?1", [shift.id])
                .map_err(|e| e.to_string())?;
            conn.execute("UPDATE shifts SET is_default=1 WHERE id=?1", [shift.id])
                .map_err(|e| e.to_string())?;
        }
        Ok(shift.id)
    } else {
        // أول وردية تُنشأ؟ تصبح الرئيسية تلقائياً
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM shifts", [], |r| r.get(0))
            .unwrap_or(0);
        let is_default = if shift.is_default || count == 0 { 1 } else { 0 };
        conn.execute(
            "INSERT INTO shifts(name,work_start,work_end,daily_hours,break_min,is_default,active,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,1,?7,?7)",
            rusqlite::params![
                name,
                ws,
                we,
                shift.daily_hours,
                shift.break_min,
                is_default,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        let id = conn.last_insert_rowid();
        if is_default == 1 {
            conn.execute("UPDATE shifts SET is_default=0 WHERE id<>?1", [id])
                .map_err(|e| e.to_string())?;
        }
        Ok(id)
    }
}

#[tauri::command]
pub fn upsert_shift(state: tauri::State<'_, AppState>, shift: Shift) -> Result<i64, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    upsert_shift_impl(&conn, shift)
}

/// يجعل الوردية الرئيسية (يرثها كل موظف غير مسنَد) وإلا يبقى الإعداد العام.
pub fn set_default_shift_impl(conn: &Connection, id: i64) -> Result<(), String> {
    let exists: i64 = conn
        .query_row("SELECT COUNT(*) FROM shifts WHERE id=?1", [id], |r| r.get(0))
        .unwrap_or(0);
    if exists == 0 {
        return Err("الوردية غير موجودة".into());
    }
    conn.execute("UPDATE shifts SET is_default=0", [])
        .map_err(|e| e.to_string())?;
    conn.execute("UPDATE shifts SET is_default=1,active=1 WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_default_shift(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    set_default_shift_impl(&conn, id)
}

/// يحذف وردية. يُنزع إسناد كل موظف منها (يصبحون على الوردية الرئيسية).
/// لا يمكن حذف آخر وردية في النظام.
pub fn delete_shift_impl(conn: &Connection, id: i64) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM shifts", [], |r| r.get(0))
        .unwrap_or(0);
    if count <= 1 {
        return Err("لا يمكن حذف آخر وردية في النظام".into());
    }
    conn.execute("UPDATE employees SET shift_id=NULL WHERE shift_id=?1", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM shifts WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    // إن كانت الوردية المحذوفة هي الرئيسية: عيّن أقل وردية مفعّلة كرئيسية
    let has_default: i64 = conn
        .query_row("SELECT COUNT(*) FROM shifts WHERE is_default=1", [], |r| r.get(0))
        .unwrap_or(0);
    if has_default == 0 {
        let _ = conn.execute(
            "UPDATE shifts SET is_default=1 WHERE id=(SELECT id FROM shifts WHERE active=1 ORDER BY id LIMIT 1)",
            [],
        );
    }
    Ok(())
}

#[tauri::command]
pub fn delete_shift(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    delete_shift_impl(&conn, id)
}

/// يُسنِد قائمة موظفين إلى وردية (إسناد جماعي). `shift_id=NULL` يعيدهم
/// للوردية الرئيسية. يعيد عدد الموظفين الذين تغيّرت ورديتهم فعلاً.
pub fn assign_employees_to_shift_impl(
    conn: &Connection,
    employee_ids: &[i64],
    shift_id: Option<i64>,
) -> Result<usize, String> {
    if let Some(id) = shift_id {
        let exists: i64 = conn
            .query_row("SELECT COUNT(*) FROM shifts WHERE id=?1", [id], |r| r.get(0))
            .unwrap_or(0);
        if exists == 0 {
            return Err("الوردية غير موجودة".into());
        }
    }
    let mut changed = 0usize;
    for eid in employee_ids {
        let n = conn
            .execute(
                "UPDATE employees SET shift_id=?1 WHERE id=?2 AND IFNULL(shift_id,-1)<>IFNULL(?1,-1)",
                rusqlite::params![shift_id, eid],
            )
            .map_err(|e| e.to_string())?;
        changed += n;
    }
    Ok(changed)
}

#[tauri::command]
pub fn assign_employees_to_shift(
    state: tauri::State<'_, AppState>,
    employee_ids: Vec<i64>,
    shift_id: Option<i64>,
) -> Result<usize, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    assign_employees_to_shift_impl(&conn, &employee_ids, shift_id)
}

/// يعيد احتساب الحضور لكل الموظفين في المدى — يُستخدم بعد تغيير وردية/إسناد.
#[tauri::command]
pub fn reassign_shift_recompute(
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<usize, String> {
    let f = NaiveDate::parse_from_str(&from, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let t = NaiveDate::parse_from_str(&to, "%Y-%m-%d").map_err(|e| e.to_string())?;
    recompute_range(&state, &f, &t)
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
/// فحص ساعة الجهاز وتصحيحها تلقائياً إن انحرفت أكثر من دقيقتين (بوّابة
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
    if diff.abs() <= 2 {
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

        for (pin, name, _card, pv) in &users {
            if !pin_to_id.contains_key(pin) {
                // موظف على الجهاز غير مسجّل محلياً → أضفه تمهيدياً
                // (بلا اسم على الجهاز؟ نستخدم الرقم الوظيفي كاسم ظاهر)
                let display = if name.trim().is_empty() {
                    pin.clone()
                } else {
                    name.clone()
                };
                conn.execute(
                    "INSERT OR IGNORE INTO employees(pin,name,active,device_privilege,created_at,updated_at)
                     VALUES(?1,?2,1,?3,?4,?4)",
                    rusqlite::params![pin, display, pv, now],
                )
                .map_err(|e| e.to_string())?;
                added_users += 1;
            } else if let Some(&emp_id) = pin_to_id.get(pin) {
                // مزامنة صلاحية الجهاز ← البرنامج: الجهاز هو المرجع (ما يظهر
                // على شاشة الموظفين هو ما عليه فعلياً على جهاز البصمة).
                let _ = conn.execute(
                    "UPDATE employees SET device_privilege=?1
                     WHERE id=?2 AND device_privilege<>?1",
                    rusqlite::params![pv, emp_id],
                );
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
pub fn get_next_device_pin(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let (ip, port, key) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings_map(&conn);
        (
            setting(&s, "device_ip", "192.168.1.205"),
            setting_i(&s, "device_port", 4370) as u16,
            setting_i(&s, "device_comm_key", 0) as u32,
        )
    };
    let mut backend = make_backend(&ip, port, key, false);
    if let Err(_) = backend.connect() {
        // fallback to local max
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut max_pin: i64 = 0;
        let mut st = conn.prepare("SELECT pin FROM employees WHERE pin IS NOT NULL AND pin <> ''").map_err(|e| e.to_string())?;
        if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
            for r in rows.flatten() {
                if let Ok(n) = r.parse::<i64>() {
                    if n > max_pin { max_pin = n; }
                }
            }
        }
        backend.close();
        return Ok((max_pin + 1).to_string());
    }
    let users = backend.pull_users().map_err(|e| e.to_string())?;
    backend.close();
    let mut max_pin: i64 = 0;
    for (pin, _name, _card, _priv) in &users {
        if let Ok(n) = pin.parse::<i64>() {
            if n > max_pin { max_pin = n; }
        }
    }
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let mut st = conn.prepare("SELECT pin FROM employees WHERE pin IS NOT NULL AND pin <> ''").map_err(|e| e.to_string())?;
        let rows: Vec<String> = st.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?
            .flatten()
            .collect();
        for r in rows {
            if let Ok(n) = r.parse::<i64>() {
                if n > max_pin { max_pin = n; }
            }
        }
    }
    Ok((max_pin + 1).to_string())
}

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

/// تغيير صلاحية موظف على جهاز البصمة: 0 = مستخدم عادي، 6 = مدير،
/// 14 = أدمن كامل (سوبر أدمن). الكتابة تحافظ على الاسم والبطاقة والبصمات.
#[tauri::command]
pub fn set_device_privilege(
    state: tauri::State<'_, AppState>,
    pin: String,
    privilege: i64,
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
    backend
        .set_user_privilege(&pin, privilege)
        .map_err(|e| e.to_string())?;
    backend.close();
    Ok(format!(
        "حُدِّثت صلاحية الرقم {pin} على الجهاز ({})",
        match privilege {
            14 => "أدمن كامل",
            6 => "مدير",
            _ => "مستخدم عادي",
        }
    ))
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
/// يعيد احتساب آخر `days` يوماً حتى اليوم (يُستدعى بعد المزامنة وعند الإقلاع).
pub fn recompute_recent(state: &AppState, days: i64) -> Result<usize, String> {
    // تاريخ اليوم بالمنطقة الزمنية المحددة (لا UTC) حتى يشمل الاحتساب صف اليوم
    // حتى بعد منتصف الليل المحلي — مهم لتحوّل الدوام الجاري إلى «بصمة ناقصة».
    let today = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        today_in_tz(&settings_map(&conn))
    };
    let from = today - chrono::Duration::days(days);
    recompute_range(state, &from, &today)
}

/// يلغي الإجازة عن أي يوم **دوّم فيه الموظف فعلياً** (له بصمات) داخل المدى:
/// - إجازة يوم واحد ← تُحذف، وإجازة متعددة الأيام ← تُشطر (يُستبعد اليوم منها).
/// - السنوية **المدفوعة** يُعاد لها يوم في الرصيد (كان خُصم عند التسجيل).
/// - يُحفظ تنبيهاً في الجرس يشرح ما حُصل (بلا تكرار لنفس اليوم).
/// - الاستثناء: إجازة أنشأها المسؤول صراحةً ليوم له بصمات (keep_punches=1،
///   من «تحويل اليوم إلى إجازة» في شاشة الحضور) فلا تُلغى تلقائياً.
///
/// يُستدعى داخل إعادة الاحتساب قبل قراءة الإجازات، فتُحتسب الأيام الملغاة حضوراً.
fn resolve_work_on_leave(
    conn: &Connection,
    employee_id: i64,
    name: &str,
    punches: &[(String, i64)],
    from: NaiveDate,
    to: NaiveDate,
) {
    // تواريخ البصمات داخل المدى (المفتاح YYYY-MM-DD من الطابع الزمني)
    let mut punch_dates: std::collections::BTreeSet<NaiveDate> = Default::default();
    for (t, _) in punches {
        if let Some(day) = t.get(0..10) {
            if let Ok(d) = NaiveDate::parse_from_str(day, "%Y-%m-%d") {
                punch_dates.insert(d);
            }
        }
    }
    if punch_dates.is_empty() {
        return;
    }

    // حماية من أي حلقة لا نهائية: كل دورة تُزيل يوماً واحداً على الأكثر
    let max_steps = 400usize;
    for _ in 0..max_steps {
        // اقرأ الإجازات غير المحميّة المتقاطعة مع المدى (حالة جديدة كل دورة)
        let leaves: Vec<(i64, String, String, String, bool)> = {
            let Ok(mut st) = conn.prepare(
                "SELECT id,from_date,to_date,leave_type,paid FROM leaves
                 WHERE employee_id=?1 AND to_date>=?2 AND from_date<=?3
                   AND IFNULL(keep_punches,0)=0",
            ) else {
                return;
            };
            let rows = st.query_map(
                rusqlite::params![employee_id, from.to_string(), to.to_string()],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                },
            );
            match rows {
                Ok(rows) => rows.flatten().collect(),
                Err(_) => return,
            }
        };

        // أول يوم تعارض: بصمة داخل نطاق إجازة
        let mut hit: Option<(i64, NaiveDate, NaiveDate, String, bool, NaiveDate)> = None;
        'outer: for (lid, f, t, ty, paid) in &leaves {
            let (Ok(lf), Ok(lt)) = (
                NaiveDate::parse_from_str(f, "%Y-%m-%d"),
                NaiveDate::parse_from_str(t, "%Y-%m-%d"),
            ) else {
                continue;
            };
            let start = lf.max(from);
            let end = lt.min(to);
            let mut d = start;
            while d <= end {
                if punch_dates.contains(&d) {
                    hit = Some((*lid, lf, lt, ty.clone(), *paid, d));
                    break 'outer;
                }
                d = match d.succ_opt() {
                    Some(n) => n,
                    None => break,
                };
            }
        }
        let Some((lid, lf, lt, ty, paid, d)) = hit else {
            break; // لا مزيد من التعارضات
        };

        // استبعد اليوم من الإجازة: حذف ليوم واحد، أو شطر للمتعددة
        let ds = d.format("%Y-%m-%d").to_string();
        if lf == lt {
            if conn.execute("DELETE FROM leaves WHERE id=?1", [lid]).is_err() {
                return;
            }
        } else if d == lf {
            if conn
                .execute(
                    "UPDATE leaves SET from_date=?1 WHERE id=?2",
                    rusqlite::params![d.succ_opt().map(|x| x.to_string()), lid],
                )
                .is_err()
            {
                return;
            }
        } else if d == lt {
            if conn
                .execute(
                    "UPDATE leaves SET to_date=?1 WHERE id=?2",
                    rusqlite::params![d.pred_opt().map(|x| x.to_string()), lid],
                )
                .is_err()
            {
                return;
            }
        } else {
            // الشطر الثاني [d+1 → lt] يُنسخ كما هو ثم يُقصَّر الأصل إلى [lf → d-1]
            if conn
                .execute(
                    "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,
                       comp_for_date,keep_punches,created_at)
                     SELECT employee_id,?1,to_date,leave_type,paid,note,comp_for_date,keep_punches,created_at
                     FROM leaves WHERE id=?2",
                    rusqlite::params![d.succ_opt().map(|x| x.to_string()), lid],
                )
                .is_err()
            {
                return;
            }
            if conn
                .execute(
                    "UPDATE leaves SET to_date=?1 WHERE id=?2",
                    rusqlite::params![d.pred_opt().map(|x| x.to_string()), lid],
                )
                .is_err()
            {
                return;
            }
        }

        // السنوية المدفوعة: كان خُصم يوم عند التسجيل ← يُعاد للرصيد
        let refunded = ty == "annual" && paid;
        if refunded {
            let _ = conn.execute(
                "UPDATE employees SET annual_leave_balance = annual_leave_balance + 1,
                 updated_at=?1 WHERE id=?2",
                rusqlite::params![
                    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                    employee_id
                ],
            );
        }

        let ty_name = match ty.as_str() {
            "annual" => "سنوية",
            "sick" => "مرضية",
            "comp" => "عطلة تعويضية",
            _ => "بدون أجر",
        };
        let tail = if refunded {
            " وأُعيد يوم إلى رصيد الإجازات السنوية"
        } else {
            ""
        };
        insert_notification(
            conn,
            "leave_work",
            Some(employee_id),
            &format!("دوام في يوم إجازة: {name}"),
            &format!(
                "دوّم {name} يوم {ds} أثناء إجازة {ty_name} — أُلغيت الإجازة عن هذا اليوم{tail}، وأُعيد احتسابه حضوراً"
            ),
            &ds,
        );
    }
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
                     annual_leave_days,leave_accrual_month,shift_id
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
                        shift_id: r.get::<_, Option<i64>>(18).ok().flatten(),
                        // هذه الاستعلامات لا تقرأ الصلاحية — تُعامَل كمستخدم عادي
                        device_privilege: 0,
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
        // جلب بصمات هذا الموظف في المدى (مع نوع كل بصمة للازدواج in/out).
        // نوسّع النافذة يوماً قبل ويوماً بعد: في الوردية الليلية قد يبدأ الدوام
        // آخر اليوم السابق (دخول) وينتهي صباح اليوم التالي (خروج) — فنضمن
        // التقاط الطرفين مهما ضاق مدى إعادة الحساب.
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
            let from_s = format!("{}T00:00:00", *from - chrono::Duration::days(1));
            let to_s = format!("{}T23:59:59", *to + chrono::Duration::days(1));
            let pin = emp.pin.clone().unwrap_or_default();
            let rows = st.query_map(
                rusqlite::params![from_s, to_s, emp.id, pin],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            );
            if let Ok(rows) = rows {
                punches = rows.flatten().collect();
            }
        }
        // دوام فعلي في يوم مغطّى بإجازة: تُلغى الإجازة ذلك اليوم ويُعاد يوم
        // الرصيد السنوية المدفوعة (مع تنبيه في الجرس)، ثم يُعاد احتساب اليوم
        // حضوراً — إلا إجازة أنشأها المسؤول صراحةً ليوم له بصمات (keep_punches).
        resolve_work_on_leave(&conn, emp.id, &emp.name, &punches, *from, *to);

        // جلب الإجازات (مع حالة الدفع) بعد الإلغاء التلقائي أعلاه
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

        // إعدادات هذا الموظف: ورديته (أو الوردية الرئيسية) — هي مرجع ساعات
        // الدوام والاستراحة (تضبط حساب دوام الموظف).
        let ps_emp = shift_settings(&conn, &ps, emp.shift_id);
        let days = compute_days_for_employee(emp.id, emp.pin.as_deref().unwrap_or(""), &punches, &leaves, &holidays, &weekend, &no_break_dates, &ps_emp, *from, *to, now);
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
            "SELECT a.id,a.employee_id,a.date,a.first_in,a.last_out,a.worked_min,a.late_min,
                    a.early_leave_min,a.overtime_min,a.absent,a.leave_type,a.status,a.manual,
                    a.leave_paid,(h.date IS NOT NULL),a.holiday_work_choice
             FROM attendance_days a LEFT JOIN holidays h ON h.date = a.date
             WHERE a.date >= ?1 AND a.date < ?2 ORDER BY a.employee_id, a.date",
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
                holiday: r.get::<_, i64>(14).unwrap_or(0) != 0,
                holiday_work_choice: r.get::<_, Option<String>>(15).ok().flatten(),
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
             annual_leave_days,leave_accrual_month,shift_id
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
                shift_id: r.get::<_, Option<i64>>(18).ok().flatten(),
                // استعلام الرواتب لا يقرأ الصلاحية — تُعامَل كمستخدم عادي
                device_privilege: 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut out = Vec::new();
    for emp in employees {
        let emp_days: Vec<AttendanceDay> =
            all_days.iter().filter(|d| d.employee_id == emp.id).cloned().collect();
        // إعدادات هذا الموظف: ورديته (أو الوردية الرئيسية) — مرجع ساعات الدوام
        // والاستراحة. ثم تُطبَّق تجاوزات الأجر/البدلات الفردية أدناه.
        let mut ps_emp = shift_settings(conn, &ps, emp.shift_id);
        if emp.hourly_rate > 0.0 {
            ps_emp.hourly_rate = emp.hourly_rate;
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

// ------------------------------------------------ دفع الرواتب + لقطة القسيمة
//
// حالة «مدفوع» الوحيدة في النظام (الأرشفة payroll_runs تجميد أرقام لا دفع).
// ولقطة القسيمة تحفظ إعدادات الطباعة ونسبها لحظة الطباعة/الدفع — كي تبقى
// القسيمة قابلة لإثبات ما طُبع فعلاً حتى لو تغيّرت الإعدادات لاحقاً.

/// تعليم راتب موظف في شهر بعينه كمدفوع — يحفظ المبلغ وطريقة الدفع وتاريخها.
/// إعادة التعليم لنفس (موظف، شهر) تستبدل السجل ولا تكرّره.
fn mark_salary_paid_impl(
    conn: &Connection,
    employee_id: i64,
    period: &str,
    net: f64,
    method: Option<String>,
    note: Option<String>,
) -> Result<String, String> {
    let paid_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO salary_payments(employee_id,period,net,method,note,paid_at)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(employee_id,period) DO UPDATE SET
           net=excluded.net, method=excluded.method, note=excluded.note,
           paid_at=excluded.paid_at",
        rusqlite::params![employee_id, period, net, method, note, paid_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(paid_at)
}

/// إلغاء تعليم الراتب كمدفوع — يعود الصافي إلى «غير مدفوع» (عدد الصفوف المحذوفة).
fn unmark_salary_paid_impl(
    conn: &Connection,
    employee_id: i64,
    period: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM salary_payments WHERE employee_id=?1 AND period=?2",
        rusqlite::params![employee_id, period],
    )
    .map_err(|e| e.to_string())
}

/// مدفوعات شهر بعينه — أو كل الفترات إن فُتحت الفترة (الأحدث أولاً).
/// مع اسم الموظف من جدول الموظفين (LEFT JOIN — اسم اختياري للموظف المحذوف).
fn salary_payments_impl(conn: &Connection, period: Option<String>) -> Result<Vec<Value>, String> {
    let map = |r: &rusqlite::Row| {
        Ok(json!({
            "employee_id": r.get::<_, i64>(0)?,
            "period": r.get::<_, String>(1)?,
            "net": r.get::<_, f64>(2)?,
            "method": r.get::<_, Option<String>>(3)?,
            "note": r.get::<_, Option<String>>(4)?,
            "paid_at": r.get::<_, String>(5)?,
            "employee_name": r.get::<_, Option<String>>(6)?,
        }))
    };
    let rows = if let Some(p) = period {
        let mut stmt = conn
            .prepare(
                "SELECT p.employee_id, p.period, p.net, p.method, p.note, p.paid_at, e.name
                 FROM salary_payments p LEFT JOIN employees e ON e.id = p.employee_id
                 WHERE p.period=?1 ORDER BY p.employee_id",
            )
            .map_err(|e| e.to_string())?;
        let q = stmt.query_map([p], map).map_err(|e| e.to_string())?;
        q.flatten().collect::<Vec<_>>()
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT p.employee_id, p.period, p.net, p.method, p.note, p.paid_at, e.name
                 FROM salary_payments p LEFT JOIN employees e ON e.id = p.employee_id
                 ORDER BY p.period DESC, p.employee_id",
            )
            .map_err(|e| e.to_string())?;
        let q = stmt.query_map([], map).map_err(|e| e.to_string())?;
        q.flatten().collect::<Vec<_>>()
    };
    Ok(rows)
}

/// حفظ لقطة إعدادات الطباعة ونسبها وأرقام القسيمة — لحظة الطباعة أو لحظة
/// الدفع. تستبدل لقطة (موظف، شهر) السابقة إن وُجدت. تُرجع وقت الحفظ (ISO).
fn save_payslip_snapshot_impl(conn: &Connection, s: &PayslipSnapshot) -> Result<String, String> {
    let printed_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO payslip_snapshots(employee_id,period,rate,ot_mult,p_work,p_risk,p_bonus,
          basic,overtime,weekend_pay,allowances,holiday_work,comp_off,
          share_work,share_risk,share_bonus,net,printed_at,enabled,sheet)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
         ON CONFLICT(employee_id,period) DO UPDATE SET
           rate=excluded.rate, ot_mult=excluded.ot_mult, p_work=excluded.p_work,
           p_risk=excluded.p_risk, p_bonus=excluded.p_bonus, basic=excluded.basic,
           overtime=excluded.overtime, weekend_pay=excluded.weekend_pay,
           allowances=excluded.allowances, holiday_work=excluded.holiday_work,
           comp_off=excluded.comp_off, share_work=excluded.share_work,
           share_risk=excluded.share_risk, share_bonus=excluded.share_bonus,
           net=excluded.net, printed_at=excluded.printed_at,
           enabled=excluded.enabled, sheet=excluded.sheet",
        rusqlite::params![
            s.employee_id,
            s.period,
            s.rate,
            s.ot_mult,
            s.p_work,
            s.p_risk,
            s.p_bonus,
            s.basic,
            s.overtime,
            s.weekend_pay,
            s.allowances,
            s.holiday_work,
            s.comp_off,
            s.share_work,
            s.share_risk,
            s.share_bonus,
            s.net,
            printed_at,
            s.enabled,
            s.sheet
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(printed_at)
}

/// لقطات القسيمة لشهر بعينه — أو كل الفترات إن فُتحت الفترة (الأحدث أولاً).
fn payslip_snapshots_impl(conn: &Connection, period: Option<String>) -> Result<Vec<Value>, String> {
    const COLS: &str = "employee_id,period,rate,ot_mult,p_work,p_risk,p_bonus,
                        basic,overtime,weekend_pay,allowances,holiday_work,comp_off,
                        share_work,share_risk,share_bonus,net,printed_at,enabled,sheet";
    let map = |r: &rusqlite::Row| {
        Ok(json!({
            "employee_id": r.get::<_, i64>(0)?,
            "period": r.get::<_, String>(1)?,
            "rate": r.get::<_, f64>(2)?,
            "ot_mult": r.get::<_, f64>(3)?,
            "p_work": r.get::<_, f64>(4)?,
            "p_risk": r.get::<_, f64>(5)?,
            "p_bonus": r.get::<_, f64>(6)?,
            "basic": r.get::<_, f64>(7)?,
            "overtime": r.get::<_, f64>(8)?,
            "weekend_pay": r.get::<_, f64>(9)?,
            "allowances": r.get::<_, f64>(10)?,
            "holiday_work": r.get::<_, f64>(11)?,
            "comp_off": r.get::<_, f64>(12)?,
            "share_work": r.get::<_, f64>(13)?,
            "share_risk": r.get::<_, f64>(14)?,
            "share_bonus": r.get::<_, f64>(15)?,
            "net": r.get::<_, f64>(16)?,
            "printed_at": r.get::<_, String>(17)?,
            "enabled": r.get::<_, bool>(18)?,
            "sheet": r.get::<_, Option<String>>(19)?,
        }))
    };
    let rows = if let Some(p) = period {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLS} FROM payslip_snapshots WHERE period=?1 ORDER BY employee_id"
            ))
            .map_err(|e| e.to_string())?;
        let q = stmt.query_map([p], map).map_err(|e| e.to_string())?;
        q.flatten().collect::<Vec<_>>()
    } else {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLS} FROM payslip_snapshots ORDER BY period DESC, employee_id"
            ))
            .map_err(|e| e.to_string())?;
        let q = stmt.query_map([], map).map_err(|e| e.to_string())?;
        q.flatten().collect::<Vec<_>>()
    };
    Ok(rows)
}

#[tauri::command]
pub fn mark_salary_paid(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    period: String,
    net: f64,
    method: Option<String>,
    note: Option<String>,
) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    mark_salary_paid_impl(&conn, employee_id, &period, net, method, note)
}

#[tauri::command]
pub fn unmark_salary_paid(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    period: String,
) -> Result<usize, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    unmark_salary_paid_impl(&conn, employee_id, &period)
}

#[tauri::command]
pub fn salary_payments(
    state: tauri::State<'_, AppState>,
    period: Option<String>,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    salary_payments_impl(&conn, period)
}

#[tauri::command]
pub fn save_payslip_snapshot(
    state: tauri::State<'_, AppState>,
    snap: PayslipSnapshot,
) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    save_payslip_snapshot_impl(&conn, &snap)
}

#[tauri::command]
pub fn payslip_snapshots(
    state: tauri::State<'_, AppState>,
    period: Option<String>,
) -> Result<Vec<Value>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    payslip_snapshots_impl(&conn, period)
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

/// النواة المشتركة للتقرير اليومي (تتضمن أعلام اليوم: no_break / no_meal، وهل
/// التاريخ عطلة رسمية، ويوم البدل المسند لدوام العطلة أو العطلة التي يبدّلها).
fn daily_report_impl(conn: &Connection, date: &str) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.employee_id, e.name, e.pin, a.first_in, a.last_out, a.worked_min,
                    a.late_min, a.early_leave_min, a.overtime_min, a.absent, a.status, a.manual,
                    a.leave_type, a.leave_paid, a.no_break, a.no_meal,
                    (SELECT COUNT(*) FROM holidays h WHERE h.date = a.date),
                    (SELECT l.from_date FROM leaves l
                       WHERE l.employee_id = a.employee_id AND l.comp_for_date = a.date LIMIT 1),
                    (SELECT l.comp_for_date FROM leaves l
                       WHERE l.employee_id = a.employee_id AND l.leave_type = 'comp'
                         AND l.from_date <= a.date AND l.to_date >= a.date LIMIT 1),
                    a.holiday_work_choice
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
            m.insert("holiday".into(), json!(r.get::<_, i64>(17).unwrap_or(0) != 0));
            m.insert("comp_off".into(), json!(r.get::<_, Option<String>>(18)?));
            m.insert("comp_for".into(), json!(r.get::<_, Option<String>>(19)?));
            m.insert("holiday_work_choice".into(), json!(r.get::<_, Option<String>>(20)?));
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
            "SELECT a.id,a.employee_id,a.date,a.first_in,a.last_out,a.worked_min,a.late_min,
                    a.early_leave_min,a.overtime_min,a.absent,a.leave_type,a.status,a.manual,
                    a.leave_paid,(h.date IS NOT NULL),a.holiday_work_choice
             FROM attendance_days a LEFT JOIN holidays h ON h.date = a.date
             WHERE a.employee_id=?1 AND a.date=?2",
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
            holiday: r.get::<_, i64>(14).unwrap_or(0) != 0,
            holiday_work_choice: r.get::<_, Option<String>>(15).ok().flatten(),
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
    set_day_manual_impl(&state, employee_id, &date, &mode, first_in, last_out)
}

/// جسم `set_day_manual` على `&AppState` ليعمل من الاختبارات مباشرة.
fn set_day_manual_impl(
    state: &AppState,
    employee_id: i64,
    date: &str,
    mode: &str,
    first_in: Option<String>,
    last_out: Option<String>,
) -> Result<Value, String> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    match mode {
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
            let fin_raw = first_in
                .filter(|s| !s.trim().is_empty())
                .ok_or("أدخل وقت الحضور (HH:MM)")?;
            let (fh, fm) =
                parse_hhmm(&fin_raw).ok_or("صيغة وقت الحضور يجب أن تكون HH:MM")?;
            // انصرافٌ أصغر من الحضور = دوامٌ يعبر منتصف الليل: يُنسب الانصراف
            // ليومه التالي وتُحتسب الجلسة كاملة ليوم الدخول (مثال: دخول 22:00
            // وخروج 02:00 صباح الغد = أربع ساعات). مهلة الدوام الجارٍ (14 سا)
            // سقفٌ للمدة المقبولة هنا: ما زاد عنها أقرب لخطأ إدخال (مثل
            // 06:00 ← 02:30) فيُرفض برسالة واضحة — قبل أي تغيير في القاعدة.
            if let Some(parsed_out) = last_out
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .and_then(parse_hhmm)
            {
                if parsed_out < (fh, fm) {
                    let span_min = (24 * 60)
                        - (fh as i64 * 60 + fm as i64)
                        + (parsed_out.0 as i64 * 60 + parsed_out.1 as i64);
                    if span_min > crate::attendance::OPEN_PUNCH_GRACE_MIN {
                        return Err(format!(
                            "الانصراف {:02}:{:02} قبل الحضور {fh:02}:{fm:02} بمدة {:.1} ساعة \
                             (تُحسب ليوم الغد) وحدود المدة 14 ساعة — راجع الوقتين",
                            parsed_out.0,
                            parsed_out.1,
                            span_min as f64 / 60.0
                        ));
                    }
                }
            }
            remove_single_day_leave(state, employee_id, date)?;
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
            let out_date = if (lh, lm) < (fh, fm) {
                d.succ_opt()
                    .ok_or("التاريخ خارج النطاق")?
                    .format("%Y-%m-%d")
                    .to_string()
            } else {
                date.to_string()
            };
            let s = settings_map(&conn);
            let ps = employee_settings(&conn, &payroll_settings(&s), employee_id);
            let weekend =
                crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
            let punches = vec![
                (format!("{date}T{fh:02}:{fm:02}:00"), 0i64),
                (format!("{out_date}T{lh:02}:{lm:02}:00"), 1i64),
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
        // 4) إنشاء إجازة ليوم واحد — وإن كان اليوم له بصمات (المسؤول يحوّل
        //    يوم دوام إلى إجازة صراحةً) تُوسم الإجازة keep_punches فلا تُلغيها
        //    إعادة الاحتساب التلقائية بسبب الدوام.
        let note = match kind {
            "annual" => "تحويل يوم إلى إجازة سنوية من شاشة الحضور",
            "sick" => "تحويل يوم إلى إجازة مرضية من شاشة الحضور",
            _ => "تحويل يوم إلى إجازة بدون أجر من شاشة الحضور",
        };
        let has_punches: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM punches
                 WHERE employee_id=?1 AND device_time>=?2 AND device_time<=?3",
                rusqlite::params![
                    employee_id,
                    format!("{date}T00:00:00"),
                    format!("{date}T23:59:59")
                ],
                |r| r.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,keep_punches,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                employee_id,
                date,
                date,
                leave_type,
                paid as i64,
                note,
                (has_punches > 0) as i64,
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
                {
                    // تُعاد الاشتقاق من الوقتين المخزّنين كما هما (نوعاً بصمة):
                    // قد يعبر الانصراف منتصف الليل فيكون تاريخه التالي، ولا
                    // نطويه على تاريخ اليوم وإلا انعدمت الجلسة.
                    let s = settings_map(&conn);
                    let ps = employee_settings(&conn, &payroll_settings(&s), employee_id);
                    let weekend =
                        crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
                    let punches = vec![(fi, 0i64), (fo, 1i64)];
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

    // 3) دوام في عطلة رسمية اختار المسؤول له تعويض «يوم بدل» ولم يُسند بعد:
    //    نُنبّه المسؤول ليمنح الموظف يوماً بديلاً (يمكن في الشهر التالي) من
    //    شاشة الحضور — زر «يوم بدل» على صف اليوم.
    if let Ok(mut st) = conn.prepare(
        "SELECT a.employee_id, e.name, a.date FROM attendance_days a
         JOIN employees e ON e.id = a.employee_id
         WHERE a.date <= ?1 AND a.first_in IS NOT NULL AND e.active = 1
           AND a.holiday_work_choice = 'comp'
           AND a.date IN (SELECT date FROM holidays)
           AND NOT EXISTS (SELECT 1 FROM leaves l
                WHERE l.employee_id = a.employee_id AND l.comp_for_date = a.date)
         ORDER BY a.date DESC LIMIT 90",
    ) {
        if let Ok(rows) = st.query_map([today], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for (id, name, date) in rows.flatten() {
                let title = format!("يوم بدل مطلوب: {name}");
                let body = format!(
                    "دوّم {name} يوم {date} (عطلة رسمية) واخترت له تعويض «يوم بدل»: امنحه يوم راحة تعويضياً (يمكن في الشهر التالي) من زر «يوم بدل» في سجل الحضور"
                );
                if insert_notification(conn, "comp_off", Some(id), &title, &body, &date) {
                    fresh.push(format!("{name} — يوم بدل عن دوام {date}"));
                }
            }
        }
    }

    // 4) غياب اليوم: موظف مفعّل مرّ عليه يوم دوام (ليس عطلة/إجازة) بلا بصمة
    //    ولا إجازة مسجّلة → تنبيه غياب. يُحفظ ليومه فلا يتكرر.
    if let Ok(mut st) = conn.prepare(
        "SELECT a.employee_id, e.name, a.date
         FROM attendance_days a JOIN employees e ON e.id = a.employee_id
         WHERE a.date = ?1 AND e.active = 1 AND a.status = 'absent' AND a.absent = 1
           AND a.leave_type IS NULL AND a.manual = 0
         ORDER BY e.name",
    ) {
        if let Ok(rows) = st.query_map([today], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) {
            for (id, name, date) in rows.flatten() {
                let title = format!("غياب: {name}");
                let body = format!("{name} غاب اليوم {date} دون بصمة ولا إجازة مسجّلة");
                if insert_notification(conn, "absent", Some(id), &title, &body, &date) {
                    fresh.push(format!("{name} — غياب {date}"));
                }
            }
        }
    }

    // 5) انصراف مبكر: أُغلق اليوم بخروج قبل نهاية الوردية (early_leave_min>0).
    //    يُحفظ ليومه فلا يتكرر.
    if let Ok(mut st) = conn.prepare(
        "SELECT a.employee_id, e.name, a.date, a.early_leave_min, COALESCE(a.last_out,'')
         FROM attendance_days a JOIN employees e ON e.id = a.employee_id
         WHERE a.date = ?1 AND e.active = 1 AND a.early_leave_min > 0
           AND a.manual = 0
         ORDER BY a.early_leave_min DESC",
    ) {
        if let Ok(rows) = st.query_map([today], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
            ))
        }) {
            for (id, name, date, early, lout) in rows.flatten() {
                let t = lout
                    .split('T')
                    .nth(1)
                    .map(|x| x[..5.min(x.len())].to_string())
                    .unwrap_or_default();
                let title = format!("انصراف مبكر: {name}");
                let body = format!(
                    "انصرف {name} يوم {date}{} مبكراً بـ {early} دقيقة عن نهاية ورديته",
                    if t.is_empty() {
                        String::new()
                    } else {
                        format!(" الساعة {t}")
                    }
                );
                if insert_notification(conn, "early_leave", Some(id), &title, &body, &date) {
                    fresh.push(format!("{name} — انصراف مبكر {early} دقيقة ({date})"));
                }
            }
        }
    }

    // 6) لا بصمات اليوم: لم تصل أي بصمة من أي موظف فعال اليوم — قد يكون الجهاز
    //    غير متصل/معطّل. تنبيه عام (بلا موظف) يُحفظ ليومه.
    let any_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM punches WHERE substr(device_time,1,10) = ?1",
            [today],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if any_today == 0 {
        let title = "لا بصمات اليوم".to_string();
        let body = format!("لم تصل أي بصمة من الجهاز اليوم {today} — تحقق من اتصال جهاز البصمة والمزامنة");
        if insert_notification(conn, "no_punches", None, &title, &body, today) {
            fresh.push(format!("لا بصمات اليوم ({today})"));
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
            "SELECT l.id, l.employee_id, e.name, l.from_date, l.to_date, l.leave_type, l.note, l.paid,
                    l.comp_for_date
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
            "comp_for_date": r.get::<_, Option<String>>(8)?,
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

/// يضبط خيار التعويض لليوم الذي داوم فيه الموظف في عطلة رسمية:
/// `choice = "money"` (نقدي، 62 + الساعات×1.5) أو `"comp"` (يوم بدل).
/// يُضبط لكل موظف/يوم على حدة من شاشة الحضور، ثم يُعاد احتساب اليوم.
#[tauri::command]
pub fn set_holiday_work_choice(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    date: String,
    choice: String,
) -> Result<Value, String> {
    set_holiday_work_choice_impl(&state, employee_id, &date, &choice)
}

/// نواة ضبط خيار التعويض (بلا غلاف tauri لتُختبر مباشرة).
fn set_holiday_work_choice_impl(
    state: &AppState,
    employee_id: i64,
    date: &str,
    choice: &str,
) -> Result<Value, String> {
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "تاريخ غير صالح")?;
    let choice = match choice.trim() {
        "money" => "money",
        "comp" => "comp",
        // "none"/"" = بلا خيار صريح (الافتراضي نقدي عند الحساب)
        "" | "none" => "",
        _ => return Err("خيار غير معروف — المتاح: money أو comp".into()),
    };
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        // يوم عطلة رسمية مسجّل؟
        let is_holiday: i64 = conn
            .query_row("SELECT COUNT(*) FROM holidays WHERE date=?1", [date], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if is_holiday == 0 {
            return Err(format!("{date} ليس عطلة رسمية مسجّلة"));
        }
        // لا يجوز ربط الخيار بيوم صار فيه يوم بدل مسند (تعارض تعويض مزدوج)
        if choice == "comp" {
            let already: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM leaves WHERE employee_id=?1 AND comp_for_date=?2",
                    rusqlite::params![employee_id, date],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if already > 0 {
                return Err(format!(
                    "يوجد يوم بدل مسند عن عطلة {date} — احذفه أولاً لتغيير الخيار"
                ));
            }
        }
        let val: Option<&str> = if choice.is_empty() { None } else { Some(choice) };
        conn.execute(
            "UPDATE attendance_days SET holiday_work_choice=?3
             WHERE employee_id=?1 AND date=?2",
            rusqlite::params![employee_id, date, val],
        )
        .map_err(|e| e.to_string())?;
    }
    // أعِد احتساب اليوم ليظهر الأثر في الحضور والراتب فوراً
    recompute_range(state, &d, &d)?;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let name: String = conn
        .query_row("SELECT name FROM employees WHERE id=?1", [employee_id], |r| r.get(0))
        .unwrap_or_default();
    drop(conn);
    Ok(json!({
        "employee_id": employee_id,
        "employee_name": name,
        "date": date,
        "choice": choice,
    }))
}

/// إسناد «يوم بدل» (عطلة تعويضية) لموظف دوّم في عطلة رسمية واختير له تعويض
/// «يوم بدل»: يُدفع ليوم البدل أجر يوم العطلة (62)، ويمكن أن يكون اليوم **في
/// الشهر التالي**. يسجّل إجازة نوع `comp` على تاريخ البدل مع
/// `comp_for_date` = تاريخ العطلة التي عوّضها، ثم يعيد احتساب يوم البدل
/// ليظهر عطلة تعويضية في الحضور والراتب.
#[tauri::command]
pub fn assign_comp_off(
    state: tauri::State<'_, AppState>,
    employee_id: i64,
    holiday_date: String,
    comp_date: String,
) -> Result<Value, String> {
    assign_comp_off_impl(&state, employee_id, &holiday_date, &comp_date)
}

/// نواة إسناد يوم البدل (بلا غلاف tauri لتُختبر مباشرة).
fn assign_comp_off_impl(
    state: &AppState,
    employee_id: i64,
    holiday_date: &str,
    comp_date: &str,
) -> Result<Value, String> {
    let hd = NaiveDate::parse_from_str(holiday_date, "%Y-%m-%d")
        .map_err(|_| "تاريخ العطلة غير صالح")?;
    let cd = NaiveDate::parse_from_str(comp_date, "%Y-%m-%d")
        .map_err(|_| "تاريخ يوم البدل غير صالح")?;
    // يوم البدل تعويضي: يجب أن يكون بعد تاريخ العطلة نفسها (يوم استراحة
    // في المستقبل بدلاً من دوام يوم العطلة) — لا يصح قبلها أو يومها.
    if cd <= hd {
        return Err(format!(
            "يوم البدل {comp_date} يجب أن يكون بعد تاريخ العطلة {holiday_date}"
        ));
    }
    let new_id;
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;

        // 1) التاريخ عطلة رسمية مسجّلة؟
        let is_holiday: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM holidays WHERE date=?1",
                [&holiday_date],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if is_holiday == 0 {
            return Err(format!("{holiday_date} ليس عطلة رسمية مسجّلة"));
        }

        // 2) الموظف دوّم فعلاً في تلك العطلة (له حضور مسجّل)؟
        let worked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attendance_days
                 WHERE employee_id=?1 AND date=?2 AND first_in IS NOT NULL",
                rusqlite::params![employee_id, holiday_date],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if worked == 0 {
            return Err(format!("لا يوجد دوام مسجّل للموظف في عطلة {holiday_date}"));
        }

        // 3) لا يوم بدل سابق لنفس العطلة
        let already: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM leaves WHERE employee_id=?1 AND comp_for_date=?2",
                rusqlite::params![employee_id, holiday_date],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if already > 0 {
            return Err(format!(
                "سبق إسناد يوم بدل عن عطلة {holiday_date} — احذفه من بطاقة الموظف لتغييره"
            ));
        }

        // 4) يوم البدل غير محجوز بإجازة أخرى
        let busy: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM leaves
                 WHERE employee_id=?1 AND from_date<=?2 AND to_date>=?2",
                rusqlite::params![employee_id, comp_date],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if busy > 0 {
            return Err(format!("اليوم {comp_date} محجوز بإجازة أخرى لهذا الموظف"));
        }

        // 5) تسجيل يوم البدل (لا يُخصم من أي رصيد)
        let note = format!("عطلة تعويضية عن دوام عطلة رسمية {holiday_date}");
        conn.execute(
            "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,
               comp_for_date,created_at)
             VALUES(?1,?2,?2,'comp',1,?3,?4,?5)",
            rusqlite::params![
                employee_id,
                comp_date,
                note,
                holiday_date,
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()
            ],
        )
        .map_err(|e| e.to_string())?;
        new_id = conn.last_insert_rowid();
    }

    // احتسب يوم البدل (ويوم العطلة) لتظهر الحالة الجديدة فوراً
    recompute_range(state, &cd, &cd)?;
    recompute_range(state, &hd, &hd)?;

    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let name: String = conn
        .query_row("SELECT name FROM employees WHERE id=?1", [employee_id], |r| r.get(0))
        .unwrap_or_default();
    drop(conn);
    Ok(json!({
        "id": new_id,
        "employee_id": employee_id,
        "name": name,
        "holiday_date": holiday_date,
        "comp_date": comp_date,
    }))
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

// ------------------------------------------------------------ بيانات تجريبية

/// بيانات تجريبية لأول تشغيل: تُنشأ **مرة واحدة فقط** إذا كانت القاعدة فارغة
/// تماماً (لا موظفين ولا بصمات) — كي تظهر بطاقات لوحة التحكم والرسوم حيّة
/// بدل «—» حتى أول مزامنة حقيقية. تُعلَّم بـ demo_seeded فلا تعود تتكرر،
/// ويمكن حذفها من الإعدادات (زر «حذف البيانات التجريبية»).
/// تعيد عدد الموظفين التجريبيين المُنشأين.
pub fn seed_demo_if_empty(conn: &Connection) -> Result<usize, String> {
    let s = settings_map(conn);
    if setting(&s, "demo_seeded", "0") == "1" {
        return Ok(0);
    }
    // قاعدة فيها موظفون حقيقيون؟ لا نُدخل تجريبياً فوق بياناتك أبداً.
    let real: i64 = conn
        .query_row("SELECT COUNT(*) FROM employees", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if real > 0 {
        db::set_setting(conn, "demo_seeded", "1").map_err(|e| e.to_string())?;
        return Ok(0);
    }

    let s = settings_map(conn);
    let now = now_in_tz(&s);
    let today = now.date();
    let weekend = crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
    let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    // (pin, اسم, اسم لاتيني, وظيفة, ساعات/يوم, سعر الساعة)
    let demo: [(&str, &str, &str, &str, f64, f64); 5] = [
        ("9001", "علاء عبدالله", "alaa", "مدير عام", 8.5, 25.0),
        ("9002", "أحمد سمير", "ahmad", "محاسب", 8.5, 15.0),
        ("9003", "سارة خالد", "sara", "موظفة استقبال", 8.0, 12.0),
        ("9004", "محمد يوسف", "mohammed", "فني صيانة", 9.0, 18.0),
        ("9005", "ليلى حسن", "laila", "مسؤولة مخزن", 8.0, 11.0),
    ];

    let mut ids: Vec<i64> = Vec::new();
    for (pin, name, name_en, job, hours, rate) in demo.iter() {
        conn.execute(
            "INSERT INTO employees(pin,name,name_en,job_title,daily_hours,hourly_rate,active,
             overtime_enabled,meal_enabled,transport_enabled,weekend_paid,
             annual_leave_balance,annual_leave_days,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,1,1,1,1,1,14,14,?7,?7)",
            rusqlite::params![pin, name, name_en, job, hours, rate, now_str],
        )
        .map_err(|e| e.to_string())?;
        ids.push(conn.last_insert_rowid());
    }

    // بصمات آخر 7 أيام بتوقيت المنطقة: دخول قريب من البداية وخروج بعد النهاية،
    // مع تنويع واقعي (تأخير، خروج مبكر، غياب يوم، بصمة ناقصة) لتعبئة الرسوم.
    // اليوم لا نضيف ما لم يحن وقته بعد حتى لا تظهر بصمات في المستقبل.
    let mut punches = 0usize;
    for off in 0..7i64 {
        let d = today - chrono::Duration::days(off);
        if weekend.contains(&d.weekday().num_days_from_monday()) {
            continue;
        }
        for (i, id) in ids.iter().enumerate() {
            // غياب تجريبي: الموظف الثالث يوم قبل أمس
            if i == 2 && off == 2 {
                continue;
            }
            let (in_h, in_m, out_h, out_m) = match i {
                0 => (8, 53, 17, 32), // منتظم
                1 => (9, 18, 17, 30), // متأخر دائماً
                2 => (8, 58, 17, 31),
                3 => (8, 45, 16, 10), // خروج مبكر
                _ => (9, 0, 17, 35),
            };
            let pin = demo[i].0;
            // بصمة ناقصة (دخول بلا خروج): الموظف الخامس أمس
            let skip_out = i == 4 && off == 1;
            let mk = |d: chrono::NaiveDate, h: i32, m: i32| -> (String, i64) {
                (
                    d.and_hms_opt(h as u32, m as u32, (i as u64 * 7 % 60) as u32)
                        .unwrap()
                        .format("%Y-%m-%dT%H:%M:%S")
                        .to_string(),
                    if h < 12 { 0 } else { 1 }, // 0=دخول 1=خروج
                )
            };
            // لا نسجّل بصمة في مستقبل اليوم: ننتظر حلول وقتها
            let due = |h: i32, m: i32| {
                off > 0
                    || now.time()
                        >= d.and_hms_opt(h as u32, m as u32, 0).unwrap().time()
            };
            if due(in_h, in_m) {
                let (t, pt) = mk(d, in_h, in_m);
                let r = conn.execute(
                    "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
                     VALUES(?1,?2,?3,?4,0,0,?5) ON CONFLICT(pin,device_time) DO NOTHING",
                    rusqlite::params![id, pin, t, pt, now_str],
                );
                if let Ok(n) = r {
                    punches += n;
                }
            }
            if !skip_out && due(out_h, out_m) {
                let (t, pt) = mk(d, out_h, out_m);
                let r = conn.execute(
                    "INSERT INTO punches(employee_id,pin,device_time,punch_type,work_code,raw_status,synced_at)
                     VALUES(?1,?2,?3,?4,0,0,?5) ON CONFLICT(pin,device_time) DO NOTHING",
                    rusqlite::params![id, pin, t, pt, now_str],
                );
                if let Ok(n) = r {
                    punches += n;
                }
            }
        }
    }

    db::set_setting(conn, "demo_seeded", "1").map_err(|e| e.to_string())?;
    let id_list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    db::set_setting(conn, "demo_seeds", &id_list).map_err(|e| e.to_string())?;
    log::info!("seeded {} demo employees, {punches} demo punches", ids.len());
    Ok(ids.len())
}

/// حذف البيانات التجريبية (موظفو pins 9001..9005 وبصماتهم وأيامهم) — زر في
/// الإعدادات. لا تمسّ الموظفين الحقيقيين مهما كان أعدادهم.
#[tauri::command]
pub fn clear_demo_data(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let seeds = setting(&settings_map(&conn), "demo_seeds", "");
    if seeds.is_empty() {
        return Ok(json!({"deleted": 0}));
    }
    let mut deleted = 0usize;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for id in seeds.split(',').filter_map(|x| x.trim().parse::<i64>().ok()) {
        deleted += tx
            .execute("DELETE FROM punches WHERE employee_id=?1", [&id])
            .map_err(|e| e.to_string())?;
        let _ = tx.execute("DELETE FROM attendance_days WHERE employee_id=?1", [&id]);
        let n = tx
            .execute("DELETE FROM employees WHERE id=?1", [&id])
            .map_err(|e| e.to_string())?;
        if n > 0 {
            deleted += n;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    db::set_setting(&conn, "demo_seeds", "").map_err(|e| e.to_string())?;
    Ok(json!({"deleted": deleted}))
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
    /// المتوقّع لكل منهما: أجر اليوم 165 ₪ (8 سا أساسية + 2 سا إضافي
    /// عند 1.5×) + بدل فطور 7 ₪، والجمع 5 × 62، والصافي 4782 ₪ —
    /// بالطريقة الأسبوعية الافتراضية (62 ÷ أيام السبت–الخميس).
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
                    // 600 د فعلية − 480 د حد الدوام (8.5 سا − استراحة) = 120 د إضافي
                    // والأجر 165 ₪ (8 سا × 15 + 2 سا × 15 × 1.5)
                    if d.status != "present" || d.worked_min != 600 || d.overtime_min != 120 {
                        problems.push(format!(
                            "{who} · {}: الحالة {} وساعات فعلية {} (إضافي {}) — المتوقّع حاضر 600 د بإضافي 120",
                            d.date, d.status, d.worked_min, d.overtime_min
                        ));
                    }
                    if (daily - 165.0).abs() > 1e-9 {
                        problems.push(format!(
                            "{who} · {}: أجر اليوم {:.2} ₪ بدل 165 ₪",
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
                (format!("{who} — أجر الدوام (26 × 165)"), r.base_pay, 4290.0),
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

    /// إدخال يدوي عابر لمنتصف الليل: الانصراف أصغر من الحضور يُنسب ليوم الغد
    /// ويحتسب كل ساعات الجلسة ليوم الدخول (22:00 ← 02:00 = 4 سا − 30 د).
    #[test]
    fn manual_overnight_entry_counts_session_on_entry_day() {
        let state = day_test_state(5.0);
        set_day_manual_impl(
            &state,
            1,
            DAY,
            "present",
            Some("22:00".into()),
            Some("02:00".into()),
        )
        .unwrap();
        let conn = state.db.lock().unwrap();
        let (fi, fo, worked, status, manual): (String, String, i64, String, i64) = conn
            .query_row(
                "SELECT first_in,last_out,worked_min,status,manual FROM attendance_days
                 WHERE employee_id=1 AND date=?1",
                [DAY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(fi, "2026-10-19T22:00:00", "الحضور في يومه");
        assert_eq!(fo, "2026-10-20T02:00:00", "الانصراف منسوب ليوم الغد");
        assert_eq!(worked, 210, "أربع ساعات − استراحة 30 د");
        assert_eq!(status, "present");
        assert_eq!(manual, 1, "تعديل يدوي مثبّت");
    }

    /// إدخالٌ يبني مدة تتجاوز 14 ساعة عبر الليل (06:00 ← 02:30) يُرفض برسالة
    /// واضحة **قبل** أي تغيير في القاعدة (لا يُمسّ اليوم ولا الرصيد).
    #[test]
    fn manual_overnight_entry_over_14h_rejected_without_touching_data() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        // اليوم إجازة سنوية أولاً — لو قبل الإدخال لقُلِّص الرصيد أو أُزيلت الإجازة
        convert_day_to_leave(&state, 1, DAY, "annual").unwrap();
        assert_eq!(balance_of(&state), 4.0, "خصم يوم الإجازة");

        let err = set_day_manual_impl(
            &state,
            1,
            DAY,
            "present",
            Some("06:00".into()),
            Some("02:30".into()),
        )
        .unwrap_err();
        assert!(err.contains("14"), "الرسالة تشير لحد المدة: {err}");
        assert_eq!(balance_of(&state), 4.0, "لا لمس للرصيد عند الرفض");
        let (leave,): (Option<String>,) = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT leave_type FROM attendance_days WHERE employee_id=1 AND date=?1",
                [DAY],
                |r| Ok((r.get(0)?,)),
            )
            .unwrap();
        assert_eq!(leave.as_deref(), Some("annual"), "الإجازة باقية كما كانت");
    }

    /// الدخول بلا انصراف بعد الإدخال الليلي لا يُفسد الجلسة: تبديل علم
    /// الاستراحة على صف يدوي عابر لمنتصف الليل يُبقي الجلسة كاملة.
    #[test]
    fn overnight_manual_row_keeps_hours_when_toggling_break() {
        let state = day_test_state(5.0);
        set_day_manual_impl(
            &state,
            1,
            DAY,
            "present",
            Some("22:00".into()),
            Some("02:00".into()),
        )
        .unwrap();
        assert_eq!(worked_of(&state), 210, "الجلسة قبل التبديل");
        set_day_flags_impl(&state, 1, DAY, true, false).unwrap();
        assert_eq!(worked_of(&state), 240, "بلا خصم استراحة — الجلسة كاملة");
        set_day_flags_impl(&state, 1, DAY, false, false).unwrap();
        assert_eq!(worked_of(&state), 210, "الرجوع يعيد الخصم");
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

    // --------------------------------- دوام في يوم إجازة + العطلة الرسمية

    /// دوام فعلي في يوم إجازة سنوية مدفوعة: تُلغى الإجازة ويُعاد الرصيد
    /// ويصبح اليوم حضوراً — مع تنبيه محفوظ في الجرس.
    #[test]
    fn working_on_paid_annual_leave_cancels_and_refunds() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
                 VALUES(1,'2026-10-19','2026-10-19','annual',1,'','x')",
                [],
            )
            .unwrap();
            // الرصيد كان خُصم عند تسجيل الإجازة يدوياً
            conn.execute(
                "UPDATE employees SET annual_leave_balance=4.0 WHERE id=1",
                [],
            )
            .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        assert_eq!(day_status(&state).0, "present", "أُعيد احتسابه حضوراً");
        assert_eq!(balance_of(&state), 5.0, "أُعيد يوم الرصيد السنوية");
        let conn = state.db.lock().unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM leaves WHERE employee_id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "حُذفت إجازة اليوم الواحد");
        let notif: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE kind='leave_work' AND day=?1",
                [DAY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(notif, 1, "تنبيه الإلغاء محفوظ");
    }

    /// الإجازة غير المدفوعة (أو المرضية بلا خصم) تُلغى عند الدوام بلا مساس بالرصيد.
    #[test]
    fn working_on_unpaid_leave_cancels_without_refund() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
                 VALUES(1,'2026-10-19','2026-10-19','annual',0,'','x')",
                [],
            )
            .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(day_status(&state).0, "present");
        assert_eq!(balance_of(&state), 5.0, "لا استعادة لما لم يُخصم");
    }

    /// الوردية الليلية عبر إعادة الحساب الكاملة: دوام 20:00→04:00 يُسجَّل
    /// ليوم الوردية واحد (لا ينشطر على يومين ولا يظهر «خروج بلا دخول» صباحاً)،
    /// وتُجمع ساعاته كاملة.
    #[test]
    fn overnight_shift_recompute_keeps_one_row() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            // الوردية هي مصدر ساعات الدوام الآن: نضبط الوردية الرئيسية ليلية.
            conn.execute(
                "UPDATE shifts SET work_start='20:00', work_end='04:00' WHERE is_default=1",
                [],
            )
            .unwrap();
            conn.execute("DELETE FROM punches", []).unwrap();
            for (dt, pt) in [("2026-10-19T20:00:00", 0i64), ("2026-10-20T04:00:00", 1i64)] {
                conn.execute(
                    "INSERT INTO punches(employee_id,pin,device_time,punch_type,synced_at)
                     VALUES(1,'1',?1,?2,'x')",
                    rusqlite::params![dt, pt],
                )
                .unwrap();
            }
        }
        let from = NaiveDate::parse_from_str("2026-10-19", "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str("2026-10-20", "%Y-%m-%d").unwrap();
        recompute_range(&state, &from, &to).unwrap();

        let conn = state.db.lock().unwrap();
        let shift: (String, Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT status, first_in, last_out, worked_min FROM attendance_days
                 WHERE employee_id=1 AND date='2026-10-19'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(shift.0, "present");
        assert_eq!(shift.1.as_deref(), Some("2026-10-19T20:00:00"));
        assert_eq!(shift.2.as_deref(), Some("2026-10-20T04:00:00"));
        assert_eq!(shift.3, 450, "8 ساعات − استراحة 30 د");

        // يوم 20 (صباحاً) لا يحمل بصمة → لا «بصمة ناقصة» ولا خروج بلا دخول
        let next: (String, Option<String>) = conn
            .query_row(
                "SELECT status, first_in FROM attendance_days
                 WHERE employee_id=1 AND date='2026-10-20'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_ne!(next.0, "incomplete", "لا خروج بلا دخول");
        assert!(next.1.is_none());
    }

    /// إجازة متعددة الأيام ودوام في وسطها: تُشطر الإجازة (يبقى ما قبل اليوم
    /// وما بعده) ويُعاد يوم واحد للرصيد ويصبح يوم الدوام حضوراً.
    #[test]
    fn working_in_middle_of_multi_day_leave_splits_it() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO leaves(employee_id,from_date,to_date,leave_type,paid,note,created_at)
                 VALUES(1,'2026-10-17','2026-10-21','annual',1,'','x')",
                [],
            )
            .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        assert_eq!(day_status(&state).0, "present", "يوم الدوام حضور");
        assert_eq!(balance_of(&state), 6.0, "يوم واحد فقط أُعيد");
        let conn = state.db.lock().unwrap();
        let mut st = conn
            .prepare("SELECT from_date,to_date FROM leaves WHERE employee_id=1 ORDER BY from_date")
            .unwrap();
        let ranges: Vec<(String, String)> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(
            ranges,
            vec![
                ("2026-10-17".to_string(), "2026-10-18".to_string()),
                ("2026-10-20".to_string(), "2026-10-21".to_string()),
            ],
            "الإجازة انشطرت حول يوم الدوام"
        );
    }

    /// استثناء مهم: تحويل المسؤول ليوم له بصمات إلى إجازة (keep_punches) لا
    /// يلغيه الاحتساب التلقائي — يبقى إجازة كما أراد المسؤول.
    #[test]
    fn admin_conversion_of_worked_day_is_not_auto_cancelled() {
        let state = day_test_state(5.0);
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        convert_day_to_leave(&state, 1, DAY, "annual").unwrap();
        assert_eq!(day_status(&state).0, "leave");
        // إعادة احتساب إضافية (كما في المزامنة) لا تُلغي التحويل
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(day_status(&state).0, "leave", "التحويل اليدوي محفوظ");
        assert_eq!(balance_of(&state), 4.0, "لا استعادة للرصيد");
    }

    /// «يوم بدل» لموظف دوّم في عطلة رسمية (سياسة comp): يسجّل عطلة تعويضية
    /// في تاريخ لاحق (يمكن في الشهر التالي)، ويُدفع لها أجر يوم العطلة 62.
    #[test]
    fn comp_off_assignment_creates_paid_rest_day() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')",
                [DAY],
            )
            .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assert_eq!(day_status(&state).0, "present", "الدوام في العطلة حضور");

        // إسناد يوم بدل في 2026-11-03 (الشهر التالي)
        let r = assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();
        assert_eq!(r["comp_date"], "2026-11-03");

        {
            let conn = state.db.lock().unwrap();
            let row: (String, String, i64, Option<String>) = conn
                .query_row(
                    "SELECT leave_type, from_date, paid, comp_for_date FROM leaves
                     WHERE employee_id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!(row.0, "comp");
            assert_eq!(row.1, "2026-11-03");
            assert_eq!(row.2, 1, "مدفوع");
            assert_eq!(row.3.as_deref(), Some(DAY), "مرتبط بعطلة دوامه");
        }
        // يوم البدل نفسه صار عطلة تعويضية في الحضور
        {
            let conn = state.db.lock().unwrap();
            let st: (String, String) = conn
                .query_row(
                    "SELECT status, leave_type FROM attendance_days
                     WHERE employee_id=1 AND date='2026-11-03'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(st, ("leave".to_string(), "comp".to_string()));
        }

        // إسناد ثانٍ لنفس العطلة يُرفض
        let err = assign_comp_off_impl(&state, 1, DAY, "2026-11-04").unwrap_err();
        assert!(err.contains("سبق إسناد"), "رسالة الرفض: {err}");
    }

    /// يوم البدل يجب أن يكون بعد تاريخ العطلة (يوم استراحة في المستقبل) —
    /// رفض يوم قبلها أو يومها نفسها.
    #[test]
    fn comp_off_date_must_be_after_holiday() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        // يوم العطلة نفسه مرفوض
        let err = assign_comp_off_impl(&state, 1, DAY, DAY).unwrap_err();
        assert!(err.contains("يجب أن يكون بعد"), "رسالة الرفض: {err}");
        // تاريخ قبل العطلة مرفوض
        let err = assign_comp_off_impl(&state, 1, DAY, "2026-10-10").unwrap_err();
        assert!(err.contains("يجب أن يكون بعد"), "رسالة الرفض: {err}");
        // بعدها يُقبل
        assign_comp_off_impl(&state, 1, DAY, "2026-10-27").unwrap();
    }

    /// سياسة comp: تنبيه «يوم بدل مطلوب» يظهر حتى يُسند، ويختفي بعده؛
    /// ولا يظهر إطلاقاً في السياسة النقدية.
    #[test]
    fn comp_off_notification_appears_until_assigned() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        // الافتراضي «نقدي» (بلا خيار صريح): لا تنبيه يوم بدل
        {
            let conn = state.db.lock().unwrap();
            let fresh = detect_missing_events(&conn, DAY);
            assert!(!fresh.iter().any(|f| f.contains("يوم بدل")), "{fresh:?}");
        }

        // اختيار المسؤول «يوم بدل» لهذا اليوم ← التنبيه يظهر
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE attendance_days SET holiday_work_choice='comp'
                 WHERE employee_id=1 AND date=?1",
                [DAY],
            )
            .unwrap();
            let fresh = detect_missing_events(&conn, DAY);
            assert!(
                fresh.iter().any(|f| f.contains("يوم بدل")),
                "تنبيه يوم البدل مطلوب: {fresh:?}"
            );
        }

        // بعد الإسناد يختفي
        assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();
        {
            let conn = state.db.lock().unwrap();
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM attendance_days a
                     JOIN employees e ON e.id=a.employee_id
                     WHERE a.date=?1 AND a.first_in IS NOT NULL AND e.active=1
                       AND a.date IN (SELECT date FROM holidays)
                       AND NOT EXISTS (SELECT 1 FROM leaves l
                            WHERE l.employee_id=a.employee_id AND l.comp_for_date=a.date)",
                    [DAY],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "لم يعد هناك دوام عطلة بلا يوم بدل");
        }
    }

    /// سياسة «نقدي»: الراتب يضم62 + ساعات×1.5 بدل الأجر العادي ليوم العطلة.
    #[test]
    fn holiday_work_payflows_through_payroll() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        let conn = state.db.lock().unwrap();
        let r = &payroll_impl(&conn, "2026-10").unwrap()[0];
        // الموظف الاختباري: 15 ₪/ساعة، بدل فطور 7 — يوم العطلة 6 ساعات عمل
        // فعلية (10.5 سا − 30 د استراحة = 600 د):62 + (10 × 15 × 1.5) = 287
        assert_eq!(r.holiday_work_days, 1);
        assert!((r.holiday_work_pay - 287.0).abs() < 1e-6, "{}", r.holiday_work_pay);
        assert!((r.base_pay).abs() < 1e-9, "لا أجر ساعات عادي فوقها");
        assert!((r.allowances - 7.0).abs() < 1e-9);
        assert!((r.net_pay - 294.0).abs() < 1e-6, "{}", r.net_pay);
    }

    /// اختيار «يوم بدل» لكل موظف/يوم من شاشة الحضور: يُحدّث العمود، ويعيد
    /// الاحتساب فيصير أجر ساعات يوم العطلة عادياً (بلا 62 في يومه)، ويظهر
    /// الخيار في التقرير اليومي؛ ورفض غير العطلة، ورفض «بدل» بعد إسناد يوم بدل.
    #[test]
    fn holiday_work_choice_switches_day_to_comp() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();

        // افتراضياً نقدي: 62 + (10 سا × 15 × 1.5) = 287
        {
            let conn = state.db.lock().unwrap();
            let r = &payroll_impl(&conn, "2026-10").unwrap()[0];
            assert_eq!(r.holiday_work_days, 1);
            assert!((r.holiday_work_pay - 287.0).abs() < 1e-6);
        }

        // يختار المسؤول «يوم بدل»
        let res = set_holiday_work_choice_impl(&state, 1, DAY, "comp").unwrap();
        assert_eq!(res["choice"], "comp");
        {
            let conn = state.db.lock().unwrap();
            let choice: Option<String> = conn
                .query_row(
                    "SELECT holiday_work_choice FROM attendance_days
                     WHERE employee_id=1 AND date=?1",
                    [DAY],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(choice.as_deref(), Some("comp"));
            // صار أجر الساعات عادياً (لا 62 ولا مضاعف): 600 د عمل − 120 إضافي
            // = 480 د أساسية (8 سا) + إضافي 2 سا × 15 × 1.5 = 165 ₪
            let r = &payroll_impl(&conn, "2026-10").unwrap()[0];
            assert_eq!(r.holiday_work_days, 0);
            assert!((r.holiday_work_pay).abs() < 1e-9);
            assert!((r.total_worked_hours - 8.0).abs() < 1e-6, "{}", r.total_worked_hours);
            assert!((r.base_pay - 165.0).abs() < 1e-6, "{}", r.base_pay);
            // الخيار يظهر في التقرير اليومي
            let rows = daily_report_impl(&conn, DAY).unwrap();
            assert_eq!(rows[0]["holiday_work_choice"], "comp");
        }

        // رفض غير العطلة
        let err = set_holiday_work_choice_impl(&state, 1, "2026-10-20", "comp").unwrap_err();
        assert!(err.contains("عطلة رسمية"), "{err}");
        // رفض خيار غير معروف
        let err = set_holiday_work_choice_impl(&state, 1, DAY, "ghost").unwrap_err();
        assert!(err.contains("خيار غير معروف"), "{err}");

        // بعد إسناد يوم بدل يُرفض تغيير الخيار إلى «بدل» مرة أخرى
        assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();
        let err = set_holiday_work_choice_impl(&state, 1, DAY, "comp").unwrap_err();
        assert!(err.contains("يوم بدل مسند"), "{err}");
    }

    /// يوم بدل مسند يُدفع له 62 في راتب شهره (حتى لو كان في الشهر التالي).
    #[test]
    fn comp_off_day_paid_in_its_own_month() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();

        let conn = state.db.lock().unwrap();
        let r = &payroll_impl(&conn, "2026-11").unwrap()[0];
        assert_eq!(r.comp_off_days, 1);
        assert!((r.comp_off_pay - 62.0).abs() < 1e-9, "أجر يوم العطلة 62");
        // دوام العطلة في نوفمبر غير موجود — الدفع هنا ليوم البدل فقط
        assert!((r.net_pay - 62.0).abs() < 1e-9, "{}", r.net_pay);
    }

    /// التقرير اليومي يكشف: هل التاريخ عطلة رسمية، ويوم البدل المسند،
    /// والعطلة التي يبدّلها يوم البدل نفسه.
    #[test]
    fn daily_report_exposes_holiday_and_comp_off() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        let rows = {
            let conn = state.db.lock().unwrap();
            daily_report_impl(&conn, DAY).unwrap()
        };
        assert_eq!(rows[0]["holiday"], true, "التاريخ عطلة رسمية");
        assert!(rows[0]["comp_off"].is_null(), "لم يُسند بعد");

        assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();
        let rows = {
            let conn = state.db.lock().unwrap();
            daily_report_impl(&conn, DAY).unwrap()
        };
        assert_eq!(rows[0]["comp_off"], "2026-11-03", "يظهر يوم البدل المسند");

        let rows = {
            let conn = state.db.lock().unwrap();
            daily_report_impl(&conn, "2026-11-03").unwrap()
        };
        assert_eq!(rows[0]["status"], "leave");
        assert_eq!(rows[0]["leave_type"], "comp");
        assert_eq!(rows[0]["comp_for"], DAY, "العطلة التي عوّضها");
    }

    /// إعادة احتساب الشهر بعد إسناد يوم بدل لا تُلغي يوم البدل (لا بصمات فيه).
    #[test]
    fn comp_off_survives_full_month_recompute() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO holidays(date,name,created_at) VALUES(?1,'عيد','x')", [DAY])
                .unwrap();
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        assign_comp_off_impl(&state, 1, DAY, "2026-11-03").unwrap();

        let from = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        recompute_range(&state, &from, &to).unwrap();

        let conn = state.db.lock().unwrap();
        let st: String = conn
            .query_row(
                "SELECT status FROM attendance_days WHERE employee_id=1 AND date='2026-11-03'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(st, "leave", "يوم البدل باقٍ بعد إعادة احتساب الشهر");
    }

    /// الأيام المستقبلية لا تُسجَّل غياباً: الصفوف المستقبلية القديمة تُنظَّف    /// عند إعادة الاحتساب ولا يُنشأ غياب بعد اليوم (كان يضخّم ملخّص الشهر).
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

    // ------------------------------------------------------------- الورديات

    /// الوردية تُضبط بحقولها وتعود في السرد مع عدد الموظفين.
    #[test]
    fn shift_crud_roundtrip() {
        let state = day_test_state(5.0);
        // الوردية الرئيسية موجودة ومبنية من الإعداد العام
        let shifts = {
            let conn = state.db.lock().unwrap();
            list_shifts_impl(&conn).unwrap()
        };
        assert_eq!(shifts.len(), 1);
        assert!(shifts[0].is_default && shifts[0].active);
        assert_eq!(
            shifts[0].employee_count, 0,
            "الموظفون غير المسنَدين يرثون الرئيسية عبر NULL (لا يُحتسبون إسناداً صريحاً)"
        );

        // أنشئ وردية جديدة
        let id = {
            let conn = state.db.lock().unwrap();
            upsert_shift_impl(
                &conn,
                Shift {
                    id: 0,
                    name: "وردية مسائية".into(),
                    work_start: "16:00".into(),
                    work_end: "23:30".into(),
                    daily_hours: 7.0,
                    break_min: 20,
                    is_default: false,
                    active: true,
                    employee_count: 0,
                },
            )
            .unwrap()
        };
        let shifts = {
            let conn = state.db.lock().unwrap();
            list_shifts_impl(&conn).unwrap()
        };
        assert_eq!(shifts.len(), 2);
        let night = shifts.iter().find(|s| s.id == id).unwrap();
        assert_eq!(night.work_start, "16:00");
        assert_eq!(night.daily_hours, 7.0);
        assert!(!night.is_default);

        // اجعلها الرئيسية
        {
            let conn = state.db.lock().unwrap();
            set_default_shift_impl(&conn, id).unwrap();
        }
        let shifts = {
            let conn = state.db.lock().unwrap();
            list_shifts_impl(&conn).unwrap()
        };
        assert_eq!(shifts.iter().filter(|s| s.is_default).count(), 1);
        assert!(shifts.iter().find(|s| s.id == id).unwrap().is_default);

        // احذف الأخرى → يبقى واحدة ولا تُحذف الأخيرة
        let other = shifts.iter().find(|s| s.id != id).unwrap().id;
        {
            let conn = state.db.lock().unwrap();
            delete_shift_impl(&conn, other).unwrap();
        }
        assert_eq!(
            state.db.lock().unwrap().query_row("SELECT COUNT(*) FROM shifts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let err = {
            let conn = state.db.lock().unwrap();
            delete_shift_impl(&conn, id).unwrap_err()
        };
        assert!(err.contains("آخر وردية"), "لا تُحذف آخر وردية: {err}");
    }

    /// رفض المدخلات غير الصالحة في الوردية.
    #[test]
    fn shift_validation_rejects_bad_input() {
        let state = day_test_state(5.0);
        let conn = state.db.lock().unwrap();
        let mk = |name: &str, ws: &str, we: &str, dh: f64, brk: i64| Shift {
            id: 0,
            name: name.into(),
            work_start: ws.into(),
            work_end: we.into(),
            daily_hours: dh,
            break_min: brk,
            is_default: false,
            active: true,
            employee_count: 0,
        };
        assert!(upsert_shift_impl(&conn, mk("", "09:00", "17:00", 8.0, 30)).is_err());
        assert!(upsert_shift_impl(&conn, mk("س", "25:00", "17:00", 8.0, 30)).is_err());
        assert!(upsert_shift_impl(&conn, mk("س", "09:00", "09:00", 8.0, 30)).is_err());
        assert!(upsert_shift_impl(&conn, mk("س", "09:00", "17:00", 0.0, 30)).is_err());
        assert!(upsert_shift_impl(&conn, mk("س", "09:00", "17:00", 8.0, -1)).is_err());
    }

    /// الإسناد الجماعي: يسنِد عدة موظفين لوردية، ويُخرج من أُزيل من القائمة،
    /// ويعيد عدد من تغيّرت ورديتهم فعلاً (لا يُحتسب من لم تتغيّر).
    #[test]
    fn bulk_assign_employees_to_shift() {
        let state = day_test_state(5.0);
        // موظفان إضافيان
        {
            let conn = state.db.lock().unwrap();
            for id in [2i64, 3] {
                conn.execute(
                    "INSERT INTO employees(id,pin,name,name_en,daily_hours,break_min,hourly_rate,
                     meal_allow,transport_allow,active,weekend_paid,annual_leave_balance,created_at,updated_at)
                     VALUES(?1,?2,'موظف','Emp',8.5,30,15.0,7.0,0.0,1,1,0.0,'x','x')",
                    rusqlite::params![id, id.to_string()],
                )
                .unwrap();
            }
        }
        let shift_id = {
            let conn = state.db.lock().unwrap();
            upsert_shift_impl(
                &conn,
                Shift {
                    id: 0,
                    name: "مسائية".into(),
                    work_start: "16:00".into(),
                    work_end: "23:30".into(),
                    daily_hours: 7.0,
                    break_min: 20,
                    is_default: false,
                    active: true,
                    employee_count: 0,
                },
            )
            .unwrap()
        };

        // إسناد الموظفين 1 و2
        let changed = {
            let conn = state.db.lock().unwrap();
            assign_employees_to_shift_impl(&conn, &[1, 2], Some(shift_id)).unwrap()
        };
        assert_eq!(changed, 2, "تغيّرت وردية موظفين");
        {
            let conn = state.db.lock().unwrap();
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM employees WHERE shift_id=?1",
                    [shift_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 2);
            // العدّاد في list_shifts يعكس الإسناد الصريح
            let shifts = list_shifts_impl(&conn).unwrap();
            assert_eq!(shifts.iter().find(|s| s.id == shift_id).unwrap().employee_count, 2);
        }

        // إعادة إسناد نفس المجموعة = لا تغيير
        let changed = {
            let conn = state.db.lock().unwrap();
            assign_employees_to_shift_impl(&conn, &[1, 2], Some(shift_id)).unwrap()
        };
        assert_eq!(changed, 0, "لا تغيير عند إعادة الإسناد نفسه");

        // إخراج الموظف 2 (إلى الرئيسية = NULL) وإضافة 3
        {
            let conn = state.db.lock().unwrap();
            let c1 = assign_employees_to_shift_impl(&conn, &[2], None).unwrap();
            let c2 = assign_employees_to_shift_impl(&conn, &[3], Some(shift_id)).unwrap();
            assert_eq!((c1, c2), (1, 1));
        }
        {
            let conn = state.db.lock().unwrap();
            let sid2: Option<i64> = conn
                .query_row("SELECT shift_id FROM employees WHERE id=2", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sid2, None, "الموظف المُزال يعود للرئيسية (NULL)");
            let sid3: Option<i64> = conn
                .query_row("SELECT shift_id FROM employees WHERE id=3", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sid3, Some(shift_id));
        }

        // وردية غير موجودة تُرفض
        let err = {
            let conn = state.db.lock().unwrap();
            assign_employees_to_shift_impl(&conn, &[1], Some(99999)).unwrap_err()
        };
        assert!(err.contains("غير موجودة"), "{err}");
    }

    /// موظف على وردية مختلفة يُحتسب بتوقيتها: وردية 16:00→23:30 تجعل بصمة
    /// 16:00 حضوراً (بلا تأخير) بينما الإعداد العام 06:00 كان سيجعلها متأخرة.
    #[test]
    fn employee_shift_changes_attendance_calc() {
        let state = day_test_state(5.0);
        // وردية مسائية بلا استراحة (لتبسيط الساعات) وساعات أساسية 7
        let night_id = {
            let conn = state.db.lock().unwrap();
            upsert_shift_impl(
                &conn,
                Shift {
                    id: 0,
                    name: "مسائية".into(),
                    work_start: "16:00".into(),
                    work_end: "23:00".into(),
                    daily_hours: 7.0,
                    break_min: 0,
                    is_default: false,
                    active: true,
                    employee_count: 0,
                },
            )
            .unwrap()
        };
        {
            let conn = state.db.lock().unwrap();
            conn.execute("UPDATE employees SET shift_id=?1 WHERE id=1", [night_id]).unwrap();
            conn.execute("DELETE FROM punches", []).unwrap();
            for (dt, pt) in [("2026-10-19T16:00:00", 0i64), ("2026-10-19T23:00:00", 1i64)] {
                conn.execute(
                    "INSERT INTO punches(employee_id,pin,device_time,punch_type,synced_at)
                     VALUES(1,'1',?1,?2,'x')",
                    rusqlite::params![dt, pt],
                )
                .unwrap();
            }
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        let conn = state.db.lock().unwrap();
        let (status, worked, late): (String, i64, i64) = conn
            .query_row(
                "SELECT status, worked_min, late_min FROM attendance_days
                 WHERE employee_id=1 AND date=?1",
                [DAY],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "present");
        assert_eq!(worked, 420, "7 ساعات بلا استراحة");
        assert_eq!(late, 0, "دخل في بداية ورديته (16:00) — لا تأخير");
    }

    /// مهلة التأخير تُطبَّق: حضور 09:15 مع مهلة 20 دقيقة → تأخير 0.
    #[test]
    fn grace_late_min_suppresses_small_late() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            // وردية تبدأ 09:00 واستراحة صفر لتبسيط الحساب
            conn.execute(
                "UPDATE shifts SET work_start='09:00', work_end='17:00', break_min=0 WHERE is_default=1",
                [],
            )
            .unwrap();
            db::set_setting(&conn, "grace_late_min", "20").unwrap();
            conn.execute("DELETE FROM punches", []).unwrap();
            for (dt, pt) in [("2026-10-19T09:15:00", 0i64), ("2026-10-19T17:00:00", 1i64)] {
                conn.execute(
                    "INSERT INTO punches(employee_id,pin,device_time,punch_type,synced_at)
                     VALUES(1,'1',?1,?2,'x')",
                    rusqlite::params![dt, pt],
                )
                .unwrap();
            }
        }
        let d = NaiveDate::parse_from_str(DAY, "%Y-%m-%d").unwrap();
        recompute_range(&state, &d, &d).unwrap();
        let late: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT late_min FROM attendance_days WHERE employee_id=1 AND date=?1",
                [DAY],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(late, 0, "15 دقيقة ضمن مهلة 20 د = تأخير صفر");
    }

    /// كشف التنبيهات الجديدة: غياب اليوم، الانصراف المبكر، ولا بصمات.
    #[test]
    fn new_alerts_detected() {
        let state = day_test_state(5.0);
        {
            let conn = state.db.lock().unwrap();
            // يوم بطريقة عادية ثم نضع صفوف الحالات يدوياً (تنبيهات تقرأ المخزّن)
            conn.execute(
                "INSERT OR REPLACE INTO attendance_days(employee_id,date,first_in,last_out,
                 worked_min,late_min,early_leave_min,overtime_min,absent,status,manual)
                 VALUES(1,'2026-10-19','2026-10-19T09:00:00','2026-10-19T15:00:00',
                 300,0,120,0,0,'present',0)",
                [],
            )
            .unwrap();
            // الموظف رقم 2 غائب اليوم
            conn.execute(
                "INSERT INTO employees(id,pin,name,name_en,daily_hours,break_min,hourly_rate,
                 meal_allow,transport_allow,active,created_at,updated_at)
                 VALUES(2,'2','ثانٍ','Second',8.5,30,10.0,7.0,0.0,1,'x','x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO attendance_days(employee_id,date,worked_min,late_min,
                 early_leave_min,overtime_min,absent,status,manual)
                 VALUES(2,'2026-10-19',0,0,0,0,1,'absent',0)",
                [],
            )
            .unwrap();
            // لا بصمات اليوم إطلاقاً (نمسحها)
            conn.execute("DELETE FROM punches", []).unwrap();
        }
        let fresh = detect_missing_events(&state.db.lock().unwrap(), "2026-10-19");
        assert!(
            fresh.iter().any(|s| s.contains("غاب") || s.contains("غياب")),
            "تنبيه غياب مفقود: {fresh:?}"
        );
        assert!(
            fresh.iter().any(|s| s.contains("انصراف مبكر")),
            "تنبيه انصراف مبكر مفقود: {fresh:?}"
        );
        assert!(
            fresh.iter().any(|s| s.contains("لا بصمات")),
            "تنبيه لا بصمات مفقود: {fresh:?}"
        );
    }

    /// دفع الشهر: تعليم ثم عرض ثم إلغاء — والتعليم ثانية لنفس (موظف، شهر)
    /// يستبدل السجل لا يكرّره (ضمانة عدم تضاعف صفوف الدفع).
    #[test]
    fn salary_payment_mark_list_unmark() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let at = mark_salary_paid_impl(&conn, 9, "2026-10", 5000.0, Some("نقدي".into()), None)
            .unwrap();
        assert!(!at.is_empty(), "تاريخ الدفع يجب ألا يكون فارغاً");
        let all = salary_payments_impl(&conn, Some("2026-10".into())).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["net"], 5000.0);
        assert_eq!(all[0]["method"], "نقدي");
        // تعليم ثانٍ للفترة نفسها = تحديث لا تكرار
        mark_salary_paid_impl(&conn, 9, "2026-10", 5100.0, Some("تحويل".into()), None).unwrap();
        let all = salary_payments_impl(&conn, Some("2026-10".into())).unwrap();
        assert_eq!(all.len(), 1, "لا يتكرر سطر الدفع لنفس الشهر");
        assert_eq!(all[0]["net"], 5100.0);
        assert_eq!(all[0]["method"], "تحويل");
        // فلترة بآخر الشهر: شهر آخر بلا دفعات
        assert!(
            salary_payments_impl(&conn, Some("2026-11".into()))
                .unwrap()
                .is_empty()
        );
        // إلغاء الدفع يحذف السجل
        let del = unmark_salary_paid_impl(&conn, 9, "2026-10").unwrap();
        assert_eq!(del, 1);
        assert!(salary_payments_impl(&conn, Some("2026-10".into()))
            .unwrap()
            .is_empty());
    }

    /// لقطة القسيمة: تُحفظ إعدادات الطباعة ونسبها والأرقام لحظة الطباعة،
    /// وتُستبدل بطباعة/دفع ثانٍ بإعدادات جديدة (لقطة واحدة لكل موظف وشهر).
    #[test]
    fn payslip_snapshot_saves_and_replaces() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init_db(&conn).unwrap();
        let snap = PayslipSnapshot {
            employee_id: 9,
            period: "2026-10".into(),
            rate: 8.5,
            ot_mult: 1.5,
            p_work: 45.0,
            p_risk: 45.0,
            p_bonus: 10.0,
            basic: 1700.0,
            overtime: 300.0,
            weekend_pay: 62.0,
            allowances: 120.0,
            holiday_work: 0.0,
            comp_off: 0.0,
            share_work: 1000.0,
            share_risk: 1000.0,
            share_bonus: 222.0,
            net: 4404.0,
            enabled: true,
            sheet: Some(r#"{"period":"2026-10","net":4404}"#.into()),
        };
        let at = save_payslip_snapshot_impl(&conn, &snap).unwrap();
        assert!(!at.is_empty());
        let list = payslip_snapshots_impl(&conn, Some("2026-10".into())).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["rate"], 8.5);
        assert_eq!(list[0]["p_work"], 45.0);
        assert_eq!(list[0]["share_risk"], 1000.0);
        assert_eq!(list[0]["net"], 4404.0);
        // النسخة المجمّدة من القسيمة تُحفظ وتُرجع كما هي (للطباعة المطابقة لاحقاً)
        assert_eq!(list[0]["enabled"], true);
        assert_eq!(
            list[0]["sheet"].as_str(),
            Some(r#"{"period":"2026-10","net":4404}"#),
            "النسخة المجمّدة يجب أن تُحفظ حرفياً"
        );
        // طباعة/دفع ثانٍ بإعدادات جديدة يستبدل اللقطة لا يضيف ثانية
        let snap2 = PayslipSnapshot {
            rate: 9.5,
            p_work: 50.0,
            p_risk: 40.0,
            p_bonus: 10.0,
            enabled: false, // تقسيم ملغٍ وقتها
            sheet: None,
            ..snap
        };
        save_payslip_snapshot_impl(&conn, &snap2).unwrap();
        let list = payslip_snapshots_impl(&conn, Some("2026-10".into())).unwrap();
        assert_eq!(list.len(), 1, "لقطة واحدة لكل (موظف، شهر)");
        assert_eq!(list[0]["rate"], 9.5);
        assert_eq!(list[0]["p_work"], 50.0);
        assert_eq!(list[0]["p_risk"], 40.0);
        assert_eq!(list[0]["enabled"], false, "علم «ملغٍ» يُستبدل مع اللقطة");
        assert!(
            list[0]["sheet"].is_null(),
            "النسخة السابقة تُمحَّد عند استبدال اللقطة"
        );
        // شهر آخر مستقل
        let snap3 = PayslipSnapshot {
            period: "2026-11".into(),
            ..snap2
        };
        save_payslip_snapshot_impl(&conn, &snap3).unwrap();
        assert_eq!(
            payslip_snapshots_impl(&conn, None).unwrap().len(),
            2,
            "بلا فلتر: كل اللقطات"
        );
    }
}
