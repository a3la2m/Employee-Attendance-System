//! نقطة الدخول: تهيئة SQLite، ونافذة React، وصينية النظام + الإقلاع التلقائي،
//! والسحب التلقائي من جهاز البصمة عند بدء التشغيل، ومراقبة المستخدمين الجدد.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attendance;
mod commands;
mod db;
mod models;
mod payroll;
mod storage;
mod zk;

use commands::AppState;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

/// يبني صينية النظام (tray) مع عناصر: إظهار/إخفاء + خروج.
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItem::with_id(app, "show", "إظهار النافذة", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "إخفاء للصينية", true, None::<&str>)?;
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
        }
        Err(e) => {
            log::warn!("auto sync failed: {e}");
            let _ = app.emit("sync", serde_json::json!({"ok": false, "error": e}));
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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
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
            app.manage(AppState {
                db: Mutex::new(conn),
                base,
            });

            // صينية النظام + نافذة رئيسية
            if let Err(e) = build_tray(app.handle()) {
                log::warn!("tray failed: {e}");
            }
            // إخفاء النافذة عند بدء التشغيل (تعمل في الخلفية)
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }

            spawn_auto_sync(app.handle().clone());
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
            commands::restore_settings,
            commands::list_employees,
            commands::upsert_employee,
            commands::delete_employee,
            commands::device_info,
            commands::sync_device,
            commands::check_time_sync,
            commands::fix_device_time,
            commands::recompute_days,
            commands::get_month_days,
            commands::get_payroll,
            commands::archive_payroll,
            commands::get_daily_report,
            commands::add_leave,
            commands::list_leaves,
            commands::dashboard,
            commands::set_company_logo,
            commands::read_logo_data_uri,
            commands::pair_punch_pins,
            commands::server_time,
            commands::import_punches_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
