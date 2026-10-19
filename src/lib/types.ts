// أنواع البيانات المطابقة لـ Rust (serde).

export interface Employee {
  id: number;
  pin: string | null;
  name: string;
  /** الاسم اللاتيني المخصص لجهاز البصمة (لا يدعم العربية). */
  name_en: string;
  job_title: string;
  phone: string;
  daily_hours: number;
  break_min: number;
  hourly_rate: number;
  meal_allow: number;
  transport_allow: number;
  active: boolean;
  /** تفعيل العمل الإضافي (الافتراضي مفعّل؛ يُطفأ لموظف بعينه). */
  overtime_enabled: boolean;
  /** استحقاق بدل الفطور. */
  meal_enabled: boolean;
  /** استحقاق بدل المواصلات. */
  transport_enabled: boolean;
  /** دفع العطلة الأسبوعية (الافتراضي نعم — تُلغى لموظف بعينه فتصبح عطلة بدون أجر). */
  weekend_paid: boolean;
  /** رصيد الإجازات السنوية (بالأيام) — يُخصم عند تسجيل إجازة سنوية. */
  annual_leave_balance: number;
  /** استحقاق الإجازة السنوية في السنة (أيام) — الافتراضي 14؛ يُضاف منه 1/12 شهرياً للرصيد. */
  annual_leave_days: number;
  /** آخر شهر احتُسب رصيد الإجازة (YYYY-MM) — للاحتساب من حيث توقف. */
  leave_accrual_month: string | null;
}

export interface AttendanceDay {
  id: number;
  employee_id: number;
  date: string;
  first_in: string | null;
  last_out: string | null;
  worked_min: number;
  late_min: number;
  early_leave_min: number;
  overtime_min: number;
  absent: boolean;
  leave_type: string | null;
  status: string; // present | late | absent | leave | holiday | weekend
  /** هل يوم الإجازة هذا مدفوع — null لغير الإجازات. */
  leave_paid?: boolean | null;
  /** تعديل يدوي من المسؤول (لا تمسه إعادة الاحتساب). */
  manual?: boolean;
}

export interface PayrollResult {
  employee_id: number;
  employee_name: string;
  pin: string | null;
  period: string;
  days_present: number;
  total_worked_hours: number;
  overtime_hours: number;
  base_pay: number;
  allowances: number;
  /** بدل العطلة الأسبوعية = بدل اليوم (`weekend_allow` — افتراض 62) × أيام
   * العطلة الأسبوعية المستحقة (يوم عطلة دون عمل وأسبوعه فيه دوام). */
  weekend_pay: number;
  /** عدد أيام الدوام المدفوعة في الفترة (رقم إعلامي). */
  paid_workdays: number;
  /** عدد أيام العطلة الأسبوعية المستحقة لبدل العطلة (5 لشهر بخمس جمع). */
  weekend_days_paid: number;
  /** أيام إجازة مدفوعة (تُحتسب ضمن بدل العطلة). */
  paid_leave_days: number;
  /** أيام إجازة غير مدفوعة ( بلا خصم — دفع بالساعات فقط). */
  unpaid_leave_days: number;
  /** أيام الإجازات الرسمية (الأعياد). */
  holiday_days: number;
  late_deduction: number;
  absence_deduction: number;
  net_pay: number;
}

export interface DeviceInfo {
  connected: boolean;
  firmware: string;
  platform: string;
  serial: string;
  device_name: string;
  users: number;
  fingerprints: number;
  records: number;
}

/// تنبيه محفوظ في سجل الإشعارات (زر الجرس).
export interface Notification {
  id: number;
  /** late | missing_out | orphan_out */
  kind: string;
  employee_id: number | null;
  title: string;
  body: string;
  /** التاريخ المرتبط بالحدث (YYYY-MM-DD). */
  day: string;
  created_at: string;
  seen: boolean;
}

/// موظف حاضر الآن: آخر بصمة اليوم له بصمة دخول (زمن الجهاز المحلي).
export interface PresentNowItem {
  employee_id: number;
  name: string;
  pin: string;
  /** وقت آخر دخول (ISO بزمن الجهاز المحلي). */
  last_in: string;
}

/** شهر رواتب مؤرشف في payroll_runs. */
export interface PayrollMonth {
  period: string; // YYYY-MM
  employees: number;
  net_total: number;
  generated_at: string;
}

/** راتب موظف في شهر مؤرشف (سجل تاريخي). */
export interface EmployeePayrollRun {
  period: string; // YYYY-MM
  days_present: number;
  total_worked_hours: number;
  overtime_hours: number;
  base_pay: number;
  allowances: number;
  weekend_pay: number;
  net_pay: number;
  generated_at: string;
}

export interface Dashboard {
  date: string;
  employees: number;
  present: number;
  absent: number;
  total_punches: number;
  last_sync: string;
}

export interface DailyRow {
  id: number;
  employee_id: number;
  name: string;
  pin: string;
  first_in: string | null;
  last_out: string | null;
  worked_min: number;
  late_min: number;
  early_leave_min: number;
  overtime_min: number;
  absent: boolean;
  status: string;
  /** تعديل يدوي من المسؤول (لا تمسه إعادة الاحتساب). */
  manual?: boolean;
  /** نوع الإجازة (annual/sick) إن كان اليوم إجازة. */
  leave_type?: string | null;
  /** هل الإجازة مدفوعة — null لغير الإجازات. */
  leave_paid?: boolean | null;
  /** مُلغي خصم الاستراحة لهذا اليوم (زر في نافذة التعديل). */
  no_break?: boolean;
  /** مُزاع بدل الفطور لهذا اليوم (زر في نافذة التعديل). */
  no_meal?: boolean;
}

export interface TimeCheck {
  device_time: string;
  server_time: string;
  diff_minutes: number;
  timezone: string;
  in_sync: boolean;
}

export interface SyncResult {
  device: DeviceInfo;
  new_users_added: number;
  new_punches: number;
  days_recomputed: number;
  last_sync_at: string;
  /** نتيجة فحص/تصحيح ساعة الجهاز أثناء المزامنة */
  clock?: ClockCheck;
}

export interface ClockCheck {
  /** null = تعذّر الفحص */
  in_sync: boolean | null;
  diff_min?: number;
  device_time?: string;
  device_time_before?: string;
  set_to?: string;
  fixed?: boolean;
  error?: string;
}

export interface Leave {
  id: number;
  employee_id: number;
  name: string;
  from_date: string;
  to_date: string;
  leave_type: string;
  note: string;
  /** هل الإجازة مدفوعة (تُحتسب ضمن بدل العطلة لكل يوم دوام). */
  paid: boolean;
}

/** إجازة رسمية (عيد) — مدفوعة لكل الموظفين. */
export interface Holiday {
  id: number;
  date: string; // YYYY-MM-DD
  name: string;
}

export type Settings = Record<string, string>;

/** معلومات مكان تخزين القاعدة + ملفات النسخ الاحتياطي. */
export interface StorageInfo {
  storage_dir: string;
  db_path: string;
  config_file: string;
  settings_json: string;
  is_custom: boolean;
}

/** نتيجة النسخ الاحتياطي المتكامل. */
export interface BackupResult {
  ok: boolean;
  db_backup: string;
  settings_json: string;
  storage_dir: string;
}

/** بيانات رسوم لوحة التحكم. */
export interface DayStat {
  date: string;
  label: string;
  present: number;
  late: number;
  absent: number;
  leave: number;
  /** إجازات رسمية (أعياد) في هذا اليوم. */
  holiday: number;
  hours: number;
  weekend: boolean;
}

export interface MonthStat {
  present: number;
  late: number;
  absent: number;
  leave: number;
  /** إجازات رسمية (أعياد) خلال الشهر. */
  holiday: number;
  /** أيام العطلة الأسبوعية خلال الشهر (سُجّلت تلقائياً). */
  weekend: number;
  workdays: number;
  hours: number;
}

export interface TopStat {
  name: string;
  hours: number;
}

export interface DashboardCharts {
  last7: DayStat[];
  month: MonthStat;
  top: TopStat[];
}
