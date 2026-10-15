//! تحديد مكان تخزين قاعدة البيانات وملفات الإعدادات + النسخ الاحتياطي.
//!
//! الملفات كلها في مجلد واحد يحدده المستخدم (يمكن أن يكون مجلداً متزامناً مع
//! Google Drive / Dropbox ...) وبأسماء إنجليزية منظّمة:
//!
//! ```text
//! <storage-dir>/
//! ├── attendance.db                       # قاعدة البيانات (SQLite)
//! ├── attendance-settings.json            # نسخة نصّية من كل الإعدادات
//! └── attendance-backup-YYYYMMDD-HHMMSS.db # لقطات قاعدة البيانات
//! ```
//!
//! مسار «الوضع الأصلي» لملف التهيئة يبقى في مجلد إعدادات التطبيق (تلقائي)،
//! وهو من يحفظ مؤشّر `storage_dir` (التي قد تنقله لاحقاً).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAIN_DB: &str = "attendance.db";
pub const SETTINGS_JSON: &str = "attendance-settings.json";
pub const STORAGE_CONFIG: &str = "storage-config.json";

/// محتوى ملف `storage-config.json` (يحدّد المجلد الذي يعيش فيه everything).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageConfig {
    /// المجلد الذي تُخزَّن فيه attendance.db وattendance-settings.json.
    pub storage_dir: Option<String>,
}

/// المسار لملف `storage-config.json` داخل مجلد إعدادات التطبيق.
pub fn config_file(base: &Path) -> PathBuf {
    base.join(STORAGE_CONFIG)
}

/// يقرأ ملف التهيئة؛ إن لم يوجد أو فسد يُرجع افتراضياً (المجلد الأساسي).
pub fn load(base: &Path) -> StorageConfig {
    let path = config_file(base);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// يكتب ملف التهيئة (ينشئ المجلد الأب عند اللزوم).
pub fn save(base: &Path, cfg: &StorageConfig) -> Result<(), String> {
    if let Some(parent) = config_file(base).parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(config_file(base), json).map_err(|e| e.to_string())
}

/// المجلد النشط فعلياً: المجلد المختار إن وُجد وموجود، وإلا المجلد الأساسي.
pub fn resolve_dir(base: &Path) -> PathBuf {
    if let Some(dir) = load(base).storage_dir.filter(|s| !s.trim().is_empty()) {
        let p = PathBuf::from(&dir);
        if p.exists() {
            return p;
        }
        // إن لم يكن موجوداً أنشأناه الآن
        let _ = fs::create_dir_all(&p);
        return p;
    }
    base.to_path_buf()
}

/// الملفات المرتبطة بمجلد التخزين (تُستخدم في الإظهار والنسخ الاحتياطي).
#[derive(Debug, Clone, Serialize)]
pub struct StorageInfo {
    /// المجلد النشط الحالي.
    pub storage_dir: String,
    /// مسار القاعدة النشطة فعلياً (التي هي مفتوحة الآن).
    pub db_path: String,
    /// ملف `storage-config.json` (المؤشّر الثابت).
    pub config_file: String,
    /// مسار `attendance-settings.json` داخل مجلد التخزين.
    pub settings_json: String,
    /// هل المجلد المختار منضبط حالياً أم نستخدم الوضع الأساسي؟
    pub is_custom: bool,
}

impl StorageInfo {
    pub fn new(base: &Path, db_path: impl AsRef<Path>) -> Self {
        let cfg = load(base);
        let active = resolve_dir(base);
        Self {
            storage_dir: active.display().to_string(),
            db_path: db_path.as_ref().display().to_string(),
            config_file: config_file(base).display().to_string(),
            settings_json: active.join(SETTINGS_JSON).display().to_string(),
            is_custom: cfg.storage_dir.is_some(),
        }
    }
}
