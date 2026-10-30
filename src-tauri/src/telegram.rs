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
    AbsenceNote, AdvanceRequest, AttendanceDay, LeaveRequest, TelegramChatMessage, TelegramLink,
    TelegramSupervisor,
};
use tauri::Emitter;
use tauri_plugin_notification::NotificationExt;
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
const BTN_SALARY: &str = "💰 آخر راتب";
const BTN_LEAVE_BAL: &str = "🏖️ رصيد الإجازات";
const BTN_LEAVE_REQ: &str = "📝 طلب إجازة";
const BTN_ADV_REQ: &str = "💵 طلب سلفة";
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
    let mut r3: Vec<Value> = vec![json!({ "text": BTN_LEAVE_BAL })];
    if enabled(s, "tg_allow_leave", true) {
        r3.push(json!({ "text": BTN_LEAVE_REQ }));
    }
    if enabled(s, "tg_allow_advance", true) {
        r3.push(json!({ "text": BTN_ADV_REQ }));
    }
    rows.push(r3);
    let mut r4: Vec<Value> = Vec::new();
    if enabled(s, "tg_allow_reason", true) {
        r4.push(json!({ "text": BTN_REASON }));
    }
    if is_sup {
        r4.push(json!({ "text": BTN_SUMMARY }));
    }
    r4.push(json!({ "text": BTN_HELP }));
    rows.push(r4);
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

/// اسم نوع الإجازة بالعربية.
pub fn leave_type_label(ty: &str) -> &'static str {
    match ty {
        "annual" => "إجازة سنوية مدفوعة",
        "sick" => "إجازة مرضية",
        "unpaid" => "إجازة بدون راتب",
        "emergency" => "إجازة طارئة / ظرف عائلي",
        "marriage" => "إجازة زواج",
        "bereavement" => "إجازة وفاة (عزاء)",
        "comp" => "يوم بدل",
        _ => "إجازة",
    }
}

/// لوحة اختيار نوع الإجازة المطلوبة من البوت.
fn leave_types_keyboard() -> Value {
    json!({
        "inline_keyboard": [
            [{ "text": "🏖️ إجازة سنوية مدفوعة", "callback_data": "lv:type:annual" }],
            [{ "text": "🏥 إجازة مرضية (مع إرفاق تقرير/صورة)", "callback_data": "lv:type:sick" }],
            [{ "text": "📄 إجازة بدون راتب", "callback_data": "lv:type:unpaid" }],
            [{ "text": "🚨 إجازة طارئة / ظرف عائلي", "callback_data": "lv:type:emergency" }],
            [
                { "text": "💍 إجازة زواج", "callback_data": "lv:type:marriage" },
                { "text": "🕊️ إجازة عزاء", "callback_data": "lv:type:bereavement" }
            ],
            [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
        ]
    })
}

/// لوحة اختيار مدة الإجازة (يوم واحد أو من تاريخ إلى تاريخ).
fn leave_duration_keyboard(today: &str, tomorrow: &str) -> Value {
    json!({
        "inline_keyboard": [
            [{ "text": format!("📅 يوم واحد — اليوم ({today})"), "callback_data": format!("lv:one:{today}") }],
            [{ "text": format!("📅 يوم واحد — غداً ({tomorrow})"), "callback_data": format!("lv:one:{tomorrow}") }],
            [{ "text": "📅 يوم واحد — اختيار تاريخ آخر", "callback_data": "lv:mode:single" }],
            [{ "text": "🗓️ عدة أيام (من تاريخ إلى تاريخ)", "callback_data": "lv:mode:range" }],
            [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
        ]
    })
}

/// لوحة اختيار تاريخ من الأيام الـ 8 القادمة.
fn upcoming_dates_keyboard(today: &str, prefix: &str) -> Value {
    let base = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::NaiveDate::from_ymd_opt(2026, 10, 29).unwrap());
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut cur_row: Vec<Value> = Vec::new();
    for i in 0..8 {
        if let Some(d) = base.checked_add_signed(chrono::Duration::days(i)) {
            let ds = d.format("%Y-%m-%d").to_string();
            let wd = weekday_ar(&ds);
            cur_row.push(json!({
                "text": format!("{wd} {}", &ds[5..]),
                "callback_data": format!("{prefix}:{ds}")
            }));
            if cur_row.len() == 2 {
                rows.push(cur_row);
                cur_row = Vec::new();
            }
        }
    }
    if !cur_row.is_empty() {
        rows.push(cur_row);
    }
    rows.push(vec![json!({ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" })]);
    json!({ "inline_keyboard": rows })
}

/// لوحة اختيار نهاية الإجازة انطلاقاً من تاريخ البداية.
fn range_end_keyboard(from_date: &str) -> Value {
    let base = chrono::NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::NaiveDate::from_ymd_opt(2026, 10, 29).unwrap());
    let add = |n: i64| -> String {
        base.checked_add_signed(chrono::Duration::days(n))
            .unwrap_or(base)
            .format("%Y-%m-%d")
            .to_string()
    };
    let d1 = add(0);
    let d2 = add(1);
    let d3 = add(2);
    let d4 = add(3);
    let d5 = add(4);
    let d7 = add(6);
    json!({
        "inline_keyboard": [
            [
                { "text": format!("يوم واحد ({d1})"), "callback_data": format!("lv:to:{d1}") },
                { "text": format!("يومان (إلى {d2})"), "callback_data": format!("lv:to:{d2}") }
            ],
            [
                { "text": format!("3 أيام (إلى {d3})"), "callback_data": format!("lv:to:{d3}") },
                { "text": format!("4 أيام (إلى {d4})"), "callback_data": format!("lv:to:{d4}") }
            ],
            [
                { "text": format!("5 أيام (إلى {d5})"), "callback_data": format!("lv:to:{d5}") },
                { "text": format!("أسبوع — 7 أيام (إلى {d7})"), "callback_data": format!("lv:to:{d7}") }
            ],
            [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
        ]
    })
}

/// لوحة المبالغ الجاهزة لطلب سلفة.
fn advance_amounts_keyboard(cur: &str) -> Value {
    json!({
        "inline_keyboard": [
            [
                { "text": format!("100 {cur}"), "callback_data": "adv:amt:100" },
                { "text": format!("200 {cur}"), "callback_data": "adv:amt:200" },
                { "text": format!("300 {cur}"), "callback_data": "adv:amt:300" }
            ],
            [
                { "text": format!("500 {cur}"), "callback_data": "adv:amt:500" },
                { "text": format!("1000 {cur}"), "callback_data": "adv:amt:1000" },
                { "text": format!("1500 {cur}"), "callback_data": "adv:amt:1500" }
            ],
            [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
        ]
    })
}

fn log_chat_message(
    conn: &Connection,
    chat: &str,
    direction: &str,
    body: &str,
    attachment: Option<&str>,
) {
    if body.trim().is_empty() && attachment.is_none() {
        return;
    }
    let emp_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM employees WHERE telegram_chat_id=?1 LIMIT 1",
            params![chat],
            |r| r.get(0),
        )
        .ok();
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let _ = conn.execute(
        "INSERT INTO telegram_messages(chat_id, employee_id, direction, body, attachment_data, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![chat, emp_id, direction, body, attachment, now],
    );
}

fn get_flow(conn: &Connection, chat: &str) -> Option<Value> {
    let raw: String = conn
        .query_row(
            "SELECT state_json FROM telegram_flow_state WHERE chat_id=?1",
            params![chat],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&raw).ok()
}

fn set_flow(conn: &Connection, chat: &str, state: &Value) {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let s = state.to_string();
    let _ = conn.execute(
        "INSERT INTO telegram_flow_state(chat_id, state_json, updated_at)
         VALUES(?1, ?2, ?3)
         ON CONFLICT(chat_id) DO UPDATE SET state_json=excluded.state_json, updated_at=excluded.updated_at",
        params![chat, s, now],
    );
}

fn clear_flow(conn: &Connection, chat: &str) {
    let _ = conn.execute("DELETE FROM telegram_flow_state WHERE chat_id=?1", params![chat]);
}

/// تنزيل صورة مرسلة إلى البوت وتحويلها إلى Data URI (لإرفاق التقرير الطبي).
fn download_telegram_photo(token: &str, file_id: &str) -> Option<String> {
    let info = api(token, "getFile", json!({ "file_id": file_id }), 25).ok()?;
    let file_path = info["result"]["file_path"].as_str()?;
    let url = format!("https://api.telegram.org/file/bot{}/{file_path}", token.trim());
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;
    let bytes = client.get(&url).send().ok()?.bytes().ok()?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mime = if file_path.to_lowercase().ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    Some(format!("data:{mime};base64,{}", commands::base64_encode(&bytes)))
}

/// إرسال رسالة غير متزامن (يُستخدم مثلاً عند إلغاء إجازة تلقائياً ببصمة الحضور).
pub fn send_async(token: String, chat: String, _emp_id: Option<i64>, text: String) {
    std::thread::spawn(move || {
        let _ = send(&token, &chat, &text, None);
    });
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

// ---------------------------------------------------- طلبات الإجازات والسلف وسجل المحادثات

#[tauri::command]
pub fn leave_requests_list(state: tauri::State<'_, AppState>) -> Result<Vec<LeaveRequest>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut st = conn
        .prepare(
            "SELECT r.id, r.employee_id, COALESCE(e.name,'—'), COALESCE(e.annual_leave_balance,0),
                    r.leave_type, r.paid, r.from_date, r.to_date, r.days_count, r.note,
                    r.attachment_data, r.status, r.admin_note, r.created_at, r.decided_at
             FROM leave_requests r
             LEFT JOIN employees e ON e.id = r.employee_id
             ORDER BY CASE WHEN r.status='pending' THEN 0 ELSE 1 END, r.id DESC
             LIMIT 250",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            let att: Option<String> = r.get(10)?;
            Ok(LeaveRequest {
                id: r.get(0)?,
                employee_id: r.get(1)?,
                employee_name: r.get(2)?,
                annual_leave_balance: r.get(3)?,
                leave_type: r.get(4)?,
                paid: r.get::<_, i64>(5)? != 0,
                from_date: r.get(6)?,
                to_date: r.get(7)?,
                days_count: r.get(8)?,
                note: r.get(9)?,
                has_attachment: att.as_ref().map(|x| !x.is_empty()).unwrap_or(false),
                attachment_data: att,
                status: r.get(11)?,
                admin_note: r.get(12)?,
                created_at: r.get(13)?,
                decided_at: r.get(14)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn decide_leave_request(
    state: tauri::State<'_, AppState>,
    id: i64,
    approve: bool,
    paid: Option<bool>,
    admin_note: String,
) -> Result<(), String> {
    let admin_note = admin_note.trim().to_string();
    let (emp_id, from_date, to_date, days_count, token, chat_opt, tg_msg) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let (emp_id, leave_type, row_paid, from_date, to_date, days_count, req_note, status): (
            i64,
            String,
            bool,
            String,
            String,
            i64,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT employee_id, leave_type, paid, from_date, to_date, days_count, note, status
                 FROM leave_requests WHERE id=?1",
                params![id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get::<_, i64>(2)? != 0,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )
            .map_err(|_| "طلب الإجازة غير موجود".to_string())?;

        if status != "pending" {
            return Err("تم البتّ في هذا الطلب مسبقاً".into());
        }

        let final_paid = paid.unwrap_or(row_paid);
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let type_lbl = leave_type_label(&leave_type);

        if approve {
            let full_note = match (req_note.trim().is_empty(), admin_note.is_empty()) {
                (true, true) => "طلب عبر بوت تلجرام".to_string(),
                (false, true) => req_note.clone(),
                (true, false) => admin_note.clone(),
                (false, false) => format!("{} — {}", req_note.trim(), admin_note),
            };
            commands::add_leave_impl(
                &conn,
                emp_id,
                &from_date,
                &to_date,
                &leave_type,
                final_paid,
                &full_note,
            )?;
            conn.execute(
                "UPDATE leave_requests SET status='approved', paid=?1, admin_note=?2, decided_at=?3 WHERE id=?4",
                params![final_paid as i64, admin_note, now, id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "UPDATE leave_requests SET status='rejected', admin_note=?1, decided_at=?2 WHERE id=?3",
                params![admin_note, now, id],
            )
            .map_err(|e| e.to_string())?;
        }

        let st = settings(&conn);
        let token = if enabled(&st, "tg_enabled", false) {
            text_of(&st, "tg_token", "")
        } else {
            String::new()
        };
        let chat_opt: Option<String> = conn
            .query_row(
                "SELECT telegram_chat_id FROM employees WHERE id=?1",
                params![emp_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        let bal: f64 = conn
            .query_row(
                "SELECT COALESCE(annual_leave_balance,0) FROM employees WHERE id=?1",
                params![emp_id],
                |r| r.get(0),
            )
            .unwrap_or(0.0);

        let note_line = if admin_note.is_empty() {
            String::new()
        } else {
            format!("\n• ملاحظة الإدارة: {admin_note}")
        };
        let bal_line = if approve && leave_type == "annual" && final_paid {
            format!("\n• رصيدك السنوي المتبقي: {bal:.1} يوم")
        } else {
            String::new()
        };
        let tg_msg = if approve {
            format!(
                "✅ تمت الموافقة على طلب إجازتك\n• النوع: {type_lbl} ({})\n• الفترة: {from_date} ← {to_date} ({days_count} يوم){bal_line}{note_line}\n\n💡 ملاحظة: في حال ختمت بصمة دخول في يوم الإجازة، تُلغى الإجازة عن ذلك اليوم تلقائياً ويُعاد اليوم لرصيدك إن كانت سنوية مدفوعة.",
                if final_paid { "مدفوعة" } else { "بدون راتب" }
            )
        } else {
            format!(
                "❌ تم رفض طلب إجازتك ({type_lbl})\n• الفترة: {from_date} ← {to_date} ({days_count} يوم){note_line}"
            )
        };
        if let Some(c) = chat_opt.as_deref().filter(|c| !c.trim().is_empty()) {
            log_chat_message(&conn, c, "out", &tg_msg, None);
        }
        (emp_id, from_date, to_date, days_count, token, chat_opt, tg_msg)
    };

    let _ = (emp_id, days_count);
    if approve {
        if let (Ok(a), Ok(b)) = (
            chrono::NaiveDate::parse_from_str(&from_date, "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(&to_date, "%Y-%m-%d"),
        ) {
            let _ = commands::recompute_range(&state, &a, &b);
        }
    }

    if !token.trim().is_empty() {
        if let Some(c) = chat_opt.filter(|c| !c.trim().is_empty()) {
            let _ = send(&token, &c, &tg_msg, None);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn advance_requests_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AdvanceRequest>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let mut st = conn
        .prepare(
            "SELECT r.id, r.employee_id, COALESCE(e.name,'—'), r.amount, r.note,
                    r.status, r.advance_id, r.admin_note, r.created_at, r.decided_at
             FROM advance_requests r
             LEFT JOIN employees e ON e.id = r.employee_id
             ORDER BY CASE WHEN r.status='pending' THEN 0 ELSE 1 END, r.id DESC
             LIMIT 250",
        )
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok(AdvanceRequest {
                id: r.get(0)?,
                employee_id: r.get(1)?,
                employee_name: r.get(2)?,
                amount: r.get(3)?,
                note: r.get(4)?,
                status: r.get(5)?,
                advance_id: r.get(6)?,
                admin_note: r.get(7)?,
                created_at: r.get(8)?,
                decided_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn decide_advance_request(
    state: tauri::State<'_, AppState>,
    id: i64,
    approve: bool,
    amount: Option<f64>,
    mode: Option<String>,
    installments_count: Option<i64>,
    start_period: Option<String>,
    admin_note: String,
) -> Result<(), String> {
    let admin_note = admin_note.trim().to_string();
    let (token, chat_opt, tg_msg) = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let (emp_id, req_amt, req_note, status): (i64, f64, String, String) = conn
            .query_row(
                "SELECT employee_id, amount, note, status FROM advance_requests WHERE id=?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|_| "طلب السلفة غير موجود".to_string())?;

        if status != "pending" {
            return Err("تم البتّ في هذا الطلب مسبقاً".into());
        }

        let st = settings(&conn);
        let cur = currency(&st);
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let def_period = today_in(&st)[..7].to_string();

        let tg_msg = if approve {
            let final_amt = amount.unwrap_or(req_amt);
            let final_mode = mode.unwrap_or_else(|| "single".into());
            let final_count = if final_mode == "installments" {
                installments_count.unwrap_or(2).clamp(1, 120)
            } else {
                1
            };
            let final_start = start_period
                .filter(|p| !p.trim().is_empty())
                .unwrap_or(def_period);
            let full_note = match (req_note.trim().is_empty(), admin_note.is_empty()) {
                (true, true) => "طلب سلفة عبر بوت تلجرام".to_string(),
                (false, true) => req_note.clone(),
                (true, false) => admin_note.clone(),
                (false, false) => format!("{} — {}", req_note.trim(), admin_note),
            };
            let adv = commands::add_employee_advance_impl(
                &conn,
                emp_id,
                final_amt,
                &final_mode,
                final_count,
                &final_start,
                &full_note,
            )?;
            conn.execute(
                "UPDATE advance_requests
                 SET status='approved', amount=?1, advance_id=?2, admin_note=?3, decided_at=?4
                 WHERE id=?5",
                params![final_amt, adv.id, admin_note, now, id],
            )
            .map_err(|e| e.to_string())?;

            let plan = if final_mode == "installments" && final_count > 1 {
                let per = final_amt / final_count as f64;
                format!(
                    "تقسيط على {final_count} أشهر ابتداءً من {final_start} ({per:.2} {cur} شهرياً)"
                )
            } else {
                format!("دفعة واحدة على راتب شهر {final_start}")
            };
            let note_line = if admin_note.is_empty() {
                String::new()
            } else {
                format!("\n• ملاحظة الإدارة: {admin_note}")
            };
            format!(
                "✅ تمت الموافقة على طلب السلفة\n• المبلغ المعتمد: {final_amt:.2} {cur}\n• طريقة الخصم: {plan}{note_line}"
            )
        } else {
            conn.execute(
                "UPDATE advance_requests SET status='rejected', admin_note=?1, decided_at=?2 WHERE id=?3",
                params![admin_note, now, id],
            )
            .map_err(|e| e.to_string())?;
            let note_line = if admin_note.is_empty() {
                String::new()
            } else {
                format!("\n• السبب/الملاحظة: {admin_note}")
            };
            format!("❌ تم رفض طلب السلفة بمبلغ {req_amt:.2} {cur}.{note_line}")
        };

        let token = if enabled(&st, "tg_enabled", false) {
            text_of(&st, "tg_token", "")
        } else {
            String::new()
        };
        let chat_opt: Option<String> = conn
            .query_row(
                "SELECT telegram_chat_id FROM employees WHERE id=?1",
                params![emp_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        if let Some(c) = chat_opt.as_deref().filter(|c| !c.trim().is_empty()) {
            log_chat_message(&conn, c, "out", &tg_msg, None);
        }
        (token, chat_opt, tg_msg)
    };

    if !token.trim().is_empty() {
        if let Some(c) = chat_opt.filter(|c| !c.trim().is_empty()) {
            let _ = send(&token, &c, &tg_msg, None);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn telegram_chat_messages(
    state: tauri::State<'_, AppState>,
    employee_id: Option<i64>,
    search: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<TelegramChatMessage>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let lim = limit.unwrap_or(300).clamp(1, 1000);
    let q = search.unwrap_or_default().trim().to_lowercase();
    let mut st = conn
        .prepare(
            "SELECT m.id, m.chat_id, COALESCE(m.employee_id, e2.id),
                    COALESCE(e.name, e2.name, s.name, 'محادثة ' || m.chat_id),
                    COALESCE(e.phone, e2.phone, s.phone, ''),
                    m.direction, m.body, m.attachment_data, m.created_at
             FROM telegram_messages m
             LEFT JOIN employees e ON e.id = m.employee_id
             LEFT JOIN employees e2 ON e2.telegram_chat_id = m.chat_id
             LEFT JOIN telegram_supervisors s ON s.chat_id = m.chat_id
             WHERE (?1 IS NULL OR COALESCE(m.employee_id, e2.id) = ?1)
             ORDER BY m.id DESC
             LIMIT ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<TelegramChatMessage> = st
        .query_map(params![employee_id, lim], |r| {
            Ok(TelegramChatMessage {
                id: r.get(0)?,
                chat_id: r.get(1)?,
                employee_id: r.get(2)?,
                employee_name: r.get(3)?,
                employee_phone: r.get(4)?,
                direction: r.get(5)?,
                body: r.get(6)?,
                attachment_data: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|m| {
            if q.is_empty() {
                true
            } else {
                m.body.to_lowercase().contains(&q)
                    || m.employee_name.to_lowercase().contains(&q)
                    || m.employee_phone.to_lowercase().contains(&q)
            }
        })
        .collect();
    Ok(rows)
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

    // هل أرسل الموظف صورة (تقرير طبي لإجازة مرضية مثلاً)؟
    if let Some(photos) = msg.get("photo").and_then(|x| x.as_array()) {
        if let Some(last) = photos.last() {
            if let Some(file_id) = last["file_id"].as_str() {
                let caption = msg["caption"].as_str().unwrap_or("").trim().to_string();
                handle_photo(app, token, &chat_id, file_id, &caption);
                return;
            }
        }
    }
    if let Some(doc) = msg.get("document") {
        let mime = doc["mime_type"].as_str().unwrap_or("");
        if mime.starts_with("image/") {
            if let Some(file_id) = doc["file_id"].as_str() {
                let caption = msg["caption"].as_str().unwrap_or("").trim().to_string();
                handle_photo(app, token, &chat_id, file_id, &caption);
                return;
            }
        }
    }

    let text = msg["text"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() {
        return;
    }
    handle_text(app, token, &chat_id, &text);
}

/// يحفظ طلب الإجازة في قاعدة البيانات ويعيد رسالة التأكيد + عنوان/نص التنبيه.
fn finalize_leave_request(
    conn: &Connection,
    emp_id: i64,
    emp_name: &str,
    leave_type: &str,
    from_date: &str,
    to_date: &str,
    note: &str,
    attachment_data: Option<&str>,
) -> Result<(String, String, String), String> {
    let days = commands::range_days(from_date, to_date);
    if days <= 0 {
        return Err("تاريخ الانتهاء يجب أن يكون مساوياً لتاريخ البداية أو بعده".into());
    }
    let paid = leave_type != "unpaid";
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO leave_requests(
            employee_id, leave_type, paid, from_date, to_date, days_count,
            note, attachment_data, status, admin_note, created_at
         ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', '', ?9)",
        params![
            emp_id,
            leave_type,
            paid as i64,
            from_date,
            to_date,
            days,
            note.trim(),
            attachment_data,
            now
        ],
    )
    .map_err(|e| format!("تعذّر حفظ طلب الإجازة: {e}"))?;

    let lbl = leave_type_label(leave_type);
    let att_line = if attachment_data.is_some() {
        "\n• المرفق: تم إرفاق الصورة 📎"
    } else {
        ""
    };
    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("\n• الملاحظة: {}", note.trim())
    };
    let reply = format!(
        "✅ تم إرسال طلب الإجازة إلى الإدارة:\n• النوع: {lbl}\n• الفترة: {from_date} ← {to_date} ({days} يوم){att_line}{note_line}\n\nسيصلك إشعار على البوت فور القبول أو الرفض."
    );
    let alert_title = format!("🏖️ طلب إجازة جديد — {emp_name}");
    let alert_body = format!(
        "{lbl} من {from_date} إلى {to_date} ({days} يوم){}{}",
        if attachment_data.is_some() { " [مع مرفق]" } else { "" },
        if note.trim().is_empty() {
            String::new()
        } else {
            format!(" — {}", note.trim())
        }
    );
    Ok((reply, alert_title, alert_body))
}

/// يحفظ طلب السلفة في قاعدة البيانات ويعيد رسالة التأكيد + عنوان/نص التنبيه.
fn finalize_advance_request(
    conn: &Connection,
    emp_id: i64,
    emp_name: &str,
    amount: f64,
    note: &str,
    cur: &str,
) -> Result<(String, String, String), String> {
    if amount <= 0.0 {
        return Err("مبلغ السلفة يجب أن يكون أكبر من صفر".into());
    }
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO advance_requests(employee_id, amount, note, status, admin_note, created_at)
         VALUES(?1, ?2, ?3, 'pending', '', ?4)",
        params![emp_id, amount, note.trim(), now],
    )
    .map_err(|e| format!("تعذّر حفظ طلب السلفة: {e}"))?;

    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("\n• الملاحظة: {}", note.trim())
    };
    let reply = format!(
        "✅ تم إرسال طلب السلفة إلى الإدارة:\n• المبلغ المطلوب: {amount:.2} {cur}{note_line}\n\nسيصلك إشعار على البوت فور القبول أو الرفض."
    );
    let alert_title = format!("💵 طلب سلفة جديد — {emp_name}");
    let alert_body = format!(
        "المبلغ: {amount:.2} {cur}{}",
        if note.trim().is_empty() {
            String::new()
        } else {
            format!(" — {}", note.trim())
        }
    );
    Ok((reply, alert_title, alert_body))
}

/// ملخص رصيد الإجازات السنوية للموظف.
fn leave_balance_text(conn: &Connection, emp_id: i64) -> String {
    let (name, balance, annual_days): (String, f64, f64) = conn
        .query_row(
            "SELECT name, COALESCE(annual_leave_balance,0), COALESCE(annual_leave_days,14)
             FROM employees WHERE id=?1",
            params![emp_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or_else(|_| ("—".into(), 0.0, 14.0));

    let mut used_annual = 0i64;
    let mut used_sick = 0i64;
    let mut used_unpaid = 0i64;
    if let Ok(mut st) = conn.prepare(
        "SELECT from_date, to_date, leave_type, paid FROM leaves WHERE employee_id=?1",
    ) {
        if let Ok(rows) = st.query_map(params![emp_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3).unwrap_or(1) != 0,
            ))
        }) {
            for (f, t, ty, paid) in rows.flatten() {
                let d = commands::range_days(&f, &t).max(0);
                if ty == "annual" && paid {
                    used_annual += d;
                } else if ty == "sick" {
                    used_sick += d;
                } else if !paid {
                    used_unpaid += d;
                }
            }
        }
    }
    let pending_cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM leave_requests WHERE employee_id=?1 AND status='pending'",
            params![emp_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let mut out = format!(
        "🏖️ رصيد الإجازات — {}\n\n• الرصيد السنوي المتاح: {balance:.1} يوم\n• الاستحقاق السنوي: {annual_days:.1} يوم / سنة\n• إجازات سنوية مستخدمة: {used_annual} يوم\n• إجازات مرضية مسجّلة: {used_sick} يوم\n• إجازات بدون راتب: {used_unpaid} يوم",
        name.trim()
    );
    if pending_cnt > 0 {
        out.push_str(&format!("\n• طلبات قيد المراجعة: {pending_cnt}"));
    }
    out
}

/// يعالج إرسال صورة إلى البوت (إرفاق تقرير طبي لطلب إجازة مرضية).
fn handle_photo(app: &tauri::AppHandle, token: &str, chat: &str, file_id: &str, caption: &str) {
    let att_opt = download_telegram_photo(token, file_id);
    let state = app.state::<AppState>();
    let Ok(conn) = state.db.lock() else { return };
    let s = settings(&conn);
    let log_body = if caption.is_empty() {
        "[صورة مرفقة]".to_string()
    } else {
        format!("[صورة مرفقة] {caption}")
    };
    log_chat_message(&conn, chat, "in", &log_body, att_opt.as_deref());

    let Some((emp_id, name, _)) = employee_by_chat(&conn, chat) else {
        drop(conn);
        let _ = send(
            token,
            chat,
            "⚠️ يجب ربط حسابك كموظف أولاً قبل إرسال المرفقات.",
            None,
        );
        return;
    };
    let is_sup = supervisor_by_chat(&conn, chat).is_some();
    let Some(att_data) = att_opt else {
        drop(conn);
        let _ = send(
            token,
            chat,
            "⚠️ تعذّر تنزيل الصورة المرفقة — حاول إرسالها مرة أخرى.",
            Some(employee_menu(&s, is_sup)),
        );
        return;
    };

    let flow = get_flow(&conn, chat);
    let today = today_in(&s);
    let (ty, from_date, to_date, note) = if let Some(fl) = &flow {
        if fl["kind"].as_str() == Some("leave") {
            let ty = fl["leave_type"].as_str().unwrap_or("sick").to_string();
            let fd = fl["from_date"].as_str().unwrap_or(&today).to_string();
            let td = fl["to_date"].as_str().unwrap_or(&fd).to_string();
            let prev_note = fl["note"].as_str().unwrap_or("");
            let note = if !caption.is_empty() {
                caption.to_string()
            } else {
                prev_note.to_string()
            };
            (ty, fd, td, note)
        } else {
            ("sick".to_string(), today.clone(), today.clone(), caption.to_string())
        }
    } else {
        ("sick".to_string(), today.clone(), today.clone(), caption.to_string())
    };

    let res = finalize_leave_request(
        &conn,
        emp_id,
        &name,
        &ty,
        &from_date,
        &to_date,
        &note,
        Some(&att_data),
    );
    clear_flow(&conn, chat);
    let kb = employee_menu(&s, is_sup);
    match res {
        Ok((reply_msg, alert_title, alert_body)) => {
            log_chat_message(&conn, chat, "out", &reply_msg, None);
            drop(conn);
            let _ = send(token, chat, &reply_msg, Some(kb));
            let _ = app
                .notification()
                .builder()
                .title(&alert_title)
                .body(&alert_body)
                .show();
            let _ = app.emit("bot-request", json!({ "kind": "leave", "employee_id": emp_id }));
            notify_supervisors_async(app, &alert_title, &alert_body);
        }
        Err(e) => {
            let err_msg = format!("⚠️ {e}");
            log_chat_message(&conn, chat, "out", &err_msg, None);
            drop(conn);
            let _ = send(token, chat, &err_msg, Some(kb));
        }
    }
}

/// نص مقروء لضغطات الأزرار في سجل المحادثة — بدل callback_data الخام
/// (مثل adv:amt:500) الذي لا يفهمه أحد في السجل.
fn callback_label(data: &str) -> String {
    if let Some(amt) = data.strip_prefix("adv:amt:") {
        return format!("💰 مبلغ السلفة: {amt}");
    }
    if let Some(ty) = data.strip_prefix("lv:type:") {
        return format!("📝 نوع الإجازة: {}", leave_type_label(ty));
    }
    if let Some(d) = data
        .strip_prefix("lv:one:")
        .or_else(|| data.strip_prefix("lv:from:"))
        .or_else(|| data.strip_prefix("lv:to:"))
    {
        return format!("📅 اختيار تاريخ: {d}");
    }
    if let Some(p) = data.strip_prefix("m:") {
        return format!("🗓️ اختيار شهر {p}");
    }
    if let Some(k) = data.strip_prefix("r:") {
        return match reason_from_key(k) {
            Some(label) => format!("✍️ سبب الغياب: {label}"),
            None => data.to_string(),
        };
    }
    match data {
        "adv:submit" => "✅ إرسال طلب السلفة".into(),
        "lv:submit" => "✅ إرسال طلب الإجازة".into(),
        "lv:mode:single" => "📅 إجازة يوم واحد".into(),
        "lv:mode:range" => "🗓️ إجازة من تاريخ إلى تاريخ".into(),
        "flow:cancel" => "✖️ إلغاء الطلب".into(),
        _ => data.to_string(),
    }
}

/// تُصلح الصفوف القديمة من سجل المحادثة التي حُفظت بـ callback_data الخام
/// (مثل «[زر] adv:amt:500») إلى نص مقروء — تُستدعى مرة عند الإقلاع فتصلح ما
/// سجّله إصدار سابق دون أي تدخل يدوي. الصفوف غير المعروفة تبقى كما هي.
pub fn rewrite_legacy_chat_log(conn: &Connection) -> usize {
    const PFX: &str = "[زر] ";
    let rows: Vec<(i64, String)> = match conn.prepare(
        "SELECT id, body FROM telegram_messages WHERE body LIKE '[زر] %'",
    ) {
        Ok(mut st) => st
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))
            .map(|it| it.flatten().collect())
            .unwrap_or_default(),
        Err(_) => return 0,
    };
    let mut n = 0;
    for (id, body) in rows {
        let Some(data) = body.strip_prefix(PFX) else { continue };
        let label = callback_label(data);
        if label == data {
            continue; // رمز غير معروف — يبقى كما هو
        }
        if let Ok(mut st) = conn.prepare("UPDATE telegram_messages SET body=?1 WHERE id=?2") {
            if st
                .execute(rusqlite::params![label.to_string(), id])
                .is_ok()
            {
                n += 1;
            }
        }
    }
    n
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

    log_chat_message(&conn, &chat, "in", &callback_label(&data), None);

    let mut reply: Option<String> = None;
    let mut keyboard: Option<Value> = None;
    let mut absence_alert: Option<String> = None;
    let mut request_alert: Option<(String, String, &'static str)> = None;

    if let Some(name) = &suspended {
        reply = Some(format!("{SUSPENDED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if let Some(name) = &blocked {
        reply = Some(format!("{BOT_BLOCKED}\n\nالحساب: {name}"));
        keyboard = Some(remove_keyboard());
    } else if let Some((emp_id, name, _)) = emp {
        let is_sup = sup.is_some();
        let today = today_in(&s);
        let tomorrow = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.succ_opt())
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| today.clone());

        if data == "flow:cancel" {
            clear_flow(&conn, &chat);
            reply = Some("✖️ تم إلغاء الطلب.".into());
            keyboard = Some(employee_menu(&s, is_sup));
        } else if data.starts_with("lv:") && !enabled(&s, "tg_allow_leave", true) {
            // طلبات الإجازات مُعطّلة من قسم «ما المسموح للموظف؟»
            reply = Some(PERM_DENIED.into());
        } else if data.starts_with("adv:") && !enabled(&s, "tg_allow_advance", true) {
            // طلبات السلف مُعطّلة من قسم «ما المسموح للموظف؟»
            reply = Some(PERM_DENIED.into());
        } else if let Some(ty) = data.strip_prefix("lv:type:") {
            set_flow(
                &conn,
                &chat,
                &json!({ "kind": "leave", "leave_type": ty, "step": "duration" }),
            );
            reply = Some(format!(
                "📌 النوع المختار: {}\n\nحدد مدة الإجازة:",
                leave_type_label(ty)
            ));
            keyboard = Some(leave_duration_keyboard(&today, &tomorrow));
        } else if data == "lv:mode:single" {
            if let Some(mut fl) = get_flow(&conn, &chat) {
                fl["step"] = json!("single_date");
                set_flow(&conn, &chat, &fl);
                reply = Some(
                    "📅 اختر تاريخ يوم الإجازة من الأزرار أدناه، أو اكتب التاريخ بصيغة YYYY-MM-DD (مثال: 2026-11-05):".into(),
                );
                keyboard = Some(upcoming_dates_keyboard(&today, "lv:one"));
            }
        } else if data == "lv:mode:range" {
            if let Some(mut fl) = get_flow(&conn, &chat) {
                fl["step"] = json!("range_from");
                set_flow(&conn, &chat, &fl);
                reply = Some(
                    "🗓️ اختر تاريخ بداية الإجازة (من) من الأزرار، أو اكتبه بصيغة YYYY-MM-DD:".into(),
                );
                keyboard = Some(upcoming_dates_keyboard(&today, "lv:from"));
            }
        } else if let Some(d) = data.strip_prefix("lv:from:") {
            if let Some(mut fl) = get_flow(&conn, &chat) {
                fl["step"] = json!("range_to");
                fl["from_date"] = json!(d);
                set_flow(&conn, &chat, &fl);
                reply = Some(format!(
                    "🗓️ بداية الإجازة: {d}\n\nاختر مدة/نهاية الإجازة من الأزرار، أو اكتب تاريخ النهاية بصيغة YYYY-MM-DD:"
                ));
                keyboard = Some(range_end_keyboard(d));
            }
        } else if let Some(d) = data.strip_prefix("lv:one:").or_else(|| data.strip_prefix("lv:to:")) {
            let is_one = data.starts_with("lv:one:");
            if let Some(mut fl) = get_flow(&conn, &chat) {
                let ty = fl["leave_type"].as_str().unwrap_or("annual").to_string();
                let from_date = if is_one {
                    d.to_string()
                } else {
                    fl["from_date"].as_str().unwrap_or(d).to_string()
                };
                let to_date = d.to_string();
                let days = commands::range_days(&from_date, &to_date);
                if days <= 0 {
                    reply = Some("⚠️ تاريخ الانتهاء يجب أن يكون بعد تاريخ البداية أو مساوياً له.".into());
                    keyboard = Some(range_end_keyboard(&from_date));
                } else {
                    fl["step"] = json!("details");
                    fl["from_date"] = json!(from_date.clone());
                    fl["to_date"] = json!(to_date.clone());
                    fl["note"] = json!("");
                    set_flow(&conn, &chat, &fl);
                    if ty == "sick" {
                        reply = Some(format!(
                            "🏥 طلب إجازة مرضية\nمن {from_date} إلى {to_date} ({days} يوم)\n\n📸 أرسل الآن صورة التقرير الطبي (أو اكتب ملاحظة)، أو اضغط الزر أدناه لإرسال الطلب بدون صورة:"
                        ));
                        keyboard = Some(json!({
                            "inline_keyboard": [
                                [{ "text": "✅ إرسال الطلب بدون صورة", "callback_data": "lv:submit" }],
                                [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                            ]
                        }));
                    } else {
                        reply = Some(format!(
                            "📝 طلب {}\nمن {from_date} إلى {to_date} ({days} يوم)\n\n✍️ اكتب سبب/ملاحظة الإجازة الآن، أو اضغط «✅ إرسال الطلب الآن»:",
                            leave_type_label(&ty)
                        ));
                        keyboard = Some(json!({
                            "inline_keyboard": [
                                [{ "text": "✅ إرسال الطلب الآن", "callback_data": "lv:submit" }],
                                [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                            ]
                        }));
                    }
                }
            }
        } else if data == "lv:submit" {
            keyboard = Some(employee_menu(&s, is_sup));
            if let Some(fl) = get_flow(&conn, &chat) {
                let ty = fl["leave_type"].as_str().unwrap_or("annual").to_string();
                let fd = fl["from_date"].as_str().unwrap_or(&today).to_string();
                let td = fl["to_date"].as_str().unwrap_or(&fd).to_string();
                let note = fl["note"].as_str().unwrap_or("").to_string();
                clear_flow(&conn, &chat);
                match finalize_leave_request(&conn, emp_id, &name, &ty, &fd, &td, &note, None) {
                    Ok((r, t, b)) => {
                        reply = Some(r);
                        request_alert = Some((t, b, "leave"));
                    }
                    Err(e) => reply = Some(format!("⚠️ {e}")),
                }
            } else {
                reply = Some("⚠️ لا يوجد طلب إجازة قيد الإعداد حالياً.".into());
            }
        } else if let Some(amt_s) = data.strip_prefix("adv:amt:") {
            if let Ok(amt) = amt_s.parse::<f64>() {
                set_flow(
                    &conn,
                    &chat,
                    &json!({ "kind": "advance", "step": "note", "amount": amt }),
                );
                let cur = currency(&s);
                reply = Some(format!(
                    "💵 مبلغ السلفة المطلوب: {amt:.2} {cur}\n\n✍️ اكتب ملاحظة أو سبب طلب السلفة، أو اضغط «✅ إرسال الطلب الآن»:"
                ));
                keyboard = Some(json!({
                    "inline_keyboard": [
                        [{ "text": "✅ إرسال الطلب الآن", "callback_data": "adv:submit" }],
                        [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                    ]
                }));
            }
        } else if data == "adv:submit" {
            keyboard = Some(employee_menu(&s, is_sup));
            if let Some(fl) = get_flow(&conn, &chat) {
                let amt = fl["amount"].as_f64().unwrap_or(0.0);
                let note = fl["note"].as_str().unwrap_or("").to_string();
                let cur = currency(&s);
                clear_flow(&conn, &chat);
                match finalize_advance_request(&conn, emp_id, &name, amt, &note, &cur) {
                    Ok((r, t, b)) => {
                        reply = Some(r);
                        request_alert = Some((t, b, "advance"));
                    }
                    Err(e) => reply = Some(format!("⚠️ {e}")),
                }
            } else {
                reply = Some("⚠️ لا يوجد طلب سلفة قيد الإعداد حالياً.".into());
            }
        } else if let Some(period) = data.strip_prefix("m:") {
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

    if let Some(m) = &reply {
        log_chat_message(&conn, &chat, "out", m, None);
    }
    drop(conn);

    if let Some(m) = reply {
        let _ = send(token, &chat, &m, keyboard);
    }
    if let Some(body) = absence_alert {
        notify_supervisors_async(app, "📝 سبب غياب", &body);
    }
    if let Some((title, body, kind)) = request_alert {
        let _ = app.notification().builder().title(&title).body(&body).show();
        let _ = app.emit("bot-request", json!({ "kind": kind }));
        notify_supervisors_async(app, &title, &body);
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
    let is_menu_btn = matches!(
        raw_text,
        BTN_TODAY
            | BTN_SHEET
            | BTN_SALARY
            | "💰 آخر مدفوع"
            | "💰 صافي الراتب"
            | BTN_LEAVE_BAL
            | BTN_LEAVE_REQ
            | BTN_ADV_REQ
            | BTN_MONTH
            | BTN_REASON
            | BTN_HELP
            | BTN_SUMMARY
    ) || raw_text.starts_with('/');

    let mapped = match raw_text {
        BTN_TODAY => "اليوم",
        BTN_SHEET => "كشف",
        BTN_SALARY | "💰 آخر مدفوع" | "💰 صافي الراتب" | "آخر راتب" | "اخر راتب" | "آخر مدفوع" | "اخر مدفوع" => "راتب",
        BTN_LEAVE_BAL | "رصيد الاجازات" | "رصيد الإجازات" => "رصيد",
        BTN_LEAVE_REQ | "طلب اجازة" | "طلب إجازة" => "اجازة",
        BTN_ADV_REQ | "طلب سلفة" | "طلب سلفه" => "سلفة",
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

    log_chat_message(&conn, chat, "in", raw_text, None);

    // الرد يُحسب والقفل ممسوك (قراءة قاعدة البيانات فقط)، ثم يُرسل بعد فكّه
    // حتى لا تتجمّد الواجهة أثناء انتظار شبكة تلجرام.
    let mut reply: Option<String> = None;
    let mut keyboard: Option<Value> = None;
    let mut absence_alert: Option<String> = None;
    let mut request_alert: Option<(String, String, &'static str)> = None;

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
        if is_menu_btn {
            clear_flow(&conn, chat);
        }
        // إن كان الموظف في وسط خطوة تفاعلية (طلب إجازة أو طلب سلفة) وأرسل نصاً عادياً:
        let active_flow = if !is_menu_btn { get_flow(&conn, chat) } else { None };
        if let Some(mut fl) = active_flow {
            let kind = fl["kind"].as_str().unwrap_or("").to_string();
            let step = fl["step"].as_str().unwrap_or("").to_string();
            let today = today_in(&s);
            if kind == "leave" && (step == "single_date" || step == "range_from" || step == "range_to") {
                let d_trim = raw_text.trim();
                if chrono::NaiveDate::parse_from_str(d_trim, "%Y-%m-%d").is_err() {
                    reply = Some("⚠️ صيغة التاريخ غير صحيحة — اكتبه بالصيغة YYYY-MM-DD (مثال: 2026-11-05) أو اختر من الأزرار.".into());
                } else if step == "range_from" {
                    fl["step"] = json!("range_to");
                    fl["from_date"] = json!(d_trim);
                    set_flow(&conn, chat, &fl);
                    reply = Some(format!(
                        "🗓️ بداية الإجازة: {d_trim}\n\nاختر تاريخ نهاية الإجازة من الأزرار، أو اكتبه بالصيغة YYYY-MM-DD:"
                    ));
                    keyboard = Some(range_end_keyboard(d_trim));
                } else {
                    let ty = fl["leave_type"].as_str().unwrap_or("annual").to_string();
                    let from_date = if step == "single_date" {
                        d_trim.to_string()
                    } else {
                        fl["from_date"].as_str().unwrap_or(d_trim).to_string()
                    };
                    let to_date = d_trim.to_string();
                    let days = commands::range_days(&from_date, &to_date);
                    if days <= 0 {
                        reply = Some("⚠️ تاريخ الانتهاء يجب أن يكون بعد تاريخ البداية أو مساوياً له.".into());
                    } else {
                        fl["step"] = json!("details");
                        fl["from_date"] = json!(from_date.clone());
                        fl["to_date"] = json!(to_date.clone());
                        fl["note"] = json!("");
                        set_flow(&conn, chat, &fl);
                        if ty == "sick" {
                            reply = Some(format!(
                                "🏥 طلب إجازة مرضية\nمن {from_date} إلى {to_date} ({days} يوم)\n\n📸 أرسل الآن صورة التقرير الطبي (أو اكتب ملاحظة)، أو اضغط الزر أدناه لإرسال الطلب بدون صورة:"
                            ));
                            keyboard = Some(json!({
                                "inline_keyboard": [
                                    [{ "text": "✅ إرسال الطلب بدون صورة", "callback_data": "lv:submit" }],
                                    [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                                ]
                            }));
                        } else {
                            reply = Some(format!(
                                "📝 طلب {}\nمن {from_date} إلى {to_date} ({days} يوم)\n\n✍️ اكتب سبب/ملاحظة الإجازة الآن، أو اضغط «✅ إرسال الطلب الآن»:",
                                leave_type_label(&ty)
                            ));
                            keyboard = Some(json!({
                                "inline_keyboard": [
                                    [{ "text": "✅ إرسال الطلب الآن", "callback_data": "lv:submit" }],
                                    [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                                ]
                            }));
                        }
                    }
                }
            } else if kind == "leave" && step == "details" {
                let ty = fl["leave_type"].as_str().unwrap_or("annual").to_string();
                let fd = fl["from_date"].as_str().unwrap_or(&today).to_string();
                let td = fl["to_date"].as_str().unwrap_or(&fd).to_string();
                let note = raw_text.chars().take(400).collect::<String>();
                if ty == "sick" {
                    fl["note"] = json!(note.clone());
                    set_flow(&conn, chat, &fl);
                    reply = Some(format!(
                        "📝 تم حفظ الملاحظة: «{note}»\n\n📸 يمكنك الآن إرسال صورة التقرير الطبي، أو الضغط على «✅ إرسال الطلب الآن بدون صورة»:"
                    ));
                    keyboard = Some(json!({
                        "inline_keyboard": [
                            [{ "text": "✅ إرسال الطلب الآن بدون صورة", "callback_data": "lv:submit" }],
                            [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                        ]
                    }));
                } else {
                    clear_flow(&conn, chat);
                    match finalize_leave_request(&conn, emp_id, &name, &ty, &fd, &td, &note, None) {
                        Ok((r, t, b)) => {
                            reply = Some(r);
                            request_alert = Some((t, b, "leave"));
                        }
                        Err(e) => reply = Some(format!("⚠️ {e}")),
                    }
                }
            } else if kind == "advance" && step == "amount" {
                let cleaned: String = raw_text
                    .chars()
                    .map(|c| match c {
                        '٠'..='٩' => char::from_u32('0' as u32 + (c as u32 - '٠' as u32)).unwrap_or(c),
                        _ => c,
                    })
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if let Ok(amt) = cleaned.parse::<f64>() {
                    if amt > 0.0 {
                        fl["step"] = json!("note");
                        fl["amount"] = json!(amt);
                        set_flow(&conn, chat, &fl);
                        let cur = currency(&s);
                        reply = Some(format!(
                            "💵 مبلغ السلفة المطلوب: {amt:.2} {cur}\n\n✍️ اكتب ملاحظة أو سبب طلب السلفة، أو اضغط «✅ إرسال الطلب الآن»:"
                        ));
                        keyboard = Some(json!({
                            "inline_keyboard": [
                                [{ "text": "✅ إرسال الطلب الآن", "callback_data": "adv:submit" }],
                                [{ "text": "✖️ إلغاء الطلب", "callback_data": "flow:cancel" }]
                            ]
                        }));
                    } else {
                        reply = Some("⚠️ أدخل مبلغاً أكبر من صفر.".into());
                    }
                } else {
                    reply = Some("⚠️ أدخل مبلغ السلفة بالأرقام (مثال: 500) أو اختر من الأزرار.".into());
                }
            } else if kind == "advance" && step == "note" {
                let amt = fl["amount"].as_f64().unwrap_or(0.0);
                let note = raw_text.chars().take(400).collect::<String>();
                let cur = currency(&s);
                clear_flow(&conn, chat);
                match finalize_advance_request(&conn, emp_id, &name, amt, &note, &cur) {
                    Ok((r, t, b)) => {
                        reply = Some(r);
                        request_alert = Some((t, b, "advance"));
                    }
                    Err(e) => reply = Some(format!("⚠️ {e}")),
                }
            }
        } else if is_sup && (cmd == "ملخص" || cmd == "تقارير") {
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
            "رصيد" | "رصيدي" | "رصيد_الاجازات" => {
                reply = Some(leave_balance_text(&conn, emp_id));
            }
            "اجازة" | "إجازة" | "اجازه" | "إجازه" | "طلب_اجازة" => {
                if !enabled(&s, "tg_allow_leave", true) {
                    reply = Some(PERM_DENIED.into());
                } else {
                    clear_flow(&conn, chat);
                    reply = Some("📝 طلب إجازة جديدة\n\nاختر نوع الإجازة من الأزرار أدناه:".into());
                    keyboard = Some(leave_types_keyboard());
                }
            }
            "سلفة" | "سلفه" | "طلب_سلفة" => {
                if !enabled(&s, "tg_allow_advance", true) {
                    reply = Some(PERM_DENIED.into());
                } else {
                    set_flow(
                        &conn,
                        chat,
                        &json!({ "kind": "advance", "step": "amount" }),
                    );
                    let cur = currency(&s);
                    reply = Some(
                        "💵 طلب سلفة مالية\n\nاختر المبلغ المطلوب من الأزرار أدناه، أو اكتب المبلغ بالأرقام:".into(),
                    );
                    keyboard = Some(advance_amounts_keyboard(&cur));
                }
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

    if let Some(m) = &reply {
        log_chat_message(&conn, chat, "out", m, None);
    }
    drop(conn);

    if let Some(m) = reply {
        let _ = send(token, chat, &m, keyboard);
    }
    if let Some(body) = absence_alert {
        notify_supervisors_async(app, "📝 سبب غياب", &body);
    }
    if let Some((title, body, kind)) = request_alert {
        let _ = app.notification().builder().title(&title).body(&body).show();
        let _ = app.emit("bot-request", json!({ "kind": kind }));
        notify_supervisors_async(app, &title, &body);
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
        out.push_str("/راتب — آخر راتب\n");
    }
    out.push_str("/رصيد — رصيد الإجازات السنوية\n");
    if enabled(s, "tg_allow_leave", true) {
        out.push_str("/اجازة — تقديم طلب إجازة (سنوية / مرضية بمرفق / بدون راتب / طارئة)\n");
    }
    if enabled(s, "tg_allow_advance", true) {
        out.push_str("/سلفة — تقديم طلب سلفة مالية\n");
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
        "\n📊 الإجمالي: {present} يوم · {:.1} ساعة",
        minutes as f64 / 60.0
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

/// نهاية اليوم: ملخص للمشرفين فقط — يُرسل بعد نهاية آخر وردية فعّالة بثلاث
/// ساعات (لا بوقت ثابت)، مرة واحدة في اليوم. رسائل الموظف نفسها (غياب بعد
/// ربع ساعة / خروج مفقود بعد 3 ساعات) تُرسل من commands::shift_alerts_due
/// عبر notify_tick، فتعمل حتى لو كان البرنامج مطفأ عند الموعد.
pub(crate) fn eod_tick(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let (already, today, now_min, due) = {
        let Ok(conn) = state.db.lock() else { return };
        let s = settings(&conn);
        if !enabled(&s, "tg_enabled", false) {
            return;
        }
        if text_of(&s, "tg_token", "").is_empty() {
            return;
        }
        // التحكم نفسه في «تنبيه نهاية الشفت» — إطفاؤه يوقف الملخص أيضاً
        if !enabled(&s, "tg_alert_end", true) {
            return;
        }
        let today = today_in(&s);
        let already = s.get("tg_eod_sent").cloned().unwrap_or_default() == today;
        let now_min = now_minutes_in(&s);
        (already, today, now_min, day_end_due_min(&conn, &s))
    };
    if already || now_min < due {
        return;
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

/// موعد ملخص نهاية اليوم: آخر وردية تنتهي اليوم + 3 ساعات. الورديات التي
/// تتجاوز منتصف الليل تُستبعد من حساب اليوم (موعدُها بعد منتصف الليل)،
/// وإن لم تُقرأ أي وردية تُستخدم وردية الأساس في الإعدادات.
fn day_end_due_min(conn: &Connection, s: &HashMap<String, String>) -> i64 {
    let mut due: i64 = 0;
    if let Ok(mut st) = conn.prepare("SELECT work_start, work_end FROM shifts WHERE active=1") {
        if let Ok(rows) =
            st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            for (ws, we) in rows.flatten() {
                let start = hm_minutes(&ws);
                let end = hm_minutes(&we);
                if end <= start {
                    continue; // وردية تتجاوز منتصف الليل — موعدُها بعد اليوم
                }
                due = due.max(end + 180);
            }
        }
    }
    if due == 0 {
        due = hm_minutes(&text_of(s, "work_end", "17:30")) + 180;
    }
    due.min(24 * 60 - 1)
}

/// يرسل رسائل تنبيهات الشفت للموظفين المرتبطين (في خيط منفصل حتى لا تجمّد
/// الواجهة). الرسائل بلا محادثة مربوطة أو ببوت موقوف تُكتفى بالسجل والإشعار.
pub fn send_shift_alert_dms(app: &tauri::AppHandle, alerts: &[commands::ShiftAlert]) {
    if alerts.is_empty() {
        return;
    }
    let app = app.clone();
    let alerts: Vec<commands::ShiftAlert> = alerts.to_vec();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let Ok(conn) = state.db.lock() else { return };
        let s = settings(&conn);
        if !enabled(&s, "tg_enabled", false) {
            return;
        }
        let token = text_of(&s, "tg_token", "");
        if token.is_empty() {
            return;
        }
        for a in &alerts {
            if let Some(chat) = &a.chat {
                let _ = send(&token, chat, &a.dm, None);
            }
        }
    });
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
    fn leave_and_advance_requests_and_chat_log_work() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        conn.execute(
            "INSERT INTO employees(id,pin,name,phone,active,bot_allowed,annual_leave_balance,annual_leave_days,telegram_chat_id,created_at,updated_at)
             VALUES(10,'10','رامي','0599888777',1,1,10.0,14.0,'chat-10','x','x')",
            [],
        )
        .unwrap();

        // 1) رصيد الإجازات
        let bal_txt = leave_balance_text(&conn, 10);
        assert!(bal_txt.contains("10.0"), "{bal_txt}");
        assert!(bal_txt.contains("14.0"), "{bal_txt}");

        // 2) طلب إجازة مرضية مع مرفق صورة
        let (rep, title, _) = finalize_leave_request(
            &conn,
            10,
            "رامي",
            "sick",
            "2026-11-01",
            "2026-11-02",
            "تقرير طبي",
            Some("data:image/jpeg;base64,AA=="),
        )
        .unwrap();
        assert!(rep.contains("إجازة مرضية"));
        assert!(rep.contains("تم إرفاق الصورة"));
        assert!(title.contains("رامي"));

        // 3) طلب إجازة سنوية مدفوعة (يومان) ثم الموافقة عليها → يخصم يومين من الرصيد (10 -> 8)
        finalize_leave_request(
            &conn,
            10,
            "رامي",
            "annual",
            "2026-11-05",
            "2026-11-06",
            "سفر",
            None,
        )
        .unwrap();
        commands::add_leave_impl(&conn, 10, "2026-11-05", "2026-11-06", "annual", true, "سفر").unwrap();
        let bal_after: f64 = conn
            .query_row("SELECT annual_leave_balance FROM employees WHERE id=10", [], |r| r.get(0))
            .unwrap();
        assert_eq!(bal_after, 8.0);

        // 4) طلب سلفة
        let (adv_rep, _, _) = finalize_advance_request(&conn, 10, "رامي", 400.0, "سلفة طارئة", "₪").unwrap();
        assert!(adv_rep.contains("400.00"));

        // 5) سجل المحادثات
        log_chat_message(&conn, "chat-10", "in", "مرحبا", None);
        log_chat_message(&conn, "chat-10", "out", "أهلاً رامي", None);
        let msg_cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM telegram_messages WHERE employee_id=10", [], |r| r.get(0))
            .unwrap();
        assert_eq!(msg_cnt, 2);
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
