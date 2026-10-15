//! مخطط قاعدة البيانات (SQLite) — كل الجداول والترحيلات.
use rusqlite::{Connection, Result};
use std::path::Path;

/// ينشئ كل الجداول إن لم تكن موجودة (idempotent).
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS employees (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            pin          TEXT UNIQUE,              -- رقم البصمة على الجهاز
            name         TEXT NOT NULL,
            job_title    TEXT DEFAULT '',
            phone        TEXT DEFAULT '',
            daily_hours  REAL NOT NULL DEFAULT 8.5,
            break_min    INTEGER NOT NULL DEFAULT 30,
            hourly_rate  REAL NOT NULL DEFAULT 10.0,
            meal_allow   REAL NOT NULL DEFAULT 7.0,
            transport_allow REAL NOT NULL DEFAULT 5.0,
            active       INTEGER NOT NULL DEFAULT 1,
            created_at   TEXT NOT NULL,
            updated_at   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS punches (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER,
            pin         TEXT NOT NULL,
            device_time TEXT NOT NULL,        -- ISO8601 كما من الجهاز
            punch_type  INTEGER DEFAULT 0,    -- 0=غير محدد 1=حضور 2=انصراف
            work_code   INTEGER DEFAULT 0,
            raw_status  INTEGER DEFAULT 0,
            synced_at   TEXT NOT NULL,
            UNIQUE(pin, device_time)          -- منع التكرار عند السحب
        );
        CREATE INDEX IF NOT EXISTS idx_punches_emp_time ON punches(employee_id, device_time);
        CREATE INDEX IF NOT EXISTS idx_punches_pin_time ON punches(pin, device_time);

        CREATE TABLE IF NOT EXISTS attendance_days (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id   INTEGER NOT NULL,
            date          TEXT NOT NULL,          -- YYYY-MM-DD
            first_in      TEXT,
            last_out      TEXT,
            worked_min    INTEGER NOT NULL DEFAULT 0,
            late_min      INTEGER NOT NULL DEFAULT 0,
            early_leave_min INTEGER NOT NULL DEFAULT 0,
            overtime_min  INTEGER NOT NULL DEFAULT 0,
            absent        INTEGER NOT NULL DEFAULT 0,
            leave_type    TEXT,                   -- annual/sick/...
            status        TEXT NOT NULL DEFAULT 'present', -- present/absent/late/leave
            UNIQUE(employee_id, date)
        );

        CREATE TABLE IF NOT EXISTS leaves (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER NOT NULL,
            from_date   TEXT NOT NULL,
            to_date     TEXT NOT NULL,
            leave_type  TEXT NOT NULL DEFAULT 'annual',
            note        TEXT DEFAULT '',
            created_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS payroll_runs (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            period      TEXT NOT NULL,            -- YYYY-MM
            employee_id INTEGER NOT NULL,
            days_present INTEGER NOT NULL DEFAULT 0,
            total_worked_hours REAL NOT NULL DEFAULT 0,
            overtime_hours REAL NOT NULL DEFAULT 0,
            base_pay    REAL NOT NULL DEFAULT 0,
            allowances  REAL NOT NULL DEFAULT 0,
            late_deduction REAL NOT NULL DEFAULT 0,
            absence_deduction REAL NOT NULL DEFAULT 0,
            net_pay     REAL NOT NULL DEFAULT 0,
            generated_at TEXT NOT NULL,
            UNIQUE(period, employee_id)
        );
        "#,
    )?;
    Ok(())
}

/// يزرع الإعدادات الافتراضية (مرة واحدة).
pub fn seed_defaults(conn: &Connection) -> Result<()> {
    let defaults = [
        ("daily_hours", "8.5"),
        ("break_min", "30"),
        ("hourly_rate", "10.0"),
        ("meal_allow", "7.0"),
        ("transport_allow", "5.0"),
        ("overtime_after_hours", "8.5"),
        ("overtime_rate_mult", "1.5"),
        ("grace_late_min", "0"),
        ("timezone", "Asia/Jerusalem"),
        ("weekend_days", "4,5"),
        ("work_start", "09:00"),
        ("work_end", "17:30"),
        ("company_name", "اسم الشركة"),
        ("company_logo", ""),
        ("device_ip", "192.168.1.205"),
        ("device_port", "4370"),
        ("device_comm_key", "0"),
        ("auto_sync_min", "5"),
        ("last_sync_at", ""),
    ];
    for (k, v) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO settings(key, value) VALUES (?1, ?2)",
            [k, v],
        )?;
    }
    Ok(())
}

/// يقرأ كل الإعدادات في خريطة.
pub fn all_settings(conn: &Connection) -> Result<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let map = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|x| x.ok())
        .collect();
    Ok(map)
}

/// يحفظ/يحدّث إعداداً واحداً.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// ينشئ قاعدة بيانات في المسار المحدد ويهيّئها.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init_db(&conn)?;
    seed_defaults(&conn)?;
    Ok(conn)
}
