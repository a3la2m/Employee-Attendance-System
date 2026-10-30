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
    /// الوردية المسنَدة للموظف — NULL يرث «الوردية الرئيسية» تلقائياً.
    #[serde(default)]
    pub shift_id: Option<i64>,
    /// صلاحية الموظف على جهاز البصمة: 0 = مستخدم عادي، 6 = مدير،
    /// 14 = أدمن كامل (سوبر أدمن). تُرسل إلى الجهاز عند الحفظ.
    #[serde(default)]
    pub device_privilege: i64,
    /// هل يُسمح لهذا الموظف باستخدام بوت تلجرام؟ (الافتراضي نعم)
    /// يُدار من شاشة الموظفين أو من جدول «الموظفون المرتبطون» في صفحة البوت.
    #[serde(default = "default_true")]
    pub bot_allowed: bool,
}

fn default_true() -> bool {
    true
}

/// وردية دوام: ساعات الدوام الأساسية التي تُحسب عليها أيام الموظف المسنَد إليها.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shift {
    pub id: i64,
    pub name: String,
    pub work_start: String, // HH:MM
    pub work_end: String,   // HH:MM
    /// ساعات الدوام الأساسية (منها الإضافي وما بعدها).
    pub daily_hours: f64,
    /// الاستراحة القياسية (دقائق) — تُخصم من أيام الدوام.
    pub break_min: i64,
    /// الوردية الرئيسية: يرثها أي موظف غير مسنَد لوردية (واحدة فقط).
    pub is_default: bool,
    /// 0 = مهجورة (تبقى في السجل لكن لا تُسنَد لموظفين جدد).
    pub active: bool,
    /// عدد الموظفين المسنَدين إليها مباشرة (رقم إعلامي في الواجهة).
    #[serde(default)]
    pub employee_count: i64,
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
    /// هل تاريخ اليوم عطلة رسمية (أحد أعياد الإعدادات) — يظهر حتى لو دوّم
    /// فيه الموظف (يوم دوام في عطلة يُدفع بسياسة العمل في العطل الرسمية).
    #[serde(default)]
    pub holiday: bool,
    /// خيار تعويض دوام هذا اليوم في العطلة الرسمية: "money" (نقدي، الافتراضي)
    /// أو "comp" (يوم بدل). يُضبط من شاشة الحضور لكل موظف/يوم على حدة.
    #[serde(default)]
    pub holiday_work_choice: Option<String>,
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
    /// أيام دُوّم فيها موظف في عطلة رسمية (سياسة «نقدي»: تُدفع بـ62 +1.5×).
    #[serde(default)]
    pub holiday_work_days: i64,
    /// أجر العمل في العطل الرسمية (السياسة النقدية):62 عن كل يوم + الساعات×1.5.
    #[serde(default)]
    pub holiday_work_pay: f64,
    /// أيام البدل التعويضية (عطلة تعويضية بدل دوام عطلة رسمية).
    #[serde(default)]
    pub comp_off_days: i64,
    /// أجر أيام البدل التعويضية (أجر يوم العطلة =62 عن كل يوم بدل).
    #[serde(default)]
    pub comp_off_pay: f64,
    pub late_deduction: f64,
    pub absence_deduction: f64,
    /// إجمالي خصم السلف لهذا الموظف في هذا الشهر (سواء من قسط واحد أو عدة أقساط مستحقة في الفترة).
    #[serde(default)]
    pub advance_deduction: f64,
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

/// لقطة إعدادات طباعة القسيمة ونسبها وأرقامها لحظة الطباعة أو لحظة الدفع —
/// تُحفظ في payslip_snapshots لتبقى القسيمة دليلاً على ما طُبع فعلاً حتى لو
/// تغيّرت إعدادات الطباعة لاحقاً (سعر الساعة، المعامل، نِسب البدلات الثلاثة).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayslipSnapshot {
    pub employee_id: i64,
    /// YYYY-MM
    pub period: String,
    /// سعر الساعة الأساسي وقتها
    pub rate: f64,
    /// معامل ساعة الإضافي وقتها
    pub ot_mult: f64,
    /// نِسب البدلات الثلاثة (%) — مجموعها 100 وقت الطباعة
    pub p_work: f64,
    pub p_risk: f64,
    pub p_bonus: f64,
    /// الأرقام المطبوعة وقتها
    pub basic: f64,
    pub overtime: f64,
    pub weekend_pay: f64,
    pub allowances: f64,
    pub holiday_work: f64,
    pub comp_off: f64,
    /// حصص البدلات الثلاثة المقطوعة فعلاً من المتبقي
    pub share_work: f64,
    pub share_risk: f64,
    pub share_bonus: f64,
    /// خصم السلفة المطبق وقتها
    #[serde(default)]
    pub advance_deduction: f64,
    /// الصافي المستحق (لا يتغيّر بالتقسيم — حارس ضد أي خطأ)
    pub net: f64,
    /// هل كان تقسيم الطباعة مفعلاً وقت الحفظ؟ (false = القسيمة طُبعت بنود
    /// الرواتب المحسوبة كما هي بدون تقسيم قانوني)
    pub enabled: bool,
    /// نسخة JSON كاملة مجمّدة من القسيمة وقتها (الأيام + البنود + الإعدادات)
    /// — تُطبع لاحقاً بالضبط كما كانت حتى لو تغيّر الحضور أو الإعدادات بعدها.
    pub sheet: Option<String>,
}

/// قسط واحد من سلفة موظف مستحق على شهر راتب معيّن.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceInstallment {
    pub id: i64,
    pub advance_id: i64,
    pub employee_id: i64,
    /// YYYY-MM
    pub period: String,
    pub amount: f64,
    /// هل تم خصم هذا القسط؟ (true = خصمت، false = فعالة لم تخصم بعد)
    pub deducted: bool,
    pub deducted_at: Option<String>,
}

/// سلفة موظف مع قائمة أقساطها وملخص المبالغ المخصومة والباقية.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmployeeAdvance {
    pub id: i64,
    pub employee_id: i64,
    pub amount: f64,
    /// "single" (على الراتب القادم دفعة واحدة) أو "installments" (تقسيط على الرواتب القادمة)
    pub mode: String,
    pub installments_count: i64,
    /// YYYY-MM أول شهر يبدأ فيه الخصم
    pub start_period: String,
    pub note: String,
    pub created_at: String,
    pub installments: Vec<AdvanceInstallment>,
    pub deducted_amount: f64,
    pub remaining_amount: f64,
    /// هل السلفة فعالة (يوجد على الأقل قسط لم يُخصم بعد)؟
    pub active: bool,
}

/// مشرف يستقبل تنبيهات الموظفين عبر تلجرام.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramSupervisor {
    pub id: i64,
    pub name: String,
    pub phone: String,
    /// معرّف محادثة تلجرام (NULL = لم يشارك رقمه بعد).
    pub chat_id: Option<String>,
    pub linked_at: Option<String>,
    pub active: bool,
}

/// حالة ربط موظف ببوت تلجرام (لعرضها في شاشة الإعدادات).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramLink {
    pub employee_id: i64,
    pub name: String,
    pub phone: String,
    pub active: bool,
    /// هل البوت مسموح لهذا الموظف؟ (0 = ممنوع)
    pub bot_allowed: bool,
    pub chat_id: Option<String>,
    pub linked_at: Option<String>,
}

/// سبب غياب أرسله الموظف عبر البوت.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbsenceNote {
    pub id: i64,
    pub employee_id: i64,
    pub employee_name: String,
    pub date: String,
    pub reason: String,
    pub source: String,
    pub created_at: String,
}

/// طلب إجازة مرسل من البوت.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaveRequest {
    pub id: i64,
    pub employee_id: i64,
    pub employee_name: String,
    pub annual_leave_balance: f64,
    pub leave_type: String,
    pub paid: bool,
    pub from_date: String,
    pub to_date: String,
    pub days_count: i64,
    pub note: String,
    pub has_attachment: bool,
    pub attachment_data: Option<String>,
    pub status: String,
    pub admin_note: String,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// طلب سلفة مرسل من البوت.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceRequest {
    pub id: i64,
    pub employee_id: i64,
    pub employee_name: String,
    pub amount: f64,
    pub note: String,
    pub status: String,
    pub advance_id: Option<i64>,
    pub admin_note: String,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// رسالة مسجلة في سجل محادثات بوت تلجرام.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChatMessage {
    pub id: i64,
    pub chat_id: String,
    pub employee_id: Option<i64>,
    pub employee_name: String,
    pub employee_phone: String,
    pub direction: String,
    pub body: String,
    pub attachment_data: Option<String>,
    pub created_at: String,
}
