//! نقطة الدخول: تهيئة SQLite، ونافذة React، وصينية النظام + الإقلاع التلقائي،
//! والسحب التلقائي من جهاز البصمة عند بدء التشغيل، ومراقبة المستخدمين الجدد.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attendance;
mod commands;
mod db;
mod models;
mod payroll;
mod storage;
mod telegram;
mod zk;

use commands::AppState;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// يبني صينية النظام (tray) مع عناصر: إظهار/إخفاء + خروج.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItem::with_id(app, "show", "إظهار النافذة", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "إخفاء إلى شريط الإشعارات", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "خروج", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("icon"))
        .tooltip("نظام الحضور والرواتب")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "hide" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // نقرة مزدوجة → إظهار النافذة
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// خلفية: السحب التلقائي من الجهاز عند الإقلاع ثم كل فترة (auto_sync_min).
fn spawn_auto_sync(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // سحب فوري عند الإقلاع
        sync_once(&app);
        loop {
            // اقرأ فترة المزامنة من الإعدادات
            let minutes = {
                let state = app.state::<AppState>();
                let guard = state.db.lock();
                match guard {
                    Ok(conn) => crate::db::all_settings(&conn)
                        .map(|s| {
                            s.get("auto_sync_min")
                                .and_then(|v| v.parse::<u64>().ok())
                                .unwrap_or(5)
                        })
                        .unwrap_or(5),
                    Err(_) => 5,
                }
            };
            let secs = minutes.max(1) * 60;
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            sync_once(&app);
        }
    });
}

/// خلفية: فحص التنبيهات كل دقيقة — تأخير الدوام (عند الوقت المحدد)، خروج
/// مفقود، وخروج بلا دخول — تُحفظ كلها في سجل الإشعارات (زر الجرس في
/// الشريط الجانبي) وتظهر كإشعارات سطح المكتب عند حدوثها.
fn spawn_notifications(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // فحص فوري عند الإقلاع: التنبيهات التي حان وقتها أثناء إيقاف البرنامج
        // أو نوّم الجهاز تُرسَل الآن (الشرط «حان الموعد» + منع التكرار ليومه).
        notify_tick(&app);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            notify_tick(&app);
        }
    });
}

/// فحص واحد للتنبيهات (يُستدعى كل دقيقة).
fn notify_tick(app: &tauri::AppHandle) {
    use chrono::Timelike;
    use tauri_plugin_notification::NotificationExt;
    let state = app.state::<AppState>();
    let today;
    let mut fresh; // تنبيهات جديدة (خروج مفقود / خروج بلا دخول)
    let alerts; // تنبيهات الشفت (غياب/خروج مفقود) ذات رسالة الموظف المباشرة
    let lates; // المتأخرون اليوم (يُحفظون في السجل أيضاً)
    {
        let Ok(conn) = state.db.lock() else { return };
        let Ok(s) = crate::db::all_settings(&conn) else { return };
        let tz_name = s
            .get("timezone")
            .cloned()
            .unwrap_or_else(|| "Asia/Jerusalem".into());
        use chrono_tz::Tz;
        let tz: Tz = tz_name.parse().unwrap_or(chrono_tz::Asia::Jerusalem);
        let now = chrono::Utc::now().with_timezone(&tz).naive_local();
        today = now.format("%Y-%m-%d").to_string();

        let now_min = now.hour() as i64 * 60 + now.minute() as i64;

        // 0) تنبيهات الشفت (رسالة الموظف على تلجرام) — تُفحص أولاً لأنها تملك
        //    تنبيه «غياب» نفسه: أول من يحفظه يمنع تكراره.
        alerts = commands::shift_alerts_due(
            &conn,
            &today,
            now_min,
            s.get("tg_alert_start").map(|v| v != "0").unwrap_or(true),
            s.get("tg_alert_end").map(|v| v != "0").unwrap_or(true),
        );

        // 1) الأحداث الناقصة → تُفحص لكل موظف بعد بداية شفته بـ 15 دقيقة
        fresh = commands::detect_missing_events_at(&conn, &today, Some(now_min));

        // 2) تنبيه التأخير — لكل موظف حسب شفته (بعد بداية الدوام بـ 15 دقيقة)،
        //    ويُحفظ في السجل مرة واحدة لكل موظف في اليوم.
        lates = if s
            .get("late_notify_enabled")
            .map(|v| v != "0")
            .unwrap_or(true)
        {
            commands::lates_due_today(&conn, &today, now_min)
        } else {
            Vec::new()
        };

        // 4) الإضافة التلقائية لأرصدة الإجازات السنوية — مرة واحدة يومياً
        //    (الاحتساب نفسه يستأنف من حيث توقف عبر leave_accrual_month،
        //    فالتشغيل المتكرر آمن ولا يُضيف إلا ما لم يُحتسب).
        if s.get("leave_accrual_last_run").cloned().unwrap_or_default() != today {
            let _ = commands::accrue_leave_impl(&conn, now.date());
        }
    }

    // تنبيهات الشفت التي فازت بالحفظ تدخل إشعار سطح المكتب والمشرفين
    fresh.extend(alerts.iter().map(|a| a.fresh_line.clone()));
    // رسائل تلجرام المباشرة للموظفين (إن كان البوت مفعّلاً)
    crate::telegram::send_shift_alert_dms(app, &alerts);
    // ملخص نهاية اليوم للمشرفين — بعد نهاية آخر شفت بـ 3 ساعات (بلا وقت ثابت)
    crate::telegram::eod_tick(app);

    // 3) إشعارات سطح المكتب للتنبيهات الجديدة فقط
    if !fresh.is_empty() {
        let lines: String = fresh
            .iter()
            .take(8)
            .map(|x| format!("• {x}\n"))
            .collect();
        let more = if fresh.len() > 8 {
            format!("… و {} آخرين", fresh.len() - 8)
        } else {
            String::new()
        };
        let body = format!("{lines}{more}").trim_end().to_string();
        let _ = app
            .notification()
            .builder()
            .title("🔔 تنبيهات الحضور")
            .body(&body)
            .show();
        crate::telegram::notify_supervisors_async(app, "🔔 تنبيهات الحضور", &body);
    }
    if !lates.is_empty() {
        let mut body = String::new();
        for (_, name, late, fin) in lates.iter().take(12) {
            let t = fin
                .split('T')
                .nth(1)
                .map(|x| x[..5.min(x.len())].to_string())
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| "—".into());
            body.push_str(&format!("• {name} — تأخير {late} دقيقة (حضور {t})\n"));
        }
        if lates.len() > 12 {
            body.push_str(&format!("… و {} آخرين", lates.len() - 12));
        }
        let _ = app
            .notification()
            .builder()
            .title(format!("⏰ تأخير عن الدوام — {today}"))
            .body(body.trim_end())
            .show();
        crate::telegram::notify_supervisors_async(
            app,
            &format!("⏰ تأخير عن الدوام — {today}"),
            body.trim_end(),
        );
    }
}


fn sync_once(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    match commands::sync_device_internal(&state) {
        Ok(v) => {
            let new_punches = v.get("new_punches").and_then(|x| x.as_i64()).unwrap_or(0);
            if new_punches > 0 {
                let _ = app.emit(
                    "sync",
                    serde_json::json!({"new_punches": new_punches, "ok": true}),
                );
            }
            // ساعة الجهاز: أبلغ بتصحيحها التلقائي أو بقاء خطئها — نتيجة الفحص
            // تأتي مع المزامنة نفسها (بلا اتصال ثانٍ للفحص)
            if let Some(c) = v.get("clock").and_then(|x| x.as_object()) {
                if c.get("fixed").and_then(|x| x.as_bool()) == Some(true) {
                    let _ = app.emit(
                        "sync",
                        serde_json::json!({
                            "ok": true,
                            "new_punches": 0,
                            "clock_fixed": {
                                "set_to": c.get("set_to"),
                                "diff_min": c.get("diff_min"),
                            },
                        }),
                    );
                } else if c.get("in_sync").and_then(|x| x.as_bool()) == Some(false) {
                    let diff = c.get("diff_min").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let _ = app.emit(
                        "sync",
                        serde_json::json!({
                            "ok": true,
                            "new_punches": 0,
                            "clock_diff_min": diff,
                            "device_time": c.get("device_time"),
                        }),
                    );
                }
            }
        }
        Err(e) => {
            log::warn!("auto sync failed: {e}");
            let _ = app.emit("sync", serde_json::json!({"ok": false, "error": e}));
            // إشعار واحد عند فقد الاتصال بالجهاز، وآخر عند عودته (يتم تتبعه داخل sync_device_internal لاحقاً)
        }
    }
}

/// مراقبة المستخدمين الجدد في ويندوز: عند ظهور SID جديد في ProfileList
/// نرسل إشعاراً ونطلب بيانات المستخدم الأساسية (يُنفَّذ على ويندوز فقط).
#[cfg(windows)]
fn watch_new_windows_users(app: tauri::AppHandle) {
    use std::collections::HashSet;
    use tauri_plugin_notification::NotificationExt;
    tauri::async_runtime::spawn_blocking(move || {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        use winreg::RegKey;
        let path = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
        let mut known: HashSet<String> = HashSet::new();
        // التهيئة: سجّل الموجودين دون إشعار
        if let Ok(hkcu) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path) {
            for name in hkcu.enum_keys().flatten() {
                if name.starts_with("S-1-5-21-") {
                    known.insert(name);
                }
            }
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            if let Ok(hkcu) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path) {
                for name in hkcu.enum_keys().flatten() {
                    if name.starts_with("S-1-5-21-") && !known.contains(&name) {
                        known.insert(name.clone());
                        // اسم المستخدم من ProfileImagePath إن توفّر
                        let display = hkcu
                            .open_subkey(&name)
                            .ok()
                            .and_then(|k| k.get_value::<String, _>("ProfileImagePath").ok())
                            .unwrap_or_else(|| name.clone());
                        let _ = app.emit(
                            "new-windows-user",
                            serde_json::json!({"sid": name, "profile": display}),
                        );
                        if let Some(state) = app.try_state::<AppState>() {
                            let _ = state.db.lock().map(|conn| {
                                let _ = crate::db::set_setting(
                                    &conn,
                                    "pending_user_prompt",
                                    &format!("يوجد مستخدم ويندوز جديد: {} — أدخل بيانات الموظف الأساسية",
                                        display),
                                );
                            });
                        }
                        // إشعار نظام
                        let _ = app
                            .notification()
                            .builder()
                            .title("مستخدم جديد في ويندوز")
                            .body(format!(
                                "تم رصد مستخدم جديد: {}\nافتح التطبيق لإدخال بيانات الموظف الأساسية.",
                                display
                            ))
                            .show();
                    }
                }
            }
        }
    });
}

#[cfg(not(windows))]
fn watch_new_windows_users(_app: tauri::AppHandle) {
    // لا يوجد نظام مستخدمين في ويندوز على لينكس — لا شيء.
}

pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .setup(|app| {
            // مجلد الإعدادات الأساسي (يحوي storage-config.json المؤشّر للمجلد المختار)
            let base = app
                .path()
                .app_config_dir()
                .expect("app_config_dir")
                .to_path_buf();
            std::fs::create_dir_all(&base)?;
            // المجلد النشط فعلياً (المختار من المستخدم أو الأساسي)
            let dir = storage::resolve_dir(&base);
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join(storage::MAIN_DB))?;
            // بيانات تجريبية لأول تشغيل: تملأ لوحة التحكم إذا كانت القاعدة فارغة
            // تماماً (تتوقف تلقائياً إذا وُجد موظفون حقيقيون).
            match commands::seed_demo_if_empty(&conn) {
                Ok(n) if n > 0 => log::info!("demo data: {n} employees seeded"),
                Ok(_) => {}
                Err(e) => log::warn!("demo seed failed: {e}"),
            }
            // تنظيف فوري: أيام «غياب» مستقبلية أُنشئت قبل الإصلاح (لا غياب لأيام
            // لم تأتِ بعد) — لا يتوقّف على نجاح الاتصال بالجهاز.
            let cleaned_future = commands::cleanup_future_absences(&conn);
            if cleaned_future > 0 {
                log::info!("cleaned {cleaned_future} future absence rows");
            }
            // ترميز الصفوف القديمة من سجل المحادثة (callback_data خام مثل
            // «adv:amt:500») إلى نص مقروء — مرة واحدة عند كل إقلاع.
            let relabeled = telegram::rewrite_legacy_chat_log(&conn);
            if relabeled > 0 {
                log::info!("relabeled {relabeled} legacy chat-log rows");
            }
            app.manage(AppState {
                db: Mutex::new(conn),
                base,
            });

            // إعادة احتساب فورية عند الإقلاع (لا تتوقّف على نجاح الاتصال
            // بالجهاز) — تشمل إلغاء الإجازات التي دوّم فيها الموظف فعلياً،
            // وتحديث صف اليوم وترحيل التاريخ إلى اليوم المحلي الجديد.
            if let Some(state) = app.try_state::<AppState>() {
                match commands::recompute_recent(&state, 60) {
                    Ok(n) => log::info!("startup recompute: {n} rows"),
                    Err(e) => log::warn!("startup recompute failed: {e}"),
                }
            }

            // أيقونة شريط الإشعارات
            if let Err(e) = build_tray(app.handle()) {
                log::warn!("tray failed: {e}");
            }
            // الإقلاع التلقائي يمرّر ‎--background → يبدأ في الخلفية بلا نافذة؛
            // التشغيل اليدوي يُظهر النافذة دائماً.
            let start_background = std::env::args().any(|a| a == "--background");
            if let Some(w) = app.get_webview_window("main") {
                if start_background {
                    let _ = w.hide();
                } else {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }

            spawn_auto_sync(app.handle().clone());
            spawn_notifications(app.handle().clone());
            telegram::spawn_bot(app.handle().clone());
            watch_new_windows_users(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_setting,
            commands::save_settings_bulk,
            commands::get_storage_info,
            commands::set_storage_dir,
            commands::backup_now,
            commands::export_storage_files,
            commands::restore_settings,
            commands::list_employees,
            commands::upsert_employee,
            commands::delete_employee,
            commands::list_shifts,
            commands::upsert_shift,
            commands::set_default_shift,
            commands::delete_shift,
            commands::assign_employees_to_shift,
            commands::reassign_shift_recompute,
            commands::device_info,
            commands::sync_device,
            commands::push_user_to_device,
            commands::get_next_device_pin,
            commands::disable_user_on_device,
            commands::set_device_privilege,
            commands::clear_demo_data,
            commands::employee_punches,
            commands::check_time_sync,
            commands::fix_device_time,
            commands::recompute_days,
            commands::get_month_days,
            commands::get_payroll,
            commands::archive_payroll,
            commands::get_daily_report,
            commands::set_day_manual,
            commands::set_day_flags,
            commands::dashboard_charts,
            commands::add_leave,
            commands::delete_leave,
            commands::accrue_leave_now,
            commands::list_leaves,
            commands::holidays_list,
            commands::holiday_add,
            commands::holiday_delete,
            commands::assign_comp_off,
            commands::set_holiday_work_choice,
            commands::dashboard,
            commands::set_company_logo,
            commands::read_logo_data_uri,
            commands::pair_punch_pins,
            commands::server_time,
            commands::import_punches_csv,
            commands::list_notifications,
            commands::mark_notifications_seen,
            commands::clear_notifications,
            commands::present_now,
            commands::payroll_months,
            commands::employee_payroll_history,
            commands::mark_salary_paid,
            commands::unmark_salary_paid,
            commands::salary_payments,
            commands::save_payslip_snapshot,
            commands::payslip_snapshots,
            commands::list_employee_advances,
            commands::add_employee_advance,
            commands::toggle_advance_installment,
            commands::delete_employee_advance,
            telegram::telegram_test,
            telegram::telegram_supervisors,
            telegram::telegram_add_supervisor,
            telegram::telegram_delete_supervisor,
            telegram::telegram_unlink_supervisor,
            telegram::telegram_links,
            telegram::telegram_unlink_employee,
            telegram::absence_notes_list,
            telegram::telegram_set_bot_allowed,
            telegram::telegram_broadcast,
            telegram::leave_requests_list,
            telegram::decide_leave_request,
            telegram::advance_requests_list,
            telegram::decide_advance_request,
            telegram::telegram_chat_messages,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // الإغلاق = الإخفاء إلى شريط النظام (يواصل العمل في الخلفية)
                api.prevent_close();
                let _ = window.hide();
                use tauri_plugin_notification::NotificationExt;
                let _ = window
                    .app_handle()
                    .notification()
                    .builder()
                    .title("نظام الحضور والرواتب")
                    .body("يعمل التطبيق في الخلفية — نقرة مزدوجة على أيقونته في شريط الإشعارات تُظهره، وقائمة أيمن بها «خروج» تنهيه.")
                    .show();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
