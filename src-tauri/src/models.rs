//! نماذج البيانات (Serde) المشتركة بين Rust و React.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employee {
    pub id: i64,
    pub pin: Option<String>,
    pub name: String,
    pub job_title: String,
    pub phone: String,
    pub daily_hours: f64,
    pub break_min: i64,
    pub hourly_rate: f64,
    pub meal_allow: f64,
    pub transport_allow: f64,
    pub active: bool,
    /// تفعيل العمل الإضافي لهذا الموظف (الافتراضي مفعّل).
    pub overtime_enabled: bool,
    /// استحقاق بدل الفطور (الافتراضي نعم).
    pub meal_enabled: bool,
    /// استحقاق بدل المواصلات (الافتراضي نعم).
    pub transport_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Punch {
    pub id: i64,
    pub employee_id: Option<i64>,
    pub pin: String,
    pub device_time: String,
    pub punch_type: i64,
    pub work_code: i64,
    pub raw_status: i64,
}

/// ملخّص يوم حضور واحد (محسوب).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttendanceDay {
    pub id: i64,
    pub employee_id: i64,
    pub date: String, // YYYY-MM-DD
    pub first_in: Option<String>,
    pub last_out: Option<String>,
    pub worked_min: i64,
    pub late_min: i64,
    pub early_leave_min: i64,
    pub overtime_min: i64,
    pub absent: bool,
    pub leave_type: Option<String>,
    pub status: String, // present / absent / late / leave
}

/// إعدادات الرواتب (من جدول settings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollSettings {
    pub daily_hours: f64,
    pub break_min: i64,
    pub hourly_rate: f64,
    pub meal_allow: f64,
    pub transport_allow: f64,
    pub overtime_after_hours: f64,
    pub overtime_rate_mult: f64,
    pub grace_late_min: i64,
    pub work_start: String, // HH:MM
    pub work_end: String,   // HH:MM
}

/// نتيجة حساب راتب شهر لموظف واحد.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollResult {
    pub employee_id: i64,
    pub employee_name: String,
    pub pin: Option<String>,
    pub period: String, // YYYY-MM
    pub days_present: i64,
    pub total_worked_hours: f64,
    pub overtime_hours: f64,
    pub base_pay: f64,
    pub allowances: f64,
    pub late_deduction: f64,
    pub absence_deduction: f64,
    pub net_pay: f64,
}

/// معلومات الجهاز كما تعطيها DLL أو البروتوكول.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceInfo {
    pub connected: bool,
    pub firmware: String,
    pub platform: String,
    pub serial: String,
    pub device_name: String,
    pub users: i64,
    pub fingerprints: i64,
    pub records: i64,
}
