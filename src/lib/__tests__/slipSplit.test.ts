import { describe, it, expect } from "vitest";
import {
  SLIP_DEFAULTS,
  readSlipCfg,
  slipToSettings,
  slipSumOk,
  slipPctSum,
  holidayHoursOf,
  splitSlip,
  r2,
  type SlipCfg,
} from "../slipSplit";
import type { AttendanceDay, PayrollResult } from "../types";

const cfg = (over: Partial<SlipCfg> = {}): SlipCfg => ({
  ...SLIP_DEFAULTS,
  enabled: true,
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

/** راتب واقعي: 140 ساعة دوام عادي + 50 إضافي بسعر 15 ومضاعف 1.5. */
const res = (over: Partial<PayrollResult> = {}): PayrollResult => ({
  employee_id: 1,
  employee_name: "تجربة",
  pin: "1",
  period: "2026-10",
  days_present: 26,
  total_worked_hours: 190, // 140 عادي + 50 إضافي
  overtime_hours: 50,
  base_pay: 3225, // 140×15 + 50×15×1.5 (الأساسي يشمل الإضافي للعرض)
  allowances: 182, // 7×26
  weekend_pay: 310, // 62×5
  paid_workdays: 26,
  weekend_days_paid: 5,
  paid_leave_days: 0,
  unpaid_leave_days: 0,
  holiday_days: 0,
  late_deduction: 0,
  absence_deduction: 0,
  net_pay: 3717, // 3225 + 182 + 310
  ...over,
});

describe("readSlipCfg / slipToSettings", () => {
  it("قيم افتراضية عند غياب الإعدادات", () => {
    const c = readSlipCfg({});
    expect(c.enabled).toBe(false);
    expect(c.rate).toBe(8.5);
    expect(c.otMult).toBe(1.5);
    expect(c.pWork).toBe(45);
    expect(c.pRisk).toBe(45);
    expect(c.pBonus).toBe(10);
  });
  it("تقرأ القيم المحفوظة", () => {
    const c = readSlipCfg({
      slip_print_enabled: "1",
      slip_hourly_rate: "9.5",
      slip_ot_mult: "2",
      slip_share_work: "40",
      slip_share_risk: "40",
      slip_share_bonus: "20",
    });
    expect(c.enabled).toBe(true);
    expect(c.rate).toBe(9.5);
    expect(c.otMult).toBe(2);
    expect(c).toMatchObject({ pWork: 40, pRisk: 40, pBonus: 20 });
  });
  it("القيم الخاطئة تعود للافتراضي", () => {
    const c = readSlipCfg({ slip_hourly_rate: "abc", slip_ot_mult: "-3" });
    expect(c.rate).toBe(8.5);
    expect(c.otMult).toBe(1.5);
  });
  it("دورة حفظ/قراءة كاملة", () => {
    const c = cfg({ rate: 9.75, otMult: 2, pWork: 50, pRisk: 30, pBonus: 20 });
    expect(readSlipCfg(slipToSettings(c))).toEqual(c);
  });
});

describe("النِسب", () => {
  it("المجموع الصحيح مقبول", () => {
    expect(slipPctSum(cfg())).toBe(100);
    expect(slipSumOk(cfg())).toBe(true);
    expect(slipSumOk(cfg({ pWork: 40, pRisk: 40, pBonus: 20 }))).toBe(true);
  });
  it("135% (45/45/45) مرفوض", () => {
    expect(slipPctSum(cfg({ pBonus: 45 }))).toBe(135);
    expect(slipSumOk(cfg({ pBonus: 45 }))).toBe(false);
  });
});

describe("holidayHoursOf", () => {
  it("ساعات دوام عطلة رسمية (نقدي) تُحتسب", () => {
    const days = [
      day({ holiday: true, worked_min: 300, status: "present" }),
      day({ id: 2, date: "2026-10-23" }),
    ];
    expect(holidayHoursOf(days)).toBe(5);
  });
  it("خيار «بدل» (comp) لا يُحتسب — يدخل الأساسي العادي", () => {
    const days = [
      day({ holiday: true, worked_min: 300, holiday_work_choice: "comp" }),
    ];
    expect(holidayHoursOf(days)).toBe(0);
  });
  it("عطلة بلا بصمة (status=holiday) وإجازة عليها لا تُحتسب", () => {
    const days = [
      day({ holiday: true, first_in: null, last_out: null, worked_min: 0, status: "holiday" }),
      day({ id: 2, holiday: true, first_in: null, worked_min: 0, status: "leave" }),
    ];
    expect(holidayHoursOf(days)).toBe(0);
  });
});

describe("splitSlip", () => {
  it("null عند الإلغاء أو النِسب ≠ 100%", () => {
    expect(splitSlip(res(), [day()], cfg({ enabled: false }))).toBeNull();
    expect(splitSlip(res(), [day()], cfg({ pBonus: 45 }))).toBeNull();
  });
  it("يقسم كما هو مطلوب: 140×8.5 أساسي و50×1.5×8.5 إضافي والباقي على البدلات", () => {
    const s = splitSlip(res(), [day()], cfg())!;
    expect(s.regularHours).toBe(140);
    expect(s.basic).toBe(1190); // 140 × 8.5
    expect(s.ot).toBe(637.5); // 50 × 1.5 × 8.5
    expect(s.others).toBe(492); // 182 + 310
    expect(s.pool).toBe(1397.5); // 3225 − 1190 − 637.5
    expect(s.shareWork).toBe(628.88); // 45%
    expect(s.shareBonus).toBe(139.75); // 10%
    expect(s.shareRisk).toBe(628.87); // الباقي (45%)
  });
  it("مجموع الأسطر = الصافي المستحق بالضبط", () => {
    const s = splitSlip(res(), [day()], cfg())!;
    const total =
      s.basic + s.ot + s.others + s.shareWork + s.shareRisk + s.shareBonus;
    expect(r2(total)).toBe(r2(res().net_pay));
  });
  it("مبالغ مجزّأة (ساعات ليسا من مضاعفات 60) لا تكسر المجموع", () => {
    const rr = res({
      total_worked_hours: 143.45,
      overtime_hours: 0,
      base_pay: 2151.75, // 143.45 × 15
      net_pay: r2(2151.75 + 182 + 310),
    });
    const s = splitSlip(rr, [day()], cfg())!;
    const total =
      s.basic + s.ot + s.others + s.shareWork + s.shareRisk + s.shareBonus;
    expect(r2(total)).toBe(r2(rr.net_pay));
  });
  it("ساعات العطلة الرسمية تخرج من الأساسي ولا تُحتسب مرتين", () => {
    const days = [
      day({ holiday: true, worked_min: 300, overtime_min: 0, status: "present" }),
    ];
    const rr = res({ total_worked_hours: 195 }); // 140 عادي + 50 إضافي + 5 عطلة
    const s = splitSlip(rr, days, cfg())!;
    expect(s.regularHours).toBe(140);
    expect(s.basic).toBe(1190);
  });
  it("لا إضافي: صف الإضافي = صفر والباقي كله على الأساسي والبدلات", () => {
    const rr = res({
      total_worked_hours: 140,
      overtime_hours: 0,
      base_pay: 2100, // 140 × 15
      net_pay: 2592, // 2100 + 182 + 310
    });
    const s = splitSlip(rr, [day({ overtime_min: 0 })], cfg())!;
    expect(s.ot).toBe(0);
    expect(s.basic).toBe(1190);
    expect(s.pool).toBe(910); // 2100 − 1190
  });
  it("سعر ساعة أعلى من الأجر الفعلي: لا نِيب سالبة والمجموع يبقى = الصافي", () => {
    const rr = res({
      total_worked_hours: 100,
      overtime_hours: 0,
      base_pay: 700, // 100 × 7 (أجر فعلي أقل من 8.5)
      allowances: 182,
      weekend_pay: 0,
      net_pay: 882,
    });
    const s = splitSlip(rr, [day({ overtime_min: 0 })], cfg())!;
    expect(s.basic).toBe(700); // محدود بما يتبقّى من الصافي
    expect(s.ot).toBe(0);
    expect(s.pool).toBe(0);
    expect(s.shareWork).toBe(0);
    expect(s.shareRisk).toBe(0);
    expect(s.shareBonus).toBe(0);
    const total =
      s.basic + s.ot + s.others + s.shareWork + s.shareRisk + s.shareBonus;
    expect(r2(total)).toBe(882);
  });
  it("بديل تعويضي (comp) ضمن البدلات المعروضة ولا يدخل التقسيم", () => {
    const rr = res({
      comp_off_days: 2,
      comp_off_pay: 124,
      net_pay: 3841, // 3717 + 124
    });
    const s = splitSlip(rr, [day()], cfg())!;
    expect(s.others).toBe(616); // 182 + 310 + 124
    expect(s.pool).toBe(1397.5); // 3841 − 616 − 1190 − 637.5
  });
});
