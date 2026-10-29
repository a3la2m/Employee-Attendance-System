import { describe, it, expect } from "vitest";
import {
  buildEmpStats,
  empHourlyRate,
  otPay,
  hm,
} from "../reportStats";
import type { AttendanceDay, Employee } from "../types";

const emp = (over: Partial<Employee> = {}): Employee => ({
  id: 1,
  pin: "1",
  name: "أحمد",
  name_en: "ahmad",
  job_title: "حرفي",
  phone: "",
  daily_hours: 8.5,
  break_min: 30,
  hourly_rate: 0,
  meal_allow: 7,
  transport_allow: 5,
  active: true,
  overtime_enabled: true,
  meal_enabled: true,
  transport_enabled: true,
  weekend_paid: true,
  annual_leave_balance: 14,
  annual_leave_days: 14,
  leave_accrual_month: null,
  shift_id: null,
  device_privilege: 0,
  bot_allowed: true,
  ...over,
});

const day = (over: Partial<AttendanceDay> = {}): AttendanceDay => ({
  id: 1,
  employee_id: 1,
  date: "2026-10-22",
  first_in: "2026-10-22T06:00:00",
  last_out: "2026-10-22T16:30:00",
  worked_min: 600,
  late_min: 0,
  early_leave_min: 0,
  overtime_min: 120,
  absent: false,
  leave_type: null,
  status: "present",
  leave_paid: null,
  holiday: false,
  holiday_work_choice: null,
  ...over,
});

describe("buildEmpStats", () => {
  it("يجمع الغياب والتأخير والمبكر والإضافي لموظف واحد", () => {
    const days = [
      day(),
      day({ id: 2, date: "2026-10-23", late_min: 45, early_leave_min: 30 }),
      day({
        id: 3,
        date: "2026-10-24",
        status: "absent",
        absent: true,
        first_in: null,
        last_out: null,
        worked_min: 0,
        overtime_min: 0,
      }),
      day({
        id: 4,
        date: "2026-10-25",
        status: "absent",
        absent: true,
        first_in: null,
        worked_min: 0,
        overtime_min: 0,
      }),
    ];
    const [s] = buildEmpStats(days, [emp()]);
    expect(s.absentDates).toEqual(["2026-10-24", "2026-10-25"]);
    expect(s.lateDays).toBe(1);
    expect(s.lateMin).toBe(45);
    expect(s.worstLateMin).toBe(45);
    expect(s.earlyDays).toBe(1);
    expect(s.earlyMin).toBe(30);
    expect(s.otMin).toBe(240);
  });

  it("يميّز الإجازات: مدفوعة وغير مدفوعة وتعويضية", () => {
    const days = [
      day({ id: 1, status: "leave", leave_type: "annual", leave_paid: true, first_in: null, overtime_min: 0 }),
      day({ id: 2, status: "leave", leave_type: "sick", leave_paid: false, first_in: null, overtime_min: 0 }),
      day({ id: 3, status: "leave", leave_type: "comp", leave_paid: true, first_in: null, overtime_min: 0 }),
    ];
    const [s] = buildEmpStats(days, [emp()]);
    expect(s.paidLeaveDays).toBe(1);
    expect(s.unpaidLeaveDays).toBe(1);
    expect(s.compDays).toBe(1);
  });

  it("دوام العطلة الرسمية يُعدّ — وخيار «بدل» لا يُعدّ", () => {
    const days = [
      day({ id: 1, holiday: true }),
      day({ id: 2, holiday: true, holiday_work_choice: "comp" }),
      day({ id: 3, status: "holiday", holiday: true, first_in: null, overtime_min: 0 }),
    ];
    const [s] = buildEmpStats(days, [emp()]);
    expect(s.holidayWorkDays).toBe(1);
  });

  it("عطلة أسبوعية بلا بصمة لا تدخل الإحصاءات", () => {
    const days = [
      day({ id: 1, status: "weekend", first_in: null, overtime_min: 0 }),
    ];
    const stats = buildEmpStats(days, [emp()]);
    expect(stats).toHaveLength(1);
    expect(stats[0].lateDays).toBe(0);
    expect(stats[0].otMin).toBe(0);
  });

  it("عدة موظفين بالترتيب الأبجدي وبلا صفوف لمن لا بيانات", () => {
    const emps = [emp({ id: 1, name: "سامي" }), emp({ id: 2, name: "أحمد" })];
    const days = [day({ employee_id: 2 }), day({ id: 2, employee_id: 2 })];
    const stats = buildEmpStats(days, emps);
    expect(stats).toHaveLength(1);
    expect(stats[0].emp.name).toBe("أحمد");
    expect(stats[0].otMin).toBe(240);
  });

  it("موظف محذوف (بلا سجل) يُتجاهل ولا يكسر", () => {
    const days = [day({ employee_id: 99 })];
    expect(buildEmpStats(days, [emp()])).toHaveLength(0);
  });
});

describe("أدوات التقارير", () => {
  it("empHourlyRate: الخاص إن وُجد وإلا العام", () => {
    expect(empHourlyRate(emp({ hourly_rate: 15 }), 10)).toBe(15);
    expect(empHourlyRate(emp({ hourly_rate: 0 }), 10)).toBe(10);
  });
  it("otPay: ساعات × سعر × مضاعف", () => {
    expect(otPay(300, 10, 1.5)).toBe(75);
  });
  it("hm: دقائق → س:د", () => {
    expect(hm(125)).toBe("2:05");
    expect(hm(0)).toBe("—");
  });
});
