// غلاف مبسّط لدوال Tauri invoke — كل استدعاء Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  BackupResult,
  Dashboard,
  DashboardCharts,
  DailyRow,
  DeviceInfo,
  Employee,
  EmployeePayrollRun,
  Holiday,
  Leave,
  Notification,
  PayrollMonth,
  PayrollResult,
  PresentNowItem,
  Settings,
  StorageInfo,
  SyncResult,
  TimeCheck,
  AttendanceDay,
  ClockCheck,
} from "./types";

// --- الإعدادات ---
export const getSettings = () => invoke<Settings>("get_settings");
export const saveSetting = (key: string, value: string) =>
  invoke<void>("save_setting", { key, value });
export const saveSettingsBulk = (items: Record<string, string>) =>
  invoke<void>("save_settings_bulk", { items });

// --- التخزين والنسخ الاحتياطي ---
export const getStorageInfo = () => invoke<StorageInfo>("get_storage_info");
export const setStorageDir = (dir: string) =>
  invoke<StorageInfo>("set_storage_dir", { dir });
export const backupNow = () => invoke<BackupResult>("backup_now");
export const restoreSettings = (path?: string) =>
  invoke<number>("restore_settings", { path: path ?? null });

// --- الموظفون ---
export const listEmployees = () => invoke<Employee[]>("list_employees");
export const upsertEmployee = (emp: Employee) =>
  invoke<number>("upsert_employee", { emp });
export const deleteEmployee = (id: number) =>
  invoke<void>("delete_employee", { id });

// --- الجهاز ---
export const deviceInfo = () => invoke<DeviceInfo>("device_info");
export const syncDevice = () => invoke<SyncResult>("sync_device");
export const pushUserToDevice = (pin: string, name: string) =>
  invoke<string>("push_user_to_device", { pin, name });
/** تعطيل مستخدم على الجهاز (حذفه منه) — يُستدعى عند إيقاف الموظف */
export const disableUserOnDevice = (pin: string) =>
  invoke<string>("disable_user_on_device", { pin });

/** وصف مختصر لفحص/تصحيح ساعة الجهاز — يُلحق بنص نتائج المزامنة */
export const clockNote = (c?: ClockCheck): string =>
  !c
    ? ""
    : c.fixed
      ? ` — ✅ صُحّحت ساعة الجهاز إلى ${c.set_to ?? ""}`
      : c.in_sync === false
        ? ` — ⚠️ ساعة الجهاز تختلف بـ${Math.round(Math.abs(c.diff_min ?? 0))} دقيقة`
        : "";

export interface PunchRow {
  device_time: string;
  punch_type: number;
  work_code: number;
  raw_status: number;
}

export const employeePunches = (
  employeeId: number,
  fromDate?: string,
  toDate?: string,
  limit?: number,
) =>
  invoke<PunchRow[]>("employee_punches", {
    employeeId,
    fromDate: fromDate || null,
    toDate: toDate || null,
    limit: limit ?? null,
  });
export const checkTimeSync = () => invoke<TimeCheck>("check_time_sync");
export const fixDeviceTime = () =>
  invoke<{ set_to: string; device_time_after: string }>("fix_device_time");

// --- الحضور ---
export const getMonthDays = (period: string) =>
  invoke<AttendanceDay[]>("get_month_days", { period });
export const recomputeDays = (from: string, to: string) =>
  invoke<number>("recompute_days", { from, to });
export const getDailyReport = (date: string) =>
  invoke<DailyRow[]>("get_daily_report", { date });

/** تعديل يدوي ليوم حضور: present / absent / auto أو تحويله لإجازة
 *  annual (خصم يوم من الرصيد) / sick (بلا خصم) / unpaid (بلا أجر بلا خصم). */
export const setDayManual = (
  employeeId: number,
  date: string,
  mode: "present" | "absent" | "auto" | "annual" | "sick" | "unpaid",
  firstIn: string | null,
  lastOut: string | null,
) =>
  invoke<AttendanceDay>("set_day_manual", {
    employeeId,
    date,
    mode,
    firstIn: firstIn || null,
    lastOut: lastOut || null,
  });

/** أزرار نافذة تعديل اليوم: إلغاء خصم الاستراحة / إزالة بدل الفطور. */
export const setDayFlags = (
  employeeId: number,
  date: string,
  noBreak: boolean,
  noMeal: boolean,
) =>
  invoke<AttendanceDay>("set_day_flags", {
    employeeId,
    date,
    noBreak,
    noMeal,
  });

// --- الرواتب ---
export const getPayroll = (period: string) =>
  invoke<PayrollResult[]>("get_payroll", { period });
export const archivePayroll = (period: string) =>
  invoke<number>("archive_payroll", { period });
export const payrollMonths = () => invoke<PayrollMonth[]>("payroll_months");
export const employeePayrollHistory = (employeeId: number) =>
  invoke<EmployeePayrollRun[]>("employee_payroll_history", {
    employeeId,
  });

// --- الإجازات ---
export const addLeave = (
  employeeId: number,
  fromDate: string,
  toDate: string,
  leaveType: string,
  paid: boolean,
  note: string,
) =>
  invoke<number>("add_leave", {
    employeeId,
    fromDate,
    toDate,
    leaveType,
    paid,
    note,
  });
export const deleteLeave = (id: number) => invoke<void>("delete_leave", { id });
export const listLeaves = () => invoke<Leave[]>("list_leaves");

// --- الاحتساب الشهري لأرصدة الإجازات السنوية (استحقاق ÷ 12 كل شهر مكتمل) ---
export interface AccrualResult {
  enabled: boolean;
  accrued: number;
  days: number;
  month?: string;
  details?: { name: string; days: number }[];
}
export const accrueLeaveNow = () => invoke<AccrualResult>("accrue_leave_now");

// --- الإجازات الرسمية (الأعياد) ---
export const holidaysList = () => invoke<Holiday[]>("holidays_list");
export const holidayAdd = (date: string, name: string) =>
  invoke<number>("holiday_add", { date, name });
export const holidayDelete = (id: number) =>
  invoke<void>("holiday_delete", { id });

// --- عام ---
export const dashboard = () => invoke<Dashboard>("dashboard");
export const presentNow = () => invoke<PresentNowItem[]>("present_now");

// --- الإشعارات (سجل التنبيهات + عدّاد الجرس) ---
export const listNotifications = (limit = 200) =>
  invoke<Notification[]>("list_notifications", { limit });
export const markNotificationsSeen = () =>
  invoke<void>("mark_notifications_seen");
export const clearNotifications = () => invoke<void>("clear_notifications");
export const dashboardCharts = () =>
  invoke<DashboardCharts>("dashboard_charts");
export const setCompanyLogo = (path: string) =>
  invoke<void>("set_company_logo", { path });
export const readLogoDataUri = (path: string) =>
  invoke<string>("read_logo_data_uri", { path });
export const importPunchesCsv = (path: string) =>
  invoke<number>("import_punches_csv", { path });
