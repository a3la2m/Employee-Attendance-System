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
  status: string; // present | late | absent | leave
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
}

export interface Leave {
  id: number;
  employee_id: number;
  name: string;
  from_date: string;
  to_date: string;
  leave_type: string;
  note: string;
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
