//! تكامل تلجرام: بوت واحد للشركة.
//!
//! - يربط الموظف/المشرف بمحادثته عندما يشارك رقم هاتفه مع البوت
//!   (زر «مشاركة رقم هاتفي» — لا نطلب الرقم كتابةً حتى لا يُنتحل).
//! - يستقبل أوامر الموظف: كشف الدوام (الشهر الحالي/شهر محدد)، حالة اليوم،
//!   صافي الراتب، وتسجيل سبب الغياب.
//! - يرسل للمشرفين التنبيهات (تأخير، خروج مفقود، أسباب الغياب) وملخصاً
//!   يومياً بمن ختم ومن لم يختم.
//!
//! كل الإعدادات في جدول settings (tg_*) ولا يلزمها خدمة خارجية.

use crate::commands::{self, AppState};
use crate::db;
use crate::models::{
    AbsenceNote, AttendanceDay, TelegramLink, TelegramSupervisor,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tauri::Manager;

const API: &str = "https://api.telegram.org/bot";

const MONTHS: [&str; 12] = [
    "يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو", "يوليو", "أغسطس", "سبتمبر",
    "أكتوبر", "نوفمبر", "ديسمبر",
];
const WEEKDAYS: [&str; 7] = [
    "الإثنين", "الثلاثاء", "الأربعاء", "الخميس", "الجمعة", "السبت", "الأحد",
];

// ------------------------------------------------------------------ أدوات عامة

fn settings(conn: &Connection) -> HashMap<String, String> {
    db::all_settings(conn).unwrap_or_default()
}

fn enabled(s: &HashMap<String, String>, key: &str, def: bool) -> bool {
    match s.get(key) {
        Some(v) if !v.trim().is_empty() => v != "0",
        _ => def,
    }
}

fn text_of(s: &HashMap<String, String>, key: &str, def: &str) -> String {
    s.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| def.to_string())
}

fn tz_of(s: &HashMap<String, String>) -> chrono_tz::Tz {
    s.get("timezone")
        .and_then(|t| t.parse().ok())
        .unwrap_or(chrono_tz::Asia::Jerusalem)
}

fn today_in(s: &HashMap<String, String>) -> String {
    chrono::Utc::now()
        .with_timezone(&tz_of(s))
        .format("%Y-%m-%d")
        .to_string()
}

fn now_minutes_in(s: &HashMap<String, String>) -> i64 {
    use chrono::Timelike;
    let n = chrono::Utc::now().with_timezone(&tz_of(s));
    n.hour() as i64 * 60 + n.minute() as i64
}

fn currency(s: &HashMap<String, String>) -> &'static str {
    match s.get("currency").map(|x| x.as_str()).unwrap_or("ILS") {
        "USD" => "$",
        "JOD" => "د.أ",
        _ => "₪",
    }
}

fn hhmm(iso: Option<&String>) -> String {
    iso.and_then(|v| v.split('T').nth(1))
        .map(|t| t[..5.min(t.len())].to_string())
        .unwrap_or_else(|| "—".into())
}

fn month_label(period: &str) -> String {
    let mut it = period.split('-');
    let y = it.next().unwrap_or("");
    let m: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(1);
    format!("{} {}", MONTHS.get(m.saturating_sub(1)).unwrap_or(&""), y)
}

fn weekday_ar(date: &str) -> &'static str {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| {
            use chrono::Datelike;
            WEEKDAYS[d.weekday().num_days_from_monday() as usize]
        })
        .unwrap_or("")
}

/// يوحّد أرقام الهواتف للمقارنة: آخر 9 أرقام فقط (يتجاهل +970/00970/0).
pub fn normalize_phone(raw: &str) -> String {
    let digits: Vec<char> = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let tail: String = digits.iter().rev().take(9).collect::<Vec<_>>().into_iter().rev().collect();
    tail
}

/// هل الرقمان لنفس الشخص؟ يكفي تطابق آخر 9 أرقام.
/// يُتجاهل ما قبلها تماماً، فـ +970 و+972 و0 المحلية تُقارن كلها بنفس الطريقة.
fn phones_match(a: &str, b: &str) -> bool {
    let ta = normalize_phone(a);
    let tb = normalize_phone(b);
    ta.len() >= 7 && ta == tb
}

const BOT_BLOCKED: &str =
    "🚫 تم منع هذا الحساب من استخدام البوت.\nراجع الإدارة لإعادة السماح.";

const SUSPENDED: &str =
    "🚫 حسابك موقوف لدى الإدارة، ولا يمكن استخدام البوت حتى يُعاد تفعيله.\nراجع الإدارة.";

/// يطبّع الوقت HH:MM إلى دقائق.
fn hm_minutes(v: &str) -> i64 {
    let mut it = v.trim().split(':');
    let h: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(17);
    let m: i64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    h * 60 + m
}

// ------------------------------------------------------------------ اتصال API

fn api(token: &str, method: &str, body: Value, timeout_secs: u64) -> Result<Value, String> {
    if token.trim().is_empty() {
        return Err("لم يُضبط توكن البوت".into());
    }
    let url = format!("{API}{}/{method}", token.trim());
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("تعذّر الاتصال بتلجرام: {e}"))?;
    let v: Value = resp.json().map_err(|e| format!("رد غير مفهوم من تلجرام: {e}"))?;
    if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
        Ok(v)
    } else {
        Err(v
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or("خطأ من تلجرام")
            .to_string())
    }
}

fn send(token: &str, chat: &str, text: &str, keyboard: Option<Value>) -> Result<(), String> {
    // تلجرام يرفض الرسائل الأطول من 4096 حرفاً — نقسّمها.
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + 3800).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        let mut body = json!({
            "chat_id": chat,
            "text": chunk,
            "disable_web_page_preview": true
        });
        if start == 0 {
            if let Some(k) = &keyboard {
                body["reply_markup"] = k.clone();
            }
        }
        api(token, "sendMessage", body, 30)?;
        start = end;
    }
    Ok(())
}

fn contact_keyboard() -> Value {
    json!({
        "keyboard": [[{ "text": "📱 مشاركة رقم هاتفي", "request_contact": true }]],
        "resize_keyboard": true,
        "one_time_keyboard": true
    })
}

fn remove_keyboard() -> Value {
    json!({ "remove_keyboard": true })
}

// ------------------------------------------------------------------ أزرار جاهزة

const BTN_TODAY: &str = "📅 دوام اليوم";
const BTN_SHEET: &str = "📋 كشف الشهر";
const BTN_MONTH: &str = "🗓️ شهر آخر";
const BTN_SALARY: &str = "💰 آخر مدفوع";
const BTN_REASON: &str = "✍️ سبب الغياب";
const BTN_HELP: &str = "ℹ️ مساعدة";
const BTN_SUMMARY: &str = "📊 ملخص اليوم";

/// لوحة أزرار الموظف السفلية — تُخفي تلقائياً ما لا تسمح به الإعدادات.
/// is_sup = الشخص موظف ومشرف معاً (نفس الرقم) → يُضاف زر ملخص اليوم.
fn employee_menu(s: &HashMap<String, String>, is_sup: bool) -> Value {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut r1: Vec<Value> = Vec::new();
    if enabled(s, "tg_allow_today", true) {
        r1.push(json!({ "text": BTN_TODAY }));
    }
    if enabled(s, "tg_allow_sheet", true) {
        r1.push(json!({ "text": BTN_SHEET }));
    }
    if !r1.is_empty() {
        rows.push(r1);
    }
    let mut r2: Vec<Value> = Vec::new();
    if enabled(s, "tg_allow_month", true) {
        r2.push(json!({ "text": BTN_MONTH }));
    }
    if enabled(s, "tg_allow_salary", true) {
        r2.push(json!({ "text": BTN_SALARY }));
    }
    if !r2.is_empty() {
        rows.push(r2);
    }
    let mut r3: Vec<Value> = Vec::new();
    if enabled(s, "tg_allow_reason", true) {
        r3.push(json!({ "text": BTN_REASON }));
    }
    if is_sup {
        r3.push(json!({ "text": BTN_SUMMARY }));
    }
    r3.push(json!({ "text": BTN_HELP }));
    rows.push(r3);
    json!({ "keyboard": rows, "resize_keyboard": true })
}

fn supervisor_menu() -> Value {
    json!({
        "keyboard": [[{ "text": BTN_SUMMARY }, { "text": BTN_HELP }]],
        "resize_keyboard": true
    })
}

/// أزرار اختيار الشهر (آخر 6 أشهر) — بدل كتابة /كشف 2026-09.
fn months_keyboard(s: &HashMap<String, String>) -> Value {
    use chrono::Datelike;
    let now = chrono::Utc::now().with_timezone(&tz_of(s));
    let (mut y, mut m) = (now.year(), now.month() as i32);
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut row: Vec<Value> = Vec::new();
    for _ in 0..6 {
        let period = format!("{y:04}-{m:02}");
        // اسم الزر أرقام فقط (شهر/سنة) بدل الاسم العربي — أوضح للموظف.
        row.push(json!({
            "text": format!("{m:02}/{y}"),
            "callback_data": format!("m:{period}")
        }));
        if row.len() == 2 {
            rows.push(std::mem::take(&mut row));
        }
        m -= 1;
        if m == 0 {
            m = 12;
            y -= 1;
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    json!({ "inline_keyboard": rows })
}

/// أزرار أسباب الغياب الجاهزة (المفتاح لاتيني والسبب عربي).
fn reasons_keyboard() -> Value {
    json!({
        "inline_keyboard": [
            [
                { "text": "ظرف عائلي", "callback_data": "r:family" },
                { "text": "مرض", "callback_data": "r:sick" }
            ],
            [
                { "text": "إذن طارئ", "callback_data": "r:urgent" },
                { "text": "ظروف مواصلات", "callback_data": "r:transport" }
            ],
            [
                { "text": "✍️ سبب آخر (كتابة)", "callback_data": "r:write" }
            ]
        ]
    })
}

/// يحوّل مفتاح السبب إلى نصه العربي (يُرفض أي مفتاح غير معروف).
fn reason_from_key(k: &str) -> Option<&'static str> {
    match k {
        "family" => Some("ظرف عائلي"),
        "sick" => Some("مرض"),
        "urgent" => Some("إذن طارئ"),
        "transport" => Some("ظروف مواصلات"),
        _ => None,
    }
}

// ------------------------------------------------------------------ أوامر الواجهة

/// اختبار الاتصال بالبوت: يعيد اسم البوت (@username).
#[tauri::command]
pub fn telegram_test(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let (token, company) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings(&conn);
        (
            text_of(&s, "tg_token", ""),
            text_of(&s, "company_name", "الشركة"),
        )
    };
    if token.is_empty() {
        return Err("أدخل توكن البوت أولاً (من @BotFather)".into());
    }
    // لا بد من إلغاء أي webhook قديم حتى يعمل getUpdates
    let _ = api(&token, "deleteWebhook", json!({}), 20);
    let v = api(&token, "getMe", json!({}), 20)?;
    let name = v["result"]["username"].as_str().unwrap_or("bot").to_string();
    let _ = api(
        &token,
        "setMyDescription",
        json!({ "description": format!("بوت {company} — كشف الدوام والتنبيهات") }),
        20,
    );
    Ok(format!("@{name}"))
}

#[tauri::command]
pub fn telegram_supervisors(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TelegramSupervisor>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut st = conn
        .prepare(
            "SELECT id,name,phone,chat_id,linked_at,active FROM telegram_supervisors
             ORDER BY active DESC, id",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok(TelegramSupervisor {
                id: r.get(0)?,
                name: r.get(1)?,
                phone: r.get(2)?,
                chat_id: r.get(3)?,
                linked_at: r.get(4)?,
                active: r.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn telegram_add_supervisor(
    state: tauri::State<'_, AppState>,
    name: String,
    phone: String,
) -> Result<i64, String> {
    let name = name.trim().to_string();
    let phone = phone.trim().to_string();
    if name.is_empty() || normalize_phone(&phone).len() < 7 {
        return Err("أدخل اسم المشرف ورقم هاتفه كاملاً".into());
    }
    let stored = phone;
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    // منع تكرار نفس الرقم كمشرف (وإلا تكرّرت التنبيهات والرسائل).
    let dup = {
        let mut found = false;
        if let Ok(mut st) = conn.prepare("SELECT phone FROM telegram_supervisors") {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                found = rows.flatten().any(|ph| phones_match(&ph, &stored));
            }
        }
        found
    };
    if dup {
        return Err("هذا الرقم مضاف مسبقاً في قائمة المشرفين".into());
    }
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO telegram_supervisors(name,phone,active,created_at) VALUES(?1,?2,1,?3)",
        params![name, stored, now],
    )
    .map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    // إن كان الرقم مرتبطاً أصلاً كموظف، نربط سجل المشرف بنفس المحادثة فوراً
    // حتى تصل التنبيهات من دون أن يعيد الشخص مشاركة رقمه.
    let mut chat: Option<String> = None;
    if let Ok(mut st) = conn.prepare(
        "SELECT phone,telegram_chat_id FROM employees
         WHERE active=1 AND telegram_chat_id IS NOT NULL",
    ) {
        if let Ok(rows) = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) {
            for (ph, c) in rows.flatten() {
                if phones_match(&ph, &stored) {
                    chat = Some(c);
                    break;
                }
            }
        }
    }
    let mut notify: Option<(String, String, Value)> = None;
    if let Some(c) = chat {
        let _ = conn.execute(
            "UPDATE telegram_supervisors SET chat_id=?1, linked_at=?2 WHERE id=?3",
            params![c, now, id],
        );
        // نُبلّغه فوراً ونحدّث أزراره: من دون ذلك يبقى زر «ملخص اليوم»
        // غائباً حتى يرسل /start، فيظن أن صلاحيات المشرف لم تُفعّل.
        let st = settings(&conn);
        let token = text_of(&st, "tg_token", "");
        if !token.trim().is_empty() {
            notify = Some((token, c, employee_menu(&st, true)));
        }
    }
    drop(conn);

    if let Some((token, chat_id, keyboard)) = notify {
        let _ = send(
            &token,
            &chat_id,
            "✅ أُضيفت لك صلاحيات المشرف أيضاً — رقمك مسجّل كموظف ومشرف.\n\nأصبحت أزرارك تشمل «📊 ملخص اليوم»، وأمر /ملخص يعمل الآن.",
            Some(keyboard),
        );
    }
    Ok(id)
}

#[tauri::command]
pub fn telegram_delete_supervisor(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM telegram_supervisors WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn telegram_unlink_supervisor(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE telegram_supervisors SET chat_id=NULL, linked_at=NULL WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn telegram_links(state: tauri::State<'_, AppState>) -> Result<Vec<TelegramLink>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut st = conn
        .prepare(
            "SELECT id,name,phone,active,bot_allowed,telegram_chat_id,telegram_linked_at
             FROM employees ORDER BY active DESC, name",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok(TelegramLink {
                employee_id: r.get(0)?,
                name: r.get(1)?,
                phone: r.get(2)?,
                active: r.get::<_, i64>(3)? != 0,
                bot_allowed: r.get::<_, i64>(4).unwrap_or(1) != 0,
                chat_id: r.get(5)?,
                linked_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn telegram_unlink_employee(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE employees SET telegram_chat_id=NULL, telegram_linked_at=NULL WHERE id=?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn absence_notes_list(
    state: tauri::State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<AbsenceNote>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let lim = limit.unwrap_or(50).clamp(1, 500);
    let mut st = conn
        .prepare(
            "SELECT a.id,a.employee_id,e.name,a.date,a.reason,a.source,a.created_at
             FROM absence_notes a LEFT JOIN employees e ON e.id=a.employee_id
             ORDER BY a.date DESC, a.id DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([lim], |r| {
            Ok(AbsenceNote {
                id: r.get(0)?,
                employee_id: r.get(1)?,
                employee_name: r.get(2).unwrap_or_else(|_| "—".into()),
                date: r.get(3)?,
                reason: r.get(4)?,
                source: r.get(5)?,
                created_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

// ------------------------------------------------------------- السماح بالبوت

/// منع/سماح موظف باستخدام بوت تلجرام — نفس القيمة المعروضة في شاشة الموظفين.
#[tauri::command]
pub fn telegram_set_bot_allowed(
    state: tauri::State<'_, AppState>,
    id: i64,
    allowed: bool,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "UPDATE employees SET bot_allowed=?1, updated_at=?2 WHERE id=?3",
        params![allowed as i64, now, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------ التعميمات

/// محادثات مستلمي التعميم: الجميع النشطون المرتبطون، أو موظف/مشرف بعينه.
fn broadcast_chats(conn: &Connection, target: &str) -> Result<Vec<String>, String> {
    if target == "all" {
        let mut chats: Vec<String> = Vec::new();
        if let Ok(mut st) = conn.prepare(
            "SELECT telegram_chat_id FROM employees
             WHERE active=1 AND bot_allowed=1 AND telegram_chat_id IS NOT NULL
             UNION
             SELECT chat_id FROM telegram_supervisors
             WHERE active=1 AND chat_id IS NOT NULL",
        ) {
            if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                chats = rows.flatten().filter(|c| !c.trim().is_empty()).collect();
            }
        }
        if chats.is_empty() {
            return Err("لا يوجد أي مستخدم مرتبط بالبوت بعد".into());
        }
        return Ok(chats);
    }
    if let Some(id) = target.strip_prefix("emp:").and_then(|x| x.parse::<i64>().ok()) {
        let chat: Option<String> = conn
            .query_row(
                "SELECT telegram_chat_id FROM employees
                 WHERE id=?1 AND active=1 AND bot_allowed=1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        return match chat {
            Some(c) if !c.trim().is_empty() => Ok(vec![c]),
            _ => Err("هذا الموظف غير مرتبط بالبوت أو حسابه موقوف أو ممنوع من البوت".into()),
        };
    }
    if let Some(id) = target.strip_prefix("sup:").and_then(|x| x.parse::<i64>().ok()) {
        let chat: Option<String> = conn
            .query_row(
                "SELECT chat_id FROM telegram_supervisors WHERE id=?1 AND active=1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        return match chat {
            Some(c) if !c.trim().is_empty() => Ok(vec![c]),
            _ => Err("هذا المشرف غير مرتبط بالبوت أو حسابه موقوف".into()),
        };
    }
    Err("مستلم غير معروف".into())
}

/// إرسال تعميم من البرنامج إلى مستخدمي البوت.
/// target: "all" لكل المرتبطين النشطين، أو "emp:<id>" لموظف، أو "sup:<id>" لمشرف.
/// يعيد (عدد النجاح، عدد الفشل).
#[tauri::command]
pub fn telegram_broadcast(
    state: tauri::State<'_, AppState>,
    text: String,
    target: String,
) -> Result<(usize, usize), String> {
    let msg = text.trim().to_string();
    if msg.is_empty() {
        return Err("اكتب نص التعميم أولاً".into());
    }
    if msg.chars().count() > 3500 {
        return Err("نص التعميم طويل جداً (الحد 3500 حرف)".into());
    }

    let (token, chats) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let s = settings(&conn);
        if !enabled(&s, "tg_enabled", false) {
            return Err("بوت تلجرام موقوف — فعّله من القسم 1 أولاً".into());
        }
        let token = text_of(&s, "tg_token", "");
        if token.trim().is_empty() {
            return Err("لم يُضبط توكن البوت بعد".into());
        }
        (token, broadcast_chats(&conn, &target)?)
    };

    let mut sent = 0usize;
    let mut failed = 0usize;
    for chat in chats {
        if send(&token, &chat, &msg, None).is_ok() {
            sent += 1;
        } else {
            failed += 1;
        }
        // فاصل بسيط احتراماً لحدود تلجرام (30 رسالة/ثانية).
        std::thread::sleep(Duration::from_millis(60));
    }
    if sent == 0 && failed > 0 {
        return Err("تعذّر إرسال التعميم — تحقّق من اتصال الإنترنت وصحة التوكن".into());
    }
    Ok((sent, failed))
}

// ------------------------------------------------------------------ التشغيل

/// يبدأ حلقة قراءة رسائل البوت (تعمل طالما التطبيق مشغّل).
pub fn spawn_bot(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let state = app.state::<AppState>();
        let (on, token, offset) = {
            let Ok(conn) = state.db.lock() else {
                std::thread::sleep(Duration::from_secs(10));
                continue;
            };
            let s = settings(&conn);
            (
                enabled(&s, "tg_enabled", false),
                text_of(&s, "tg_token", ""),
                s.get("tg_offset")
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(0),
            )
        };
        if token.is_empty() {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        if !on {
            // البوت متوقف: نتجاهل أي رسائل تصل أثناء الإيقاف (نقدّم المؤشر فقط
            // من دون الرد عليها، حتى لا يُردّ عليها عند إعادة التفعيل).
            let body = json!({
                "offset": offset,
                "timeout": 0,
                "allowed_updates": ["message", "callback_query"]
            });
            if let Ok(v) = api(&token, "getUpdates", body, 10) {
                if let Some(ups) = v["result"].as_array() {
                    if let Some(last) = ups.last().and_then(|u| u["update_id"].as_i64()) {
                        if let Ok(conn) = app.state::<AppState>().db.lock() {
                            let _ = db::set_setting(&conn, "tg_offset", &(last + 1).to_string());
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        let body = json!({
            "offset": offset,
            "timeout": 20,
            "allowed_updates": ["message", "callback_query"]
        });
        match api(&token, "getUpdates", body, 35) {
            Ok(v) => {
                if let Some(ups) = v["result"].as_array() {
                    for u in ups {
                        handle_update(&app, &token, u);
                        if let Some(id) = u["update_id"].as_i64() {
                            if let Ok(conn) = app.state::<AppState>().db.lock() {
                                let _ = db::set_setting(&conn, "tg_offset", &(id + 1).to_string());
                            }
                        }
                    }
                }
                eod_tick(&app, &token);
                std::thread::sleep(Duration::from_millis(400));
            }
            Err(e) => {
                log::warn!("telegram getUpdates: {e}");
                std::thread::sleep(Duration::from_secs(6));
            }
        }
    });
}

fn handle_update(app: &tauri::AppHandle, token: &str, u: &Value) {
    // فحص فوري: إن أوقف المسؤول البوت أثناء انتظار getUpdates، لا نرد على أي رسالة.
    {
        let state = app.state::<AppState>();
        let Ok(conn) = state.db.lock() else { return };
        let s = settings(&conn);
        if !enabled(&s, "tg_enabled", false) {
            return;
        }
    }
    if let Some(cb) = u.get("callback_query") {
        if cb.is_object() {
            handle_callback(app, token, cb);
            return;
        }
    }
    let msg = &u["message"];
    if !msg.is_object() {
        return;
    }
    let Some(chat_id) = msg["chat"]["id"].as_i64().map(|x| x.to_string()) else {
        return;
    };
    let from_id = msg["from"]["id"]
        .as_i64()
        .map(|x| x.to_string())
        .unwrap_or_default();

    if let Some(c) = msg.get("contact") {
        // نرفض أي جهة اتصال ليست لصاحب الحساب نفسه.
        if c["user_id"].as_i64().map(|x| x.to_string()).as_deref() != Some(from_id.as_str()) {
            return;
        }
        let phone = c["phone_number"].as_str().unwrap_or("");
        link_contact(app, token, &chat_id, phone);
        return;
    }

    let text = msg["text"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return;
    }
    handle_text(app, token, &chat_id, &text);
}

/// يعالج ضغطات الأزرار داخل تلجرام (اختيار شهر أو سبب غياب).
fn handle_callback(app: &tauri::AppHandle, token: &str, cb: &Value) {
    let cq_id = cb["id"].as_str().unwrap_or("");
    let data = cb["data"].as_str().unwrap_or("");
    let Some(chat) = cb["message"]["chat"]["id"].as_i64().map(|x| x.to_string()) else {
        return;
    };

    // إيقاف مؤشر الانتظار عند العميل فوراً
    if !cq_id.is_empty() {
        let _ = api(
            token,
            "answerCallbackQuery",
            json!({ "callback_query_id": cq_id }),
            15,
        );
    }
    if data.is_empty() {
        return;
    }

    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else { return };
    let s = settings(&conn);
    let emp = employee_by_chat(&conn, &chat);
    let sup = supervisor_by_chat(&conn, &chat);
    let suspended = if emp.is_none() && sup.is_none() {
        suspended_employee(&conn, &chat).or_else(|| suspended_supervisor(&conn, &chat))
    } else {
        None
    };
    let blocked = if emp.is_none() && sup.is_none() {
        bot_blocked_employee(&conn, &chat)
    } else {
        None
    };

    let mut reply: Option<String> = None;
    let mut keyboard: Option<Value> = None;
    let mut absence_alert: Option<String> = None;

    if let Some(name) = &suspended {
        reply = Some(format!("{SUSPENDED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if let Some(name) = &blocked {
        reply = Some(format!("{BOT_BLOCKED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if let Some((emp_id, name, _)) = emp {
        // الشخص قد يكون موظفاً ومشرفاً — أزرار الموظف تعمل، ويبقى زر الملخص.
        let is_sup = sup.is_some();
        if let Some(period) = data.strip_prefix("m:") {
            keyboard = Some(employee_menu(&s, is_sup));
            if !enabled(&s, "tg_allow_month", true) {
                reply = Some(PERM_DENIED.into());
            } else if !valid_period(period) {
                reply = Some("📅 شهر غير صالح.".into());
            } else {
                reply = Some(sheet_text(&conn, emp_id, period));
            }
        } else if let Some(key) = data.strip_prefix("r:") {
            keyboard = Some(employee_menu(&s, is_sup));
            if !enabled(&s, "tg_allow_reason", true) {
                reply = Some(PERM_DENIED.into());
            } else if key == "write" {
                reply = Some("✍️ اكتب السبب بعد الأمر، مثال:\n/سبب ظرف عائلي".into());
            } else if let Some(reason) = reason_from_key(key) {
                let day = today_in(&s);
                let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                match conn.execute(
                    "INSERT INTO absence_notes(employee_id,date,reason,source,created_at)
                     VALUES(?1,?2,?3,'telegram',?4)
                     ON CONFLICT(employee_id,date) DO UPDATE SET
                       reason=excluded.reason, created_at=excluded.created_at",
                    params![emp_id, day, reason, now],
                ) {
                    Ok(_) => {
                        reply = Some(format!(
                            "✅ سُجّل سبب غياب اليوم ({day}):\n{reason}\n\nوصل التنبيه للإدارة."
                        ));
                        absence_alert = Some(format!("{name} — {day}\n{reason}"));
                    }
                    Err(e) => {
                        reply = Some(format!("⚠️ تعذّر الحفظ: {e}"));
                    }
                }
            }
        }
    } else if sup.is_some() {
        reply = Some("هذه الأزرار خاصة بالموظفين. استخدم الأزرار أدناه.".into());
        keyboard = Some(supervisor_menu());
    }

    drop(conn);

    if let Some(m) = reply {
        let _ = send(token, &chat, &m, keyboard);
    }
    if let Some(body) = absence_alert {
        notify_supervisors_async(app, "📝 سبب غياب", &body);
    }
}

/// ربط المحادثة برقم الهاتف (موظف و/أو مشرف).
/// الشخص قد يكون موظفاً ومشرفاً بنفس الرقم — عندها يُربط الدورين معاً.
fn link_contact(app: &tauri::AppHandle, token: &str, chat: &str, phone: &str) {
    let norm = normalize_phone(phone);
    if norm.len() < 7 {
        let _ = send(token, chat, "⚠️ تعذّر قراءة رقم الهاتف. جرّب زر «مشاركة رقم هاتفي» مرة أخرى.", Some(contact_keyboard()));
        return;
    }
    let state = app.state::<AppState>();
    let linked: Option<(String, Option<Value>)> = {
        let Ok(conn) = state.db.lock() else { return };

        // بحث مستقل عن الدورين بنفس الرقم (قد يكون الشخص الاثنين معاً).
        let mut emp: Option<(i64, String, bool, bool)> = None;
        if let Ok(mut st) =
            conn.prepare("SELECT id,name,active,phone,bot_allowed FROM employees")
        {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4).unwrap_or(1) != 0,
                ))
            }) {
                for (id, name, active, ph, allowed) in rows.flatten() {
                    if !ph.trim().is_empty() && phones_match(&ph, phone) {
                        emp = Some((id, name, active, allowed));
                        break;
                    }
                }
            }
        }
        let mut sup: Option<(i64, String)> = None;
        if let Ok(mut st) =
            conn.prepare("SELECT id,name,phone FROM telegram_supervisors WHERE active=1")
        {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            }) {
                for (id, name, ph) in rows.flatten() {
                    if phones_match(&ph, phone) {
                        sup = Some((id, name));
                        break;
                    }
                }
            }
        }

        if emp.is_none() && sup.is_none() {
            None
        } else if sup.is_none() && emp.as_ref().map(|e| !e.2).unwrap_or(false) {
            // موظف موقوف وليس مشرفاً نشطاً → منع صريح.
            let name = emp.as_ref().map(|e| e.1.clone()).unwrap_or_default();
            Some((format!("{SUSPENDED}\n\nالحساب: {name}"), None))
        } else if sup.is_none() && emp.as_ref().map(|e| !e.3).unwrap_or(false) {
            // موظف ممنوع من البوت وليس مشرفاً نشطاً → منع صريح.
            let name = emp.as_ref().map(|e| e.1.clone()).unwrap_or_default();
            Some((format!("{BOT_BLOCKED}\n\nالحساب: {name}"), None))
        } else {
            let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
            // نُزيل أي ربط سابق لهذه المحادثة في الجدولين ثم نربط الأدوار المطابقة.
            let _ = conn.execute(
                "UPDATE employees SET telegram_chat_id=NULL, telegram_linked_at=NULL
                 WHERE telegram_chat_id=?1",
                params![chat],
            );
            let _ = conn.execute(
                "UPDATE telegram_supervisors SET chat_id=NULL, linked_at=NULL WHERE chat_id=?1",
                params![chat],
            );

            let mut lines: Vec<String> = Vec::new();
            let mut emp_linked = false;
            if let Some((id, name, active, allowed)) = &emp {
                if *active && *allowed {
                    let _ = conn.execute(
                        "UPDATE employees SET telegram_chat_id=?1, telegram_linked_at=?2 WHERE id=?3",
                        params![chat, now, id],
                    );
                    lines.push(format!("👤 موظف: {name}"));
                    emp_linked = true;
                }
            }
            if let Some((id, name)) = &sup {
                let _ = conn.execute(
                    "UPDATE telegram_supervisors SET chat_id=?1, linked_at=?2 WHERE id=?3",
                    params![chat, now, id],
                );
                lines.push(format!("👨‍💼 مشرف: {name}"));
            }

            let s = settings(&conn);
            let dual = emp_linked && sup.is_some();
            let title = if dual {
                "✅ تم ربط حسابك (موظف + مشرف)"
            } else if emp_linked {
                "✅ تم ربط حسابك"
            } else {
                "✅ تم ربط حساب المشرف"
            };
            let menu = if emp_linked {
                employee_menu(&s, sup.is_some())
            } else {
                supervisor_menu()
            };
            let extra = if dual {
                "\n\nتصلك تنبيهات الموظفين كمشرف أيضاً."
            } else {
                ""
            };
            Some((
                format!(
                    "{title}\n{}\n\nاختر من الأزرار أدناه، أو أرسل /مساعدة.{extra}",
                    lines.join("\n")
                ),
                Some(menu),
            ))
        }
    };

    match linked {
        Some((msg, kb)) => {
            let _ = send(token, chat, &msg, kb);
        }
        None => {
            let _ = send(
                token,
                chat,
                "⚠️ رقمك غير مسجّل لدى الإدارة.\nتأكد أن رقم هاتفك مسجّل في بطاقة الموظف (أو ضمن المشرفين) ثم أعد المحاولة.",
                Some(contact_keyboard()),
            );
        }
    }
}

fn employee_by_chat(conn: &Connection, chat: &str) -> Option<(i64, String, String)> {
    conn.query_row(
        "SELECT id,name,phone FROM employees
         WHERE telegram_chat_id=?1 AND active=1 AND bot_allowed=1",
        params![chat],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .ok()
}

fn supervisor_by_chat(conn: &Connection, chat: &str) -> Option<(String, String)> {
    conn.query_row(
        "SELECT name,phone FROM telegram_supervisors WHERE chat_id=?1 AND active=1",
        params![chat],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// موظف مرتبط لكنه موقوف (active=0): يُميَّز عن «غير المرتبط» برسالة واضحة،
/// ويُمنع من كل الأوامر والأزرار حتى يُعاد تفعيله من شاشة الموظفين.
fn suspended_employee(conn: &Connection, chat: &str) -> Option<String> {
    conn.query_row(
        "SELECT name FROM employees WHERE telegram_chat_id=?1 AND active=0",
        params![chat],
        |r| r.get(0),
    )
    .ok()
}

/// مشرف مرتبط لكنه موقوف.
fn suspended_supervisor(conn: &Connection, chat: &str) -> Option<String> {
    conn.query_row(
        "SELECT name FROM telegram_supervisors WHERE chat_id=?1 AND active=0",
        params![chat],
        |r| r.get(0),
    )
    .ok()
}

/// موظف مرتبط لكن الإدارة منعته من استخدام البوت (bot_allowed=0).
fn bot_blocked_employee(conn: &Connection, chat: &str) -> Option<String> {
    conn.query_row(
        "SELECT name FROM employees
         WHERE telegram_chat_id=?1 AND active=1 AND bot_allowed=0",
        params![chat],
        |r| r.get(0),
    )
    .ok()
}

fn handle_text(app: &tauri::AppHandle, token: &str, chat: &str, raw: &str) {
    // أزرار اللوحة تُترجم إلى الأوامر نفسها، فتسلك نفس المسار والصلاحيات.
    let raw_text = raw.trim();
    let mapped = match raw_text {
        BTN_TODAY => "اليوم",
        BTN_SHEET => "كشف",
        BTN_SALARY | "💰 صافي الراتب" | "آخر مدفوع" | "اخر مدفوع" => "راتب",
        BTN_MONTH => "شهور",
        BTN_REASON => "أسباب",
        BTN_HELP => "مساعدة",
        BTN_SUMMARY => "ملخص",
        _ => raw_text,
    };
    let text = mapped.trim_start_matches('/').trim();
    let lower = text.to_lowercase();
    let (cmd, arg) = match text.split_once(char::is_whitespace) {
        Some((a, b)) => (a, b.trim()),
        None => (text, ""),
    };

    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else { return };
    let s = settings(&conn);

    // من هو صاحب المحادثة؟
    let emp = employee_by_chat(&conn, chat);
    let sup = supervisor_by_chat(&conn, chat);

    // موقوف؟ (الحساب مربوط لكن الإدارة أوقفته) — يُمنع من البوت كلياً.
    let suspended = if emp.is_none() && sup.is_none() {
        suspended_employee(&conn, chat).or_else(|| suspended_supervisor(&conn, chat))
    } else {
        None
    };
    // ممنوع من البوت؟ (الموظف نشط لكن الإدارة منعته) — إلا إذا كان مشرفاً نشطاً أيضاً.
    let blocked = if emp.is_none() && sup.is_none() {
        bot_blocked_employee(&conn, chat)
    } else {
        None
    };

    // الرد يُحسب والقفل ممسوك (قراءة قاعدة البيانات فقط)، ثم يُرسل بعد فكّه
    // حتى لا تتجمّد الواجهة أثناء انتظار شبكة تلجرام.
    let mut reply: Option<String> = None;
    let mut keyboard: Option<Value> = None;
    let mut absence_alert: Option<String> = None;

    // /start أو أي رسالة من غير مرتبط → طلب مشاركة الرقم
    if let Some(name) = &suspended {
        reply = Some(format!("{SUSPENDED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if let Some(name) = &blocked {
        reply = Some(format!("{BOT_BLOCKED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if cmd.eq_ignore_ascii_case("start") || lower == "ابدأ" || lower == "بدء" {
        if let Some((_, name, _)) = &emp {
            let dual = sup.is_some();
            reply = Some(format!(
                "👋 أهلاً {name}{}\n\nاختر ما تريد من الأزرار أدناه.",
                if dual { " (موظف ومشرف)" } else { "" }
            ));
            keyboard = Some(employee_menu(&s, dual));
        } else if let Some((name, _)) = &sup {
            reply = Some(format!("👋 أهلاً {name}\n\nاختر من الأزرار أدناه."));
            keyboard = Some(supervisor_menu());
        } else {
            reply = Some(
                "👋 أهلاً بك في بوت الحضور.\n\nلربط حسابك اضغط الزر أدناه لمشاركة رقم هاتفك المسجّل لدى الشركة."
                    .into(),
            );
            keyboard = Some(contact_keyboard());
        }
    } else if emp.is_none() && sup.is_none() {
        reply = Some(
            "🔗 حسابك غير مرتبط بعد.\nاضغط «📱 مشاركة رقم هاتفي» لربطه برقمك المسجّل لدى الإدارة."
                .into(),
        );
        keyboard = Some(contact_keyboard());
    } else if cmd == "الغاء" || cmd == "إلغاء" || lower == "unlink" {
        // إلغاء الربط
        let _ = conn.execute(
            "UPDATE employees SET telegram_chat_id=NULL, telegram_linked_at=NULL WHERE telegram_chat_id=?1",
            params![chat],
        );
        let _ = conn.execute(
            "UPDATE telegram_supervisors SET chat_id=NULL, linked_at=NULL WHERE chat_id=?1",
            params![chat],
        );
        reply = Some("تم إلغاء ربط هذه المحادثة. لإعادة الربط أرسل /start.".into());
        keyboard = Some(remove_keyboard());
    } else if let Some((emp_id, name, _)) = emp {
        let period_now = today_in(&s)[..7].to_string();
        let is_sup = sup.is_some();
        keyboard = Some(employee_menu(&s, is_sup));
        if is_sup && (cmd == "ملخص" || cmd == "تقارير") {
            reply = Some(day_summary(&conn).unwrap_or_else(|e| e));
        } else if cmd == "شهور" {
            if !enabled(&s, "tg_allow_month", true) {
                reply = Some(PERM_DENIED.into());
            } else {
                reply = Some("🗓️ اختر الشهر الذي تريد كشفه:".into());
                keyboard = Some(months_keyboard(&s));
            }
        } else if cmd == "أسباب" {
            if !enabled(&s, "tg_allow_reason", true) {
                reply = Some(PERM_DENIED.into());
            } else {
                reply = Some("✍️ اختر سبب الغياب من الأزرار، أو اكتبه بعد /سبب:".into());
                keyboard = Some(reasons_keyboard());
            }
        } else {
        match cmd {
            "مساعدة" | "help" | "أوامر" => {
                let help = if is_sup {
                    format!("{}\n\n{SUP_HELP}", employee_help(&s))
                } else {
                    employee_help(&s)
                };
                reply = Some(format!("{HELP_HEADER}\n{name}\n\n{help}"));
            }
            "كشف" | "كشف_الدوام" => {
                if arg.is_empty() {
                    if !enabled(&s, "tg_allow_sheet", true) {
                        reply = Some(PERM_DENIED.into());
                    } else {
                        reply = Some(sheet_text(&conn, emp_id, &period_now));
                    }
                } else if !enabled(&s, "tg_allow_month", true) {
                    reply = Some(PERM_DENIED.into());
                } else if !valid_period(arg) {
                    reply = Some("📅 أرسل الشهر بالصيغة: /كشف 2026-09".into());
                } else {
                    reply = Some(sheet_text(&conn, emp_id, arg));
                }
            }
            "اليوم" | "حالة" | "حالتي" => {
                reply = Some(if !enabled(&s, "tg_allow_today", true) {
                    PERM_DENIED.into()
                } else {
                    today_text(&conn, emp_id, &s)
                });
            }
            "راتب" | "الراتب" => {
                reply = Some(if !enabled(&s, "tg_allow_salary", true) {
                    PERM_DENIED.into()
                } else {
                    salary_text(&conn, emp_id, &s).unwrap_or_else(|e| e)
                });
            }
            "سبب" | "غياب" => {
                if !enabled(&s, "tg_allow_reason", true) {
                    reply = Some(PERM_DENIED.into());
                } else if arg.is_empty() {
                    reply = Some("✍️ أرسل السبب بعد الأمر، مثال:\n/سبب ظرف عائلي".into());
                } else {
                    let day = today_in(&s);
                    let reason = arg.chars().take(300).collect::<String>();
                    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                    match conn.execute(
                        "INSERT INTO absence_notes(employee_id,date,reason,source,created_at)
                         VALUES(?1,?2,?3,'telegram',?4)
                         ON CONFLICT(employee_id,date) DO UPDATE SET
                           reason=excluded.reason, created_at=excluded.created_at",
                        params![emp_id, day, reason, now],
                    ) {
                        Ok(_) => {
                            reply = Some(format!(
                                "✅ سُجّل سبب غياب اليوم ({day}):\n{reason}\n\nوصل التنبيه للإدارة."
                            ));
                            absence_alert = Some(format!("{name} — {day}\n{reason}"));
                        }
                        Err(e) => {
                            reply = Some(format!("⚠️ تعذّر الحفظ: {e}"));
                        }
                    }
                }
            }
            _ => {
                reply = Some(format!(
                    "🤖 لم أفهم «{text}».\n\n{}",
                    if is_sup {
                        format!("{}\n\n{SUP_HELP}", employee_help(&s))
                    } else {
                        employee_help(&s)
                    }
                ));
            }
        }
        }
    } else if let Some((name, _)) = &sup {
        // لوحة المشرف (مشرف فقط، بلا حساب موظف)
        reply = Some(match cmd {
            "ملخص" | "اليوم" | "تقارير" => day_summary(&conn).unwrap_or_else(|e| e),
            "مساعدة" | "help" => SUP_HELP.to_string(),
            _ => format!("👋 {name}\n\n{SUP_HELP}"),
        });
        keyboard = Some(supervisor_menu());
    }

    drop(conn);

    if let Some(m) = reply {
        let _ = send(token, chat, &m, keyboard);
    }
    if let Some(body) = absence_alert {
        notify_supervisors_async(app, "📝 سبب غياب", &body);
    }
}

const PERM_DENIED: &str = "🚫 هذا الخيار غير مسموح من الإدارة.";
const HELP_HEADER: &str = "🤖 أوامر بوت الحضور";
const SUP_HELP: &str = "👨‍💼 أوامر المشرف:\n/ملخص — ملخص حضور اليوم\n/مساعدة — هذه القائمة\n/الغاء — إلغاء ربط المحادثة\n\nأو استخدم الأزرار الجاهزة أسفل المحادثة.";

fn employee_help(s: &HashMap<String, String>) -> String {
    let mut out = String::from("يمكنك الضغط على الأزرار الجاهزة بدل كتابة الأوامر.\n\n");

    if enabled(s, "tg_allow_today", true) {
        out.push_str("/اليوم — هل ختمت دخولك/خروجك اليوم؟\n");
    }
    if enabled(s, "tg_allow_sheet", true) {
        out.push_str("/كشف — كشف دوام الشهر الحالي\n");
    }
    if enabled(s, "tg_allow_month", true) {
        out.push_str("/كشف 2026-09 — كشف شهر محدد\n");
    }
    if enabled(s, "tg_allow_salary", true) {
        out.push_str("/راتب — آخر مدفوع\n");
    }
    if enabled(s, "tg_allow_reason", true) {
        out.push_str("/سبب ... — تسجيل سبب الغياب\n");
    }
    out.push_str("/الغاء — إلغاء ربط المحادثة\n");
    out.push_str("/مساعدة — هذه القائمة");
    out
}

fn valid_period(p: &str) -> bool {
    let b = p.as_bytes();
    if b.len() != 7 || b[4] != b'-' {
        return false;
    }
    b[..4].iter().all(|c| c.is_ascii_digit()) && b[5..].iter().all(|c| c.is_ascii_digit())
}

// ------------------------------------------------------------------ بناء الرسائل

fn sheet_text(conn: &Connection, emp_id: i64, period: &str) -> String {
    let days = match commands::month_days_impl(conn, period) {
        Ok(d) => d,
        Err(e) => return format!("⚠️ تعذّر جلب الكشف: {e}"),
    };
    let mine: Vec<&AttendanceDay> = days.iter().filter(|d| d.employee_id == emp_id).collect();
    if mine.is_empty() {
        return format!("📭 لا توجد أيام حضور مسجّلة لك في {}.", month_label(period));
    }
    let mut out = format!("📋 كشف دوامك — {}\n", month_label(period));
    let mut present = 0;
    let mut minutes = 0;
    let mut late = 0;
    for d in &mine {
        let dd = d.date.get(8..10).unwrap_or("");
        let mm = d.date.get(5..7).unwrap_or("");
        let wd = weekday_ar(&d.date);
        let line = match d.status.as_str() {
            "weekend" => format!("{dd}/{mm} {wd} — عطلة أسبوعية"),
            "holiday" => format!("{dd}/{mm} {wd} — عطلة رسمية"),
            "leave" => format!(
                "{dd}/{mm} {wd} — إجازة{}",
                if d.leave_paid == Some(true) { " مدفوعة" } else { "" }
            ),
            "absent" => format!("{dd}/{mm} {wd} — غياب"),
            _ => {
                if d.first_in.is_some() {
                    present += 1;
                    minutes += d.worked_min;
                    late += d.late_min;
                }
                format!(
                    "{dd}/{mm} {wd} — {} ← {}  ({} س)",
                    hhmm(d.first_in.as_ref()),
                    hhmm(d.last_out.as_ref()),
                    (d.worked_min as f64 / 60.0)
                )
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&format!(
        "\n📊 الإجمالي: {present} يوم · {:.1} ساعة{}",
        minutes as f64 / 60.0,
        if late > 0 { format!(" · تأخير {late} دقيقة") } else { String::new() }
    ));
    out
}

fn today_text(conn: &Connection, emp_id: i64, s: &HashMap<String, String>) -> String {
    let today = today_in(s);
    let period = today[..7].to_string();
    let days = commands::month_days_impl(conn, &period).unwrap_or_default();
    match days.iter().find(|d| d.employee_id == emp_id && d.date == today) {
        Some(d) => {
            if d.status == "weekend" {
                format!("🌴 اليوم {today} عطلة أسبوعية — لا يلزم الختم.")
            } else if d.status == "holiday" {
                format!("🎉 اليوم {today} عطلة رسمية — لا يلزم الختم.")
            } else if d.status == "leave" {
                format!("🗓️ اليوم {today} إجازة مسجّلة لك.")
            } else if let Some(fin) = &d.first_in {
                match &d.last_out {
                    Some(out) => format!(
                        "📅 اليوم {today}\n✅ دخول: {}\n✅ خروج: {}\n⏱️ الساعات: {:.1}",
                        hhmm(Some(fin)),
                        hhmm(Some(out)),
                        d.worked_min as f64 / 60.0
                    ),
                    None => format!(
                        "📅 اليوم {today}\n✅ دخول: {}\n⏳ لم تسجّل انصرافك بعد — لا تنسَ الختم عند المغادرة.",
                        hhmm(Some(fin))
                    ),
                }
            } else {
                format!(
                    "📅 اليوم {today}\n❌ لم يُسجَّل حضورك.\nإن كان لديك عذر أرسله: /سبب <السبب>",
                )
            }
        }
        None => format!(
            "📅 اليوم {today}\n❌ لا يوجد تسجيل حضور لك اليوم.\nإن كان لديك عذر أرسله: /سبب <السبب>",
        ),
    }
}

/// صافي آخر شهر علّمته الإدارة «مدفوعاً» لهذا الموظف في صفحة الرواتب.
/// يعتمد المبلغ الفعلي المسجَّل وقت الدفع، وتُعرض التفاصيل من اللقطة المحفوظة.
fn salary_text(
    conn: &Connection,
    emp_id: i64,
    s: &HashMap<String, String>,
) -> Result<String, String> {
    let paid: Option<(String, f64, String)> = conn
        .query_row(
            "SELECT period, net, paid_at FROM salary_payments
             WHERE employee_id=?1 ORDER BY period DESC, paid_at DESC LIMIT 1",
            params![emp_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((period, net, paid_at)) = paid else {
        return Err(
            "💤 لا يوجد راتب مدفوع لك حتى الآن.\nراجع الإدارة إن كنت تتوقع راتباً مدفوعاً."
                .into(),
        );
    };
    let name: String = conn
        .query_row(
            "SELECT name FROM employees WHERE id=?1",
            params![emp_id],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let cur = currency(s);
    let snap: Option<(i64, f64, f64, f64, f64, f64, f64)> = conn
        .query_row(
            "SELECT days_present,total_worked_hours,overtime_hours,base_pay,
                    allowances,weekend_pay,advance_deduction
             FROM payroll_runs WHERE period=?1 AND employee_id=?2",
            params![period, emp_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .ok();

    let mut out = format!(
        "💰 {}\n\nالشهر: {} ({period})\nالصافي المدفوع: {:.2} {cur}\nتاريخ الدفع: {}",
        name.trim(),
        month_label(&period),
        net,
        &paid_at[..paid_at.len().min(10)]
    );
    if let Some((days, hours, ot, base, allow, weekend, adv)) = snap {
        out.push_str(&format!(
            "\n\n• أيام الحضور: {days}\n• الساعات: {hours:.1}\n• الإضافي: {ot:.1} ساعة\n• الأساسي: {base:.2} {cur}\n• البدلات: {allow:.2} {cur}\n• بدل العطلة: {weekend:.2} {cur}"
        ));
        if adv > 0.0 {
            out.push_str(&format!("\n• خصم السلفة: − {adv:.2} {cur}"));
        }
    }
    Ok(out)
}

/// ملخص اليوم للمشرف: من حضر، من لم يسجّل خروجاً، ومن غاب.
pub fn day_summary(conn: &Connection) -> Result<String, String> {
    let s = settings(conn);
    let today = today_in(&s);
    let period = today[..7].to_string();
    let days = commands::month_days_impl(conn, &period)?;
    let today_rows: Vec<&AttendanceDay> = days.iter().filter(|d| d.date == today).collect();

    let mut present: Vec<(String, String, String)> = Vec::new(); // name, in, out
    let mut missing_out: Vec<(String, String)> = Vec::new();
    let mut with_in: std::collections::HashSet<i64> = Default::default();
    for d in &today_rows {
        if let Some(fin) = &d.first_in {
            with_in.insert(d.employee_id);
            match &d.last_out {
                Some(out) => present.push((
                    employee_name(conn, d.employee_id),
                    hhmm(Some(fin)),
                    hhmm(Some(out)),
                )),
                None => missing_out.push((employee_name(conn, d.employee_id), hhmm(Some(fin)))),
            }
        }
    }

    let mut out = format!("📊 ملخص الحضور — {today}\n\n");
    out.push_str(&format!("✅ ختموا دخولاً وخروجاً ({}):\n", present.len()));
    if present.is_empty() {
        out.push_str("— لا أحد\n");
    }
    for (n, i, o) in present.iter().take(40) {
        out.push_str(&format!("• {n} ({i} ← {o})\n"));
    }
    out.push_str(&format!("\n⏳ دخلوا ولم يسجّلوا خروجاً ({}):\n", missing_out.len()));
    if missing_out.is_empty() {
        out.push_str("— لا أحد\n");
    }
    for (n, i) in missing_out.iter().take(40) {
        out.push_str(&format!("• {n} (دخول {i})\n"));
    }

    // الغائبون: موظفون فعّالون بلا دخول اليوم وليس يوم عطلة/إجازة
    let weekday_idx = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
        .map(|d| {
            use chrono::Datelike;
            d.weekday().num_days_from_monday()
        })
        .unwrap_or(0);
    let weekend = crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()));
    let holiday = conn
        .query_row(
            "SELECT COUNT(*) FROM holidays WHERE date=?1",
            params![today],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    let leaves: std::collections::HashSet<i64> = today_rows
        .iter()
        .filter(|d| d.status == "leave")
        .map(|d| d.employee_id)
        .collect();

    let mut absent: Vec<String> = Vec::new();
    if !weekend.contains(&weekday_idx) && !holiday {
        if let Ok(mut st) = conn.prepare("SELECT id,name FROM employees WHERE active=1 ORDER BY name") {
            if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) {
                for (id, name) in rows.flatten() {
                    if !with_in.contains(&id) && !leaves.contains(&id) {
                        absent.push(name);
                    }
                }
            }
        }
    }
    out.push_str(&format!("\n❌ لم يسجّلوا حضوراً ({}):\n", absent.len()));
    if weekend.contains(&weekday_idx) || holiday {
        out.push_str("— اليوم عطلة\n");
    } else if absent.is_empty() {
        out.push_str("— لا أحد\n");
    } else {
        for n in absent.iter().take(40) {
            out.push_str(&format!("• {n}\n"));
        }
    }
    Ok(out)
}

fn employee_name(conn: &Connection, id: i64) -> String {
    conn.query_row("SELECT name FROM employees WHERE id=?1", params![id], |r| r.get(0))
        .unwrap_or_else(|_| format!("#{id}"))
}

// ------------------------------------------------------------------ التنبيهات

/// يرسل رسالة لكل المشرفين المرتبطين (في خيط منفصل حتى لا يجمّد الواجهة).
pub fn notify_supervisors_async(app: &tauri::AppHandle, title: &str, body: &str) {
    let app = app.clone();
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let (token, chats) = {
            let Ok(conn) = state.db.lock() else { return };
            let s = settings(&conn);
            if !enabled(&s, "tg_enabled", false) {
                return;
            }
            let token = text_of(&s, "tg_token", "");
            if token.is_empty() {
                return;
            }
            let mut chats: Vec<String> = Vec::new();
            if let Ok(mut st) = conn.prepare(
                "SELECT chat_id FROM telegram_supervisors WHERE active=1 AND chat_id IS NOT NULL",
            ) {
                if let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) {
                    chats = rows.flatten().collect();
                }
            }
            (token, chats)
        };
        for c in chats {
            let _ = send(&token, &c, &format!("{title}\n{body}"), None);
        }
    });
}

/// نهاية اليوم: تنبيه الموظفين الذين لم يسجّلوا انصرافهم/حضورهم + ملخص للمشرفين.
fn eod_tick(app: &tauri::AppHandle, token: &str) {
    let state = app.state::<AppState>();
    let (on, target_min, already, today, now_min, employees, days, weekend, holiday) = {
        let Ok(conn) = state.db.lock() else { return };
        let s = settings(&conn);
        if !enabled(&s, "tg_enabled", false) {
            return;
        }
        if text_of(&s, "tg_token", "").is_empty() {
            return;
        }
        let today = today_in(&s);
        let already = s.get("tg_eod_sent").cloned().unwrap_or_default() == today;
        let now_min = now_minutes_in(&s);
        let mut employees: Vec<(i64, String, String)> = Vec::new();
        if let Ok(mut st) = conn.prepare(
            "SELECT id,name,telegram_chat_id,phone FROM employees
             WHERE active=1 AND bot_allowed=1 AND telegram_chat_id IS NOT NULL",
        ) {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            }) {
                for (id, name, chat, _phone) in rows.flatten() {
                    if !chat.is_empty() {
                        employees.push((id, name, chat));
                    }
                }
            }
        }
        let days = commands::month_days_impl(&conn, &today[..7]).unwrap_or_default();
        let weekday_idx = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .map(|d| {
                use chrono::Datelike;
                d.weekday().num_days_from_monday()
            })
            .unwrap_or(0);
        let weekend = crate::payroll::parse_weekend(s.get("weekend_days").map(|x| x.as_str()))
            .contains(&weekday_idx);
        let holiday = conn
            .query_row(
                "SELECT COUNT(*) FROM holidays WHERE date=?1",
                params![today],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        (
            enabled(&s, "tg_eod_enabled", true),
            hm_minutes(&text_of(&s, "tg_eod_time", "17:00")),
            already,
            today,
            now_min,
            employees,
            days,
            weekend,
            holiday,
        )
    };
    if !on || already || now_min < target_min {
        return;
    }

    for (id, name, chat) in &employees {
        let row = days.iter().find(|d| d.employee_id == *id && d.date == today);
        let msg = match row {
            Some(d) if d.first_in.is_some() && d.last_out.is_none() => Some(format!(
                "⏳ {name}\nسجّلت دخولك اليوم ولم تسجّل انصرافك بعد.\nلا تنسَ الختم عند المغادرة."
            )),
            Some(d) if d.first_in.is_none() && d.status != "leave" => Some(format!(
                "❌ {name}\nلم يُسجَّل حضورك اليوم ({today}).\nإن كان لديك عذر أرسله: /سبب <السبب>"
            )),
            None if !weekend && !holiday => Some(format!(
                "❌ {name}\nلم يُسجَّل حضورك اليوم ({today}).\nإن كان لديك عذر أرسله: /سبب <السبب>"
            )),
            _ => None,
        };
        if let Some(m) = msg {
            let _ = send(token, chat, &m, None);
        }
    }

    // ملخص للمشرفين ثم تسجيل أن تنبيه اليوم أُرسل
    let summary = {
        let Ok(conn) = state.db.lock() else { return };
        day_summary(&conn).ok()
    };
    if let Some(sum) = summary {
        notify_supervisors_async(app, "🌙 نهاية اليوم", &sum);
    }
    if let Ok(conn) = state.db.lock() {
        let _ = db::set_setting(&conn, "tg_eod_sent", &today);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_normalization() {
        assert_eq!(normalize_phone("0599 123 456"), "599123456");
        assert_eq!(normalize_phone("+970599123456"), "599123456");
        assert_eq!(normalize_phone("00970599123456"), "599123456");
        assert_eq!(normalize_phone("+972 59-912-3456"), "599123456");
    }

    #[test]
    fn phone_matching_ignores_country_prefix() {
        // 970 و972 لا يهمّان — نفس آخر 9 أرقام = نفس الشخص.
        assert!(phones_match("+970599123456", "+972599123456"));
        assert!(phones_match("+972599123456", "0599123456"));
        assert!(phones_match("+970599123456", "970599123456"));
        assert!(phones_match("00970599123456", "599123456"));
        // أرقام مختلفة أو قصيرة لا تتطابق.
        assert!(!phones_match("+970599123456", "0599123457"));
        assert!(!phones_match("059912", "059912"));
    }

    #[test]
    fn salary_shows_latest_paid_month_only() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,phone,active,created_at,updated_at)
             VALUES(4,'4','سالم','0599000444',1,'x','x')",
            [],
        )
        .unwrap();
        let s: HashMap<String, String> = HashMap::new();
        // بلا أي دفعة → رسالة واضحة (لا حساب حي)
        assert!(salary_text(&conn, 4, &s).is_err());

        conn.execute(
            "INSERT INTO salary_payments(employee_id,period,net,paid_at)
             VALUES(4,'2026-09',1250.25,'2026-10-01T10:00:00'),
                   (4,'2026-10',373.07,'2026-10-28T10:00:00')",
            [],
        )
        .unwrap();
        let t = salary_text(&conn, 4, &s).unwrap();
        assert!(t.contains("أكتوبر 2026"), "{t}");
        assert!(t.contains("373.07"), "{t}");
        assert!(t.contains("سالم"), "{t}");
        assert!(!t.contains("1250.25"), "{t}");
        assert!(!t.contains("مُعلَّم كمدفوع"), "{t}");
        assert!(!t.contains("معلم كمدفوع"), "{t}");

        // حذف دفعة أكتوبر → يعود تلقائياً لآخر دفعة أقدم
        conn.execute("DELETE FROM salary_payments WHERE period='2026-10'", []).unwrap();
        let t2 = salary_text(&conn, 4, &s).unwrap();
        assert!(t2.contains("سبتمبر 2026"), "{t2}");
        assert!(t2.contains("1250.25"), "{t2}");
    }

    #[test]
    fn broadcast_targets_only_active_linked() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO employees(id,pin,name,phone,active,bot_allowed,telegram_chat_id,created_at,updated_at)
             VALUES(1,'1','نشط','0599000001',1,1,'c1','x','x'),
                    (2,'2','موقوف','0599000002',0,1,'c2','x','x'),
                    (3,'3','غير مرتبط','0599000003',1,1,NULL,'x','x'),
                    (4,'4','ممنوع من البوت','0599000006',1,0,'c5','x','x');
             INSERT INTO telegram_supervisors(id,name,phone,chat_id,active,created_at)
             VALUES(1,'مشرف','0599000004','c3',1,'x'),
                    (2,'مشرف موقوف','0599000005','c4',0,'x');",
        )
        .unwrap();

        let all = broadcast_chats(&conn, "all").unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"c1".to_string()));
        assert!(all.contains(&"c3".to_string()));
        assert!(!all.contains(&"c2".to_string()));
        assert!(!all.contains(&"c4".to_string()));
        assert!(!all.contains(&"c5".to_string())); // ممنوع من البوت

        assert_eq!(broadcast_chats(&conn, "emp:1").unwrap(), vec!["c1"]);
        assert_eq!(broadcast_chats(&conn, "sup:1").unwrap(), vec!["c3"]);
        assert!(broadcast_chats(&conn, "emp:2").is_err()); // موقوف
        assert!(broadcast_chats(&conn, "emp:3").is_err()); // غير مرتبط
        assert!(broadcast_chats(&conn, "emp:4").is_err()); // ممنوع من البوت
        assert!(broadcast_chats(&conn, "sup:2").is_err()); // مشرف موقوف
        assert!(broadcast_chats(&conn, "bogus").is_err());
        // تحديث إلغاء ربط الجميع → خطأ واضح بدل إرسال صامت
        conn.execute("UPDATE employees SET telegram_chat_id=NULL", []).unwrap();
        conn.execute("UPDATE telegram_supervisors SET chat_id=NULL", []).unwrap();
        assert!(broadcast_chats(&conn, "all").is_err());
    }

    #[test]
    fn suspended_employee_is_detected() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,phone,active,telegram_chat_id,created_at,updated_at)
             VALUES(5,'5','موقوف','0599000111',0,'chat-5','x','x')",
            [],
        )
        .unwrap();
        // لا يظهر كمرتبط (لأنه موقوف) لكن يُكتشف كموقوف
        assert!(employee_by_chat(&conn, "chat-5").is_none());
        assert_eq!(suspended_employee(&conn, "chat-5").as_deref(), Some("موقوف"));
        // بعد إعادة التفعيل يعود مرتبطاً وينتهي المنع تلقائياً
        conn.execute("UPDATE employees SET active=1 WHERE id=5", []).unwrap();
        assert!(suspended_employee(&conn, "chat-5").is_none());
        assert!(employee_by_chat(&conn, "chat-5").is_some());
    }

    #[test]
    fn bot_blocked_employee_is_detected() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,phone,active,bot_allowed,telegram_chat_id,created_at,updated_at)
             VALUES(6,'6','ممنوع','0599000222',1,0,'chat-6','x','x'),
                    (7,'7','مسموح','0599000333',1,1,'chat-7','x','x')",
            [],
        )
        .unwrap();
        // الممنوع لا يُعرَّف كموظف مرتبط، لكن يُكتشف برسالة منع واضحة.
        assert!(employee_by_chat(&conn, "chat-6").is_none());
        assert_eq!(bot_blocked_employee(&conn, "chat-6").as_deref(), Some("ممنوع"));
        // المسموح يعمل طبيعياً
        assert!(employee_by_chat(&conn, "chat-7").is_some());
        assert!(bot_blocked_employee(&conn, "chat-7").is_none());
        // إعادة السماح تعيده فوراً
        conn.execute("UPDATE employees SET bot_allowed=1 WHERE id=6", []).unwrap();
        assert!(employee_by_chat(&conn, "chat-6").is_some());
        assert!(bot_blocked_employee(&conn, "chat-6").is_none());
    }

    #[test]
    fn employee_menu_follows_permissions() {
        let mut s: HashMap<String, String> = HashMap::new();
        // كل الصلاحيات مفعّلة افتراضياً
        let all = employee_menu(&s, false).to_string();
        assert!(all.contains(BTN_TODAY));
        assert!(all.contains(BTN_SHEET));
        assert!(all.contains(BTN_MONTH));
        assert!(all.contains(BTN_SALARY));
        assert!(all.contains(BTN_REASON));
        assert!(all.contains(BTN_HELP));

        // إطفاء الراتب والكشف يحذف زرّيهما فقط
        s.insert("tg_allow_salary".into(), "0".into());
        s.insert("tg_allow_sheet".into(), "0".into());
        let some = employee_menu(&s, false).to_string();
        assert!(!some.contains(BTN_SALARY));
        assert!(!some.contains(BTN_SHEET));
        assert!(some.contains(BTN_TODAY));
        assert!(some.contains(BTN_HELP));
        // الموظف-المشرف يرى زر ملخص اليوم إضافةً لأزرار الموظف
        let dual = employee_menu(&HashMap::new(), true).to_string();
        assert!(dual.contains(BTN_SUMMARY));
        assert!(dual.contains(BTN_TODAY));
        assert!(!all.contains(BTN_SUMMARY));
    }

    #[test]
    fn reason_keys_are_mapped() {
        assert_eq!(reason_from_key("family"), Some("ظرف عائلي"));
        assert_eq!(reason_from_key("sick"), Some("مرض"));
        assert_eq!(reason_from_key("hack"), None);
    }

    #[test]
    fn months_keyboard_has_six_valid_periods() {
        let mut s: HashMap<String, String> = HashMap::new();
        s.insert("timezone".into(), "Asia/Jerusalem".into());
        let kb = months_keyboard(&s);
        let rows = kb["inline_keyboard"].as_array().unwrap();
        let cells: Vec<&Value> = rows.iter().flat_map(|r| r.as_array().unwrap()).collect();
        assert_eq!(cells.len(), 6);
        for c in cells {
            let data = c["callback_data"].as_str().unwrap();
            assert!(data.starts_with("m:"));
            assert!(valid_period(&data[2..]));
            // الزر يعرض أرقاماً فقط: MM/YYYY (بلا أسماء أشهر عربية)
            let text = c["text"].as_str().unwrap();
            assert!(text.contains('/'), "label: {text}");
            assert!(text.chars().all(|ch| ch.is_ascii_digit() || ch == '/'), "label: {text}");
        }
    }

    #[test]
    fn period_validation() {
        assert!(valid_period("2026-09"));
        assert!(!valid_period("2026-9"));
        assert!(!valid_period("2026/09"));
        assert!(!valid_period("abc"));
    }
}
