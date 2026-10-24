//! تجريد طبقة الجهاز — تنفيذان: بروتوكول Rust نقي (يعمل على أي OS) أو DLL عبر FFI (ويندوز).
pub mod protocol;
#[cfg(windows)]
pub mod ffi;

use crate::models::DeviceInfo;

/// نتيجة سحب خام: (pin, device_time ISO, punch_type, work_code, raw_status).
pub type RawPunch = (String, String, i64, i64, i64);
/// نتيجة موظف خام: (pin, name, card, privilege).
pub type RawUser = (String, String, u32, i64);

pub trait DeviceBackend {
    fn connect(&mut self) -> anyhow::Result<DeviceInfo>;
    fn info(&mut self) -> anyhow::Result<DeviceInfo>;
    /// سحب كل سجلات الحضور.
    fn pull_attendance(&mut self) -> anyhow::Result<Vec<RawPunch>>;
    /// سحب كل الموظفين المسجّلين على الجهاز.
    fn pull_users(&mut self) -> anyhow::Result<Vec<RawUser>>;
    /// ضبط ساعة الجهاز من السيرفر (يعالج انحراف Asia/Jerusalem).
    fn set_time(&mut self, iso: &str) -> anyhow::Result<()>;
    /// قراءة ساعة الجهاز الحالية بصيغة ISO8601.
    fn get_time_iso(&mut self) -> anyhow::Result<String>;
    /// محاولة إضافة موظف إلى الجهاز (تعمل عبر DLL؛ عبر البروتوكول قد تُرفض).
    fn add_user(&mut self, pin: &str, name: &str) -> anyhow::Result<()>;
    /// تعديل صلاحية مستخدم قائم على الجهاز: 0 = عادي، 6 = مدير، 14 = أدمن
    /// كامل (سوبر أدمن) — يحافظ على اسمه وبطاقته وبصماته كما هي.
    fn set_user_privilege(&mut self, pin: &str, privilege: i64) -> anyhow::Result<()>;
    /// حذف مستخدم من الجهاز = تعطيله عن البصمات حسب رقم البصمة.
    fn delete_user(&mut self, pin: &str) -> anyhow::Result<()>;
    fn close(&mut self);
}

/// يبني الخلفية المناسبة حسب نظام التشغيل والإعدادات.
pub fn make_backend(
    ip: &str,
    port: u16,
    comm_key: u32,
    prefer_dll: bool,
) -> Box<dyn DeviceBackend> {
    #[cfg(windows)]
    {
        if prefer_dll {
            if let Some(b) = ffi::ZkDllBackend::try_load(ip, port, comm_key) {
                log::info!("using ZKTeco DLL backend");
                return Box::new(b);
            }
        }
    }
    let _ = prefer_dll;
    log::info!("using pure-Rust ZK protocol backend");
    Box::new(protocol::ZkProtocol::new(ip, port, comm_key))
}
