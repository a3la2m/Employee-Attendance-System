// غلاف مبسّط لدوال Tauri invoke — كل استدعاء Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  BackupResult,
  Dashboard,
  DailyRow,
  DeviceInfo,
  Employee,
  Leave,
  PayrollResult,
  Settings,
  StorageInfo,
  SyncResult,
  TimeCheck,
  AttendanceDay,
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

// --- الرواتب ---
export const getPayroll = (period: string) =>
  invoke<PayrollResult[]>("get_payroll", { period });
export const archivePayroll = (period: string) =>
  invoke<number>("archive_payroll", { period });

// --- الإجازات ---
export const addLeave = (
  employee_id: number,
  from_date: string,
  to_date: string,
  leave_type: string,
  note: string,
) =>
  invoke<number>("add_leave", { employee_id, from_date, to_date, leave_type, note });
export const listLeaves = () => invoke<Leave[]>("list_leaves");

// --- عام ---
export const dashboard = () => invoke<Dashboard>("dashboard");
export const setCompanyLogo = (path: string) =>
  invoke<void>("set_company_logo", { path });
export const readLogoDataUri = (path: string) =>
  invoke<string>("read_logo_data_uri", { path });
export const importPunchesCsv = (path: string) =>
  invoke<number>("import_punches_csv", { path });
