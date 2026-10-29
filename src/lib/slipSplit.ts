/** تقسيم قسيمة الراتب للطباعة فقط — «إعدادات الطباعة لقسيمة الراتب».
 *
 * الفكرة: بعد احتساب الرواتب كما هي (لا تغيير في الحساب إطلاقاً)، تُعاد صياغة
 * القسيمة **المطبوعة** قانونياً:
 * - الأساسي يُعاد للحد الأدنى: ساعات الدوام × سعر ساعة قابل للتعديل (افتراض 8.5).
 * - العمل الإضافي: ساعات الإضافي × المعامل (افتراض 1.5) × سعر الساعة نفسه.
 * - باقي المبلغ المستحق يُقسَّم على ثلاثة بدلات مسمّاة بنسبة لكل بدل
 *   (مجموع النِسب يجب أن يساوي 100% وإلا لا يُطبَّق التقسيم).
 * - بدلات الفطور والمواصلات وبدل العطلة ودوام العطلة الرسمية والأيام التعويضية
 *   تُعرض كما هي، والصافي المستحق لا يتغيّر إطلاقاً.
 */
import type { AttendanceDay, PayrollResult, Settings } from "./types";

/** مفاتيح إعدادات التقسيم في جدول settings. */
export const SLIP_KEYS = {
  enabled: "slip_print_enabled",
  rate: "slip_hourly_rate",
  otMult: "slip_ot_mult",
  pWork: "slip_share_work",
  pRisk: "slip_share_risk",
  pBonus: "slip_share_bonus",
} as const;

/** إعدادات تقسيم قسيمة الراتب للطباعة. */
export interface SlipCfg {
  enabled: boolean;
  /** سعر الساعة الأساسي المطبوع — الحد الأدنى (افتراض 8.5). */
  rate: number;
  /** معامل ساعة العمل الإضافي (افتراض 1.5). */
  otMult: number;
  /** نِسب البدلات الثلاثة من المبلغ المتبقي — مجموعها 100%. */
  pWork: number;
  pRisk: number;
  pBonus: number;
}

export const SLIP_DEFAULTS: SlipCfg = {
  enabled: false,
  rate: 8.5,
  otMult: 1.5,
  pWork: 45,
  pRisk: 45,
  pBonus: 10,
};

/** الأرقام المعدّلة في النموذج (حقول مفتاحية — لا "enabled"). */
export type SlipNumKey = "rate" | "otMult" | "pWork" | "pRisk" | "pBonus";

/** تقريب لمنزلتين عشريتين (المبالغ المطبوعة). */
export const r2 = (x: number) => Math.round(x * 100) / 100;

const num = (v: string | undefined, fb: number): number => {
  if (v === undefined || v.trim() === "") return fb;
  const x = Number(v);
  return Number.isFinite(x) && x >= 0 ? x : fb;
};

/** قراءة الإعدادات من خريطة settings — بقيم افتراضية عند الغياب أو الخلل. */
export function readSlipCfg(cfg: Settings): SlipCfg {
  return {
    enabled: cfg[SLIP_KEYS.enabled] === "1",
    rate: num(cfg[SLIP_KEYS.rate], SLIP_DEFAULTS.rate),
    otMult: num(cfg[SLIP_KEYS.otMult], SLIP_DEFAULTS.otMult),
    pWork: num(cfg[SLIP_KEYS.pWork], SLIP_DEFAULTS.pWork),
    pRisk: num(cfg[SLIP_KEYS.pRisk], SLIP_DEFAULTS.pRisk),
    pBonus: num(cfg[SLIP_KEYS.pBonus], SLIP_DEFAULTS.pBonus),
  };
}

/** تحويل الإعدادات إلى حقول تُحفظ دفعة واحدة في settings. */
export function slipToSettings(s: SlipCfg): Record<string, string> {
  return {
    [SLIP_KEYS.enabled]: s.enabled ? "1" : "0",
    [SLIP_KEYS.rate]: String(s.rate),
    [SLIP_KEYS.otMult]: String(s.otMult),
    [SLIP_KEYS.pWork]: String(s.pWork),
    [SLIP_KEYS.pRisk]: String(s.pRisk),
    [SLIP_KEYS.pBonus]: String(s.pBonus),
  };
}

/** مجموع نِسب البدلات الثلاثة (يجب 100%). */
export const slipPctSum = (s: SlipCfg) => r2(s.pWork + s.pRisk + s.pBonus);

/** هل المجموع = 100%؟ وإلا لا يُطبَّق التقسيم ولا تُطبع القسيمة مقسّمة. */
export const slipSumOk = (s: SlipCfg) =>
  Math.abs(s.pWork + s.pRisk + s.pBonus - 100) <= 0.005;

/** ساعات دوام العطل الرسمية (الاختيار النقدي) في الفترة.
 *
 * تُدفع ببدل العطلة (62 + ساعات×1.5) وتُستثنى من أجر الدوام العادي في
 * `compute_payroll`، فلا تدخل في «الأساسي» المطبوع ولا في «الإضافي».
 * مطابقة لمنطق payroll.rs: يوم عليه عطلة رسمية، له بصمة، والاختيار ليس «comp».
 */
export function holidayHoursOf(days: AttendanceDay[]): number {
  let min = 0;
  for (const d of days) {
    if (!d.holiday || !d.first_in || d.absent) continue;
    if (d.status === "leave" || d.status === "holiday" || d.status === "weekend")
      continue;
    if (d.holiday_work_choice === "comp") continue;
    min += d.worked_min;
  }
  return min / 60;
}

/** نتيجة تقسيم القسيمة — مجموع الأسطر يساوي الصافي المستحق بالضبط. */
export interface SlipSplit {
  /** ساعات الدوام العادي (بدون الإضافي وبدون ساعات العطل الرسمية). */
  regularHours: number;
  /** الأساسي المطبوع: ساعات الدوام × سعر الساعة. */
  basic: number;
  /** الإضافي المطبوع: ساعات الإضافي × المعامل × سعر الساعة. */
  ot: number;
  /** مجموع بقية الأسطر (فطور+مواصلات + بدل عطلة + عطلة رسمية + تعويضية). */
  others: number;
  /** المتبقي الذي يُقسَّم على البدلات الثلاثة. */
  pool: number;
  shareWork: number;
  shareRisk: number;
  shareBonus: number;
}

/** حساب تقسيم القسيمة — null إذا كان الميزة ملغاة أو النِسب ≠ 100%.
 *
 * الضمانة القانونية: (الأساسي + الإضافي + البدلات الحالية + النِسب الثلاثة)
 * = الصافي المستحق بالضبط، ولا تُعرض نِيب سالبة أبداً حتى لو تجاوز سعر الساعة
 * المطبوع الأجر الفعلي للموظف (يُحدَّد الأساسي بما يتبقّى من الصافي).
 */
export function splitSlip(
  sel: PayrollResult,
  days: AttendanceDay[],
  s: SlipCfg,
): SlipSplit | null {
  if (!s.enabled || !slipSumOk(s)) return null;

  const regularHours = Math.max(
    0,
    sel.total_worked_hours - sel.overtime_hours - holidayHoursOf(days),
  );
  // بقية الأسطر المعروضة كما هي — تُطرح من الإجمالي قبل تقسيم الباقي
  // ملاحظة: الصافي المستحق = الإجمالي − خصم السلفة (إن وُجد)، لذا فإن الإجمالي قبل خصم السلفة = net_pay + advance_deduction
  const advDed = r2(sel.advance_deduction ?? 0);
  const grossNet = r2(sel.net_pay + advDed);
  const others = r2(
    sel.allowances +
      sel.weekend_pay +
      (sel.holiday_work_pay ?? 0) +
      (sel.comp_off_pay ?? 0),
  );
  const avail = r2(Math.max(0, grossNet - others));

  // الإضافي أولاً (حقوق مستحقة بالساعات)، ثم الأساسي بحد أقصى ما يتبقّى
  const ot = r2(
    Math.min(Math.max(0, sel.overtime_hours * s.rate * s.otMult), avail),
  );
  const basic = r2(
    Math.min(Math.max(0, regularHours * s.rate), avail - ot),
  );
  const pool = r2(avail - ot - basic);

  const shareWork = r2((pool * s.pWork) / 100);
  const shareBonus = r2((pool * s.pBonus) / 100);
  // الحصة المتبقية للبدل الثاني حتى يتطابق مجموع الأسطر مع الصافي تماماً
  const shareRisk = r2(pool - shareWork - shareBonus);

  return {
    regularHours,
    basic,
    ot,
    others,
    pool,
    shareWork,
    shareRisk,
    shareBonus,
  };
}
