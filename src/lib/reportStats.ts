/** إحصاءات الموظفين الشهرة للتقارير — دوال نقية تُختبر في vitest.
 *
 * تُبنى من سجلات الحضور الشهرية (getMonthDays) + قائمة الموظفين، وتحوّل
 * سطور الأيام إلى صفوف تقارير: غياب / تأخير وانصراف مبكر / إضافي / إجازات.
 */
import type { AttendanceDay, Employee } from "./types";

/** صف إحصاءات موظف في شهر واحد. */
export interface EmpStat {
  emp: Employee;
  /** تواريخ أيام الغياب (YYYY-MM-DD). */
  absentDates: string[];
  lateDays: number;
  lateMin: number;
  /** أطول تأخير في الشهر (دقائق). */
  worstLateMin: number;
  earlyDays: number;
  earlyMin: number;
  otMin: number;
  paidLeaveDays: number;
  unpaidLeaveDays: number;
  /** أيام العطل التعويضية (نوعها comp). */
  compDays: number;
  /** أيام الدوام في العطل الرسمية (الاختيار النقدي). */
  holidayWorkDays: number;
}

const blank = (emp: Employee): EmpStat => ({
  emp,
  absentDates: [],
  lateDays: 0,
  lateMin: 0,
  worstLateMin: 0,
  earlyDays: 0,
  earlyMin: 0,
  otMin: 0,
  paidLeaveDays: 0,
  unpaidLeaveDays: 0,
  compDays: 0,
  holidayWorkDays: 0,
});

/** بناء إحصاءات كل الموظفين في الشهر — بالترتيب الأبجدي، بلا صفوف فارغة.
 *
 * قواعد الاحتساب (مطابقة لـ payroll.rs):
 * - الغياب: status="absent".
 * - الإجازات: status="leave" — comp تُعدّ تعويضية، والباقي مدفوع/غير مدفوع.
 * - دوام العطلة الرسمية: عليه عطلة رسمية + له بصمة + الاختيار ليس "comp".
 * - التأخير والانصراف المبكر: من الدقائق المحسوبة على أيام الدوام فقط.
 * - الإضافي: مجموع overtime_min في أيام الدوام (لا في العطل الرسمية).
 */
export function buildEmpStats(
  days: AttendanceDay[],
  emps: Employee[],
): EmpStat[] {
  const byEmp = new Map<number, EmpStat>();
  for (const d of days) {
    let s = byEmp.get(d.employee_id);
    if (!s) {
      const emp = emps.find((e) => e.id === d.employee_id);
      if (!emp) continue; // موظف محذوف — لا صف له
      s = blank(emp);
      byEmp.set(d.employee_id, s);
    }
    if (d.status === "absent") {
      s.absentDates.push(d.date);
      continue;
    }
    if (d.status === "leave") {
      if (d.leave_type === "comp") s.compDays += 1;
      else if (d.leave_paid) s.paidLeaveDays += 1;
      else s.unpaidLeaveDays += 1;
      continue;
    }
    if (d.status === "holiday" || d.status === "weekend") continue;
    // يوم دوام فعلي (أو عطلة رسمية دُوّم فيها)
    if (d.holiday && d.holiday_work_choice !== "comp") s.holidayWorkDays += 1;
    if (d.late_min > 0) {
      s.lateDays += 1;
      s.lateMin += d.late_min;
      s.worstLateMin = Math.max(s.worstLateMin, d.late_min);
    }
    if (d.early_leave_min > 0) {
      s.earlyDays += 1;
      s.earlyMin += d.early_leave_min;
    }
    s.otMin += d.overtime_min;
  }
  return [...byEmp.values()].sort((a, b) =>
    a.emp.name.localeCompare(b.emp.name, "ar"),
  );
}

/** سعر ساعة الموظف: الخاص به إن وُجد، وإلا سعر الساعة العام من الإعدادات. */
export const empHourlyRate = (e: Employee, settingsRate: number) =>
  e.hourly_rate > 0 ? e.hourly_rate : settingsRate;

/** أجر ساعات إضافية لموظف: الساعات × سعر الساعة × مضاعف الإضافي. */
export const otPay = (otMin: number, rate: number, mult: number) =>
  (otMin / 60) * rate * mult;

/** دقائق → «س:د» (مثل 125 → 2:05). */
export const hm = (m: number) =>
  m ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}` : "—";
