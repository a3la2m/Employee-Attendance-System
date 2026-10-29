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
        -- فهرس عام بالتاريخ (لـ present_now وتقارير اليوم — بدون عملية substr غير الفهرسة)
        CREATE INDEX IF NOT EXISTS idx_punches_time ON punches(device_time);

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
            no_break      INTEGER NOT NULL DEFAULT 0, -- إلغاء خصم الاستراحة لهذا اليوم
            no_meal       INTEGER NOT NULL DEFAULT 0, -- إزالة بدل الفطور لهذا اليوم
            UNIQUE(employee_id, date)
        );
        CREATE INDEX IF NOT EXISTS idx_attendance_days_date ON attendance_days(date);

        CREATE TABLE IF NOT EXISTS leaves (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER NOT NULL,
            from_date   TEXT NOT NULL,
            to_date     TEXT NOT NULL,
            leave_type  TEXT NOT NULL DEFAULT 'annual',
            note        TEXT DEFAULT '',
            created_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS holidays (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            date        TEXT NOT NULL UNIQUE,   -- YYYY-MM-DD
            name        TEXT NOT NULL DEFAULT '',
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

        CREATE TABLE IF NOT EXISTS notifications (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            kind        TEXT NOT NULL,            -- late | missing_out | orphan_out
            employee_id INTEGER,
            title       TEXT NOT NULL,
            body        TEXT NOT NULL,
            day         TEXT NOT NULL,            -- YYYY-MM-DD التاريخ المرتبط بالحدث
            created_at  TEXT NOT NULL,
            seen        INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_notifications_seen ON notifications(seen);

        -- الورديات: ساعات الدوام (بداية/نهاية + الساعات الأساسية + الاستراحة)
        -- تُعرّف مرة وتُسنَد للموظفين. الوردية الرئيسية (is_default=1) هي التي
        -- يرثها أي موظف غير مسنَد لوردية. الوردية غير المفعّلة (active=0) مهجورة
        -- (تظهر في السجل للتوثيق لكن لا تُسنَد لموظفين جدد).
        CREATE TABLE IF NOT EXISTS shifts (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            work_start  TEXT NOT NULL DEFAULT '09:00',   -- HH:MM
            work_end    TEXT NOT NULL DEFAULT '17:30',   -- HH:MM
            daily_hours REAL NOT NULL DEFAULT 8.5,       -- ساعات الدوام الأساسية
            break_min   INTEGER NOT NULL DEFAULT 30,     -- الاستراحة القياسية (دقائق)
            is_default  INTEGER NOT NULL DEFAULT 0,      -- الوردية الرئيسية (واحدة فقط)
            active      INTEGER NOT NULL DEFAULT 1,      -- 0 = مهجورة
            created_at  TEXT NOT NULL DEFAULT '',
            updated_at  TEXT NOT NULL DEFAULT ''
        );

        -- دفع الرواتب: حالة «مدفوع» الوحيدة في النظام (الأرشفة تجميد أرقام فقط).
        -- صف واحد لكل (موظف، شهر) — إعادة التعليم تستبدل السجل ولا تكرّره.
        CREATE TABLE IF NOT EXISTS salary_payments (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER NOT NULL,
            period      TEXT NOT NULL,                 -- YYYY-MM
            net         REAL NOT NULL DEFAULT 0,       -- الصافي المدفوع وقت التسجيل
            method      TEXT,                          -- نقدي | تحويل مصرفي | شيك | أخرى
            note        TEXT,
            paid_at     TEXT NOT NULL,                 -- ISO — تاريخ التعليم كمدفوع
            UNIQUE(employee_id, period)
        );

        -- لقطة إعدادات طباعة القسيمة ونسبها لحظة الطباعة أو لحظة الدفع —
        -- تبقى القسيمة قابلة لإثبات ما طُبع فعلاً حتى لو تغيّرت الإعدادات لاحقاً.
        CREATE TABLE IF NOT EXISTS payslip_snapshots (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER NOT NULL,
            period      TEXT NOT NULL,                 -- YYYY-MM
            rate        REAL NOT NULL DEFAULT 0,       -- سعر الساعة الأساسي لحظتها
            ot_mult     REAL NOT NULL DEFAULT 1.5,     -- معامل ساعة الإضافي
            p_work      REAL NOT NULL DEFAULT 0,       -- نسب البدلات الثلاثة (%)
            p_risk      REAL NOT NULL DEFAULT 0,
            p_bonus     REAL NOT NULL DEFAULT 0,
            basic       REAL NOT NULL DEFAULT 0,       -- الأرقام المطبوعة لحظتها
            overtime    REAL NOT NULL DEFAULT 0,
            weekend_pay REAL NOT NULL DEFAULT 0,
            allowances  REAL NOT NULL DEFAULT 0,
            holiday_work REAL NOT NULL DEFAULT 0,
            comp_off    REAL NOT NULL DEFAULT 0,
            share_work  REAL NOT NULL DEFAULT 0,       -- حصص البدلات الثلاثة المقطوعة
            share_risk  REAL NOT NULL DEFAULT 0,
            share_bonus REAL NOT NULL DEFAULT 0,
            net         REAL NOT NULL DEFAULT 0,       -- الصافي (لا يتغير بالتقسيم)
            printed_at  TEXT NOT NULL,                 -- ISO
            enabled     INTEGER NOT NULL DEFAULT 1,    -- هل كان التقسيم مفعلاً وقتها؟
            sheet       TEXT,                          -- نسخة JSON مجمّدة من القسيمة وقتها
            UNIQUE(employee_id, period)
        );

        -- سلف الموظفين: تُحفظ السلفة الأصلية ونوع خصمها (دفعة واحدة على راتب قادم أو تقسيط على عدة رواتب).
        CREATE TABLE IF NOT EXISTS employee_advances (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id  INTEGER NOT NULL,
            amount       REAL NOT NULL,                -- إجمالي مبلغ السلفة
            mode         TEXT NOT NULL DEFAULT 'single', -- 'single' | 'installments'
            installments INTEGER NOT NULL DEFAULT 1,   -- عدد الأقساط
            start_period TEXT NOT NULL,                -- YYYY-MM أول شهر يبدأ فيه الخصم
            note         TEXT NOT NULL DEFAULT '',
            created_at   TEXT NOT NULL
        );

        -- أقساط السلف لكل شهر: كل قسط يرتبط بشهر راتب معيّن (period) ويمكن تعليمه كمخصوم يدوياً أو مع دفع الراتب.
        CREATE TABLE IF NOT EXISTS advance_installments (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            advance_id  INTEGER NOT NULL,
            employee_id INTEGER NOT NULL,
            period      TEXT NOT NULL,                 -- YYYY-MM
            amount      REAL NOT NULL,
            deducted    INTEGER NOT NULL DEFAULT 0,    -- 1 = تم الخصم، 0 = لم تخصم بعد (فعالة)
            deducted_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_adv_inst_emp_period ON advance_installments(employee_id, period);
        CREATE INDEX IF NOT EXISTS idx_adv_inst_adv ON advance_installments(advance_id);

        -- بوت تلجرام: مشرفو الشركة الذين يستقبلون تنبيهات الموظفين (بعد ربط
        -- محادثتهم بمشاركة رقم الهاتف). chat_id يبقى NULL حتى يتم الربط.
        CREATE TABLE IF NOT EXISTS telegram_supervisors (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            name        TEXT NOT NULL,
            phone       TEXT NOT NULL,
            chat_id     TEXT,
            linked_at   TEXT,
            active      INTEGER NOT NULL DEFAULT 1,
            created_at  TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_tg_sup_chat
            ON telegram_supervisors(chat_id) WHERE chat_id IS NOT NULL;

        -- أسباب الغياب التي يرسلها الموظف عبر البوت (سبب واحد لكل يوم).
        CREATE TABLE IF NOT EXISTS absence_notes (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            employee_id INTEGER NOT NULL,
            date        TEXT NOT NULL,
            reason      TEXT NOT NULL,
            source      TEXT NOT NULL DEFAULT 'telegram',
            created_at  TEXT NOT NULL,
            UNIQUE(employee_id, date)
        );
        CREATE INDEX IF NOT EXISTS idx_absence_date ON absence_notes(date);

        "#,
    )?;

    // ترحيل الأعمدة الجديدة (آمن — يتجاهل الخطأ إن كان العمود موجوداً)
    migrate_column(conn, "employees", "overtime_enabled", "INTEGER NOT NULL DEFAULT 1")?;
    migrate_column(conn, "employees", "meal_enabled", "INTEGER NOT NULL DEFAULT 1")?;
    migrate_column(conn, "employees", "transport_enabled", "INTEGER NOT NULL DEFAULT 1")?;
    // الاسم اللاتيني للشاشة التي لا تدعم العربية (يُرسل إلى الجهاز)
    migrate_column(conn, "employees", "name_en", "TEXT NOT NULL DEFAULT ''")?;
    // تعديل يدوي ليوم حضور (1 = مُثبَّت من المسؤول، تتجاهله إعادة الاحتساب)
    migrate_column(conn, "attendance_days", "manual", "INTEGER NOT NULL DEFAULT 0")?;
    // دفع العطلة الأسبوعية (1 = مدفوعة — تُلغى لموظف بعينه فتصبح بدون أجر)
    migrate_column(conn, "employees", "weekend_paid", "INTEGER NOT NULL DEFAULT 1")?;
    // أجر العطلة الأسبوعية في أرشيف الرواتب
    migrate_column(conn, "payroll_runs", "weekend_pay", "REAL NOT NULL DEFAULT 0")?;
    // هل الإجازة مدفوعة (1 = تُحتسب ضمن بدل العطلة لكل يوم دوام)
    migrate_column(conn, "leaves", "paid", "INTEGER NOT NULL DEFAULT 1")?;
    // رصيد الإجازات السنوية للموظف (بالأيام)
    migrate_column(conn, "employees", "annual_leave_balance", "REAL NOT NULL DEFAULT 0")?;
    // الاستحقاق السنوي للإجازات (أيام في السنة — الافتراضي 14، يُخصم شهرياً ÷12)
    migrate_column(conn, "employees", "annual_leave_days", "REAL NOT NULL DEFAULT 14")?;
    // آخر شهر (YYYY-MM) أُضيفت أرصدته تلقائياً — لتستأنف الإضافة من حيث توقفت
    migrate_column(conn, "employees", "leave_accrual_month", "TEXT")?;
    // حالة إجازة اليوم: 1 = مدفوعة، 0 = غير مدفوعة، NULL = ليس يوم إجازة
    migrate_column(conn, "attendance_days", "leave_paid", "INTEGER")?;
    // أزرار اليوم في شاشة الحضور: إلغاء خصم الاستراحة / إزالة بدل الفطور
    migrate_column(conn, "attendance_days", "no_break", "INTEGER NOT NULL DEFAULT 0")?;
    migrate_column(conn, "attendance_days", "no_meal", "INTEGER NOT NULL DEFAULT 0")?;
    // اليوم التعويضي: التاريخ الذي يُعوّض عطلة رسمية دُوّم فيها (زر «يوم بدل»)
    migrate_column(conn, "leaves", "comp_for_date", "TEXT")?;
    // إجازة صنعها المسؤول صراحة ليوم له بصمات (لا تُلغى تلقائياً بالدوام)
    migrate_column(conn, "leaves", "keep_punches", "INTEGER NOT NULL DEFAULT 0")?;
    // خيار تعويض الدوام في عطلة رسمية لذلك اليوم/الموظف: 'money' (نقدي،
    // الافتراضي) أو 'comp' (يوم بدل). يُضبط من شاشة الحضور لكل يوم على حدة.
    migrate_column(conn, "attendance_days", "holiday_work_choice", "TEXT")?;
    // الوردية المسنَدة للموظف (NULL = يرث الوردية الرئيسية)
    migrate_column(conn, "employees", "shift_id", "INTEGER")?;
    // صلاحية الموظف على جهاز البصمة: 0 = عادي، 6 = مدير، 14 = أدمن كامل (سوبر أدمن)
    migrate_column(conn, "employees", "device_privilege", "INTEGER NOT NULL DEFAULT 0")?;
    // لقطة القسيمة: هل كان التقسيم مفعلاً وقت الحفظ + نسخة JSON كاملة مجمّدة
    // (أيام الجدول والبنود والإعدادات) لإعادة إنتاج الطباعة حرفياً لاحقاً
    migrate_column(conn, "payslip_snapshots", "enabled", "INTEGER NOT NULL DEFAULT 1")?;
    migrate_column(conn, "payslip_snapshots", "sheet", "TEXT")?;
    migrate_column(conn, "payslip_snapshots", "advance_deduction", "REAL NOT NULL DEFAULT 0")?;
    migrate_column(conn, "payroll_runs", "advance_deduction", "REAL NOT NULL DEFAULT 0")?;
    // هل يُسمح للموظف باستخدام بوت تلجرام؟ (1 افتراضياً — يُمنع يدوياً)
    migrate_column(conn, "employees", "bot_allowed", "INTEGER NOT NULL DEFAULT 1")?;
    // ربط الموظف ببوت تلجرام (يُملأ عندما يشارك رقم هاتفه مع البوت)
    migrate_column(conn, "employees", "telegram_chat_id", "TEXT")?;
    migrate_column(conn, "employees", "telegram_linked_at", "TEXT")?;
    // محادثة واحدة لكل موظف (NULL متعدد مسموح)
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_emp_tg_chat
         ON employees(telegram_chat_id) WHERE telegram_chat_id IS NOT NULL;",
    )?;
    Ok(())
}

/// يضيف عموداً إلى جدول إن لم يكن موجوداً (يضغط الأخطاء غير الحرجّة).
fn migrate_column(conn: &Connection, table: &str, col: &str, def: &str) -> Result<()> {
    // تحقّق أولاً من وجود العمود
    let exists: bool = {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        names.iter().any(|n| n == col)
    };
    if !exists {
        conn.execute(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, def), [])?;
    }
    Ok(())
}

/// يزرع الإعدادات الافتراضية (مرة واحدة).
pub fn seed_defaults(conn: &Connection) -> Result<()> {
    let defaults = [
        ("daily_hours", "8.5"),
        ("break_min", "30"),
        ("hourly_rate", "10.0"),
        ("currency", "ILS"),
        ("meal_allow", "7.0"),
        ("transport_allow", "5.0"),
        ("overtime_after_hours", "8.5"),
        ("overtime_rate_mult", "1.5"),
        ("grace_late_min", "0"),
        ("weekend_allow", "62.0"),
        // طريقة احتساب بدل العطلة: weekly = مبلغ الأسبوع ÷ أيام السبت–الخميس
        ("weekend_pay_mode", "weekly"),
        ("timezone", "Asia/Jerusalem"),
        ("weekend_days", "4"),
        ("work_start", "09:00"),
        ("work_end", "17:30"),
        ("company_name", "اسم الشركة"),
        ("company_logo", ""),
        ("device_ip", "192.168.1.205"),
        ("device_port", "4370"),
        ("device_comm_key", "0"),
        ("auto_sync_min", "5"),
        ("auto_fix_clock", "1"),
        ("last_sync_at", ""),
        // إشعار تأخير الموظفين (خليا يعمل مرة واحدة يومياً عند الوقت المحدد)
        ("late_notify_enabled", "1"),
        ("late_notify_time", "09:00"),
        ("late_notify_sent", ""),
        // خصم الاستراحة القياسية (نصف ساعة) من أيام الدوام — زر «لا تخصم الاستراحة»
        ("deduct_break", "1"),
        // الإضافة التلقائية الشهرية لأرصدة الإجازات السنوية (الاستحقاق السنوي ÷ 12)
        ("leave_accrual_enabled", "1"),
        // آخر فحص لأرصدة الإجازات (YYYY-MM-DD — مرة واحدة يومياً كالتنبيهات)
        ("leave_accrual_last_run", ""),
    ];
    for (k, v) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO settings(key, value) VALUES (?1, ?2)",
            [k, v],
        )?;
    }

    // وردية رئيسية افتراضية واحدة — تُبنى من الإعداد العام (work_start/work_end/
    // daily_hours/break_min). فقط إن لم توجد أي وردية إطلاقاً.
    let shift_count: i64 = conn.query_row("SELECT COUNT(*) FROM shifts", [], |r| r.get(0))?;
    if shift_count == 0 {
        let get = |k: &str, d: &str| -> String {
            conn.query_row("SELECT value FROM settings WHERE key=?1", [k], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| d.to_string())
        };
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO shifts(name,work_start,work_end,daily_hours,break_min,is_default,active,created_at,updated_at)
             VALUES('الوردية الرئيسية',?1,?2,?3,?4,1,1,?5,?5)",
            rusqlite::params![
                get("work_start", "09:00"),
                get("work_end", "17:30"),
                get("daily_hours", "8.5").parse::<f64>().unwrap_or(8.5),
                get("break_min", "30").parse::<i64>().unwrap_or(30),
                now,
            ],
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
