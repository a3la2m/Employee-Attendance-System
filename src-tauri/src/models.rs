//! نماذج البيانات (Serde) المشتركة بين Rust و React.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employee {
    pub id: i64,
    pub pin: Option<String>,
    pub name: String,
    /// الاسم اللاتيني المخصص لجهاز البصمة (لا يدعم العربية).
    pub name_en: String,
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
    /// دفع العطلة الأسبوعية (الافتراضي نعم — تُلغى لموظف بعينه فتصبح عطلة بدون أجر).
    pub weekend_paid: bool,
    /// رصيد الإجازات السنوية (بالأيام) — يُخصم عند تسجيل إجازة سنوية ويُستعاد عند حذفها.
    pub annual_leave_balance: f64,
    /// الاستحقاق السنوي للإجازات (أيام في السنة — الافتراضي 14، قابل للتعديل لكل موظف).
    /// يُضاف منه (الاستحقاق ÷ 12) تلقائياً عن كل شهر عمل كامل.
    pub annual_leave_days: f64,
    /// آخر شهر (YYYY-MM) أُضيفت أرصدته — لتستأنف الإضافة من حيث توقفت (يديره النظام).
    pub leave_accrual_month: Option<String>,
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
    pub status: String, // present / late / absent / leave / holiday
    /// هل يوم الإجازة هذا مدفوع (Some) — None لغير الإجازات.
    pub leave_paid: Option<bool>,
    /// تعديل يدوي من المسؤول (لا تمسه إعادة الاحتساب التلقائية).
    pub manual: bool,
}

/// إعدادات الرواتب (من جدول settings).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollSettings {
    pub daily_hours: f64,
    pub break_min: i64,
    /// هل تُخصم الاستراحة القياسية من أيام الدوام؟ (زر «لا تخصم الاستراحة» —
    /// الافتراضي نعم: تُخصم دائماً حتى لو عمل الموظف أقل من ساعات الدوام).
    pub deduct_break: bool,
    pub hourly_rate: f64,
    pub meal_allow: f64,
    pub transport_allow: f64,
    pub overtime_after_hours: f64,
    pub overtime_rate_mult: f64,
    pub grace_late_min: i64,
    /// مبلغ العطلة الأسبوعية (62): في الطريقة الأسبوعية هو مبلغ الأسبوع الكامل
    /// (يُقسَّم على ستة أيام دوام)، وفي الطريقة القدية مبلغ كل يوم عطلة.
    pub weekend_allow: f64,
    /// طريقة احتساب بدل العطلة من الإعداد `weekend_pay_mode`:
    /// "weekly" (الافتراضي — 62 عن كل أسبوع ÷ أيام السبت–الخميس بالنسبة)
    /// أو "per_day" (62 عن كل يوم عطلة أسبوعية مستحق).
    pub weekend_pay_mode: String,
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
    /// بدل العطلة الأسبوعية في الفترة — حسب طريقة `weekend_pay_mode`:
    /// "weekly" (الافتراضي) = مبلغ الأسبوع (62) × (أيام الدوام المستحق من
    /// السبت–الخميس ÷ أيام الدوام المتاحة بأسبوع السبت→الجمعة)؛ و"per_day" =
    /// 62 × أيام العطلة الأسبوعية المستحقة. (شهر كامل بخمس جمع = 310 فيهما)
    pub weekend_pay: f64,
    /// عدد أيام الدوام المحضورة/المدفوعة في الفترة (رقم إعلامي).
    pub paid_workdays: i64,
    /// عدد أيام العطلة الأسبوعية التي دُفع عنها بدل (5 لشهر بخمس جمع).
    pub weekend_days_paid: i64,
    /// أيام إجازة مدفوعة (تُحتسب ضمن بدل العطلة).
    pub paid_leave_days: i64,
    /// أيام إجازة غير مدفوعة (لا تُحتسب ولا تُخصم — دفع بالساعات فقط).
    pub unpaid_leave_days: i64,
    /// أيام الإجازات الرسمية (الأعياد) خلال الفترة.
    pub holiday_days: i64,
    pub late_deduction: f64,
    pub absence_deduction: f64,
    pub net_pay: f64,
}

/// تنبيه محفوظ في سجل الإشعارات (يبقى للرجوع إليه عبر زر الجرس).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    /// late | missing_out | orphan_out
    pub kind: String,
    pub employee_id: Option<i64>,
    pub title: String,
    pub body: String,
    /// التاريخ المرتبط بالحدث (YYYY-MM-DD)
    pub day: String,
    pub created_at: String,
    pub seen: bool,
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
