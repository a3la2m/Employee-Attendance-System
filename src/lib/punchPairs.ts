/** دمج حركات البصمة في صفوف «دخول ← خروج» لكل يوم — دالات نقية تُختبر بـ vitest.
 *
 * القاعدة: تُمشى حركات اليوم بالترتيب الزمني، فكل دخول يُقابله أول خروج بعده في
 * نفس اليوم ويُعرضان في صف واحد مع المدة بينهما. الحركات بلا مقابل (دخول دون
 * خروج / خروج دون دخول) تظهر في صف مستقل. «الدخول/الخروج» هنا باتجاه الحركة
 * كما يسجّلها الجهاز: حضور/عودة استراحة/إضافي داخلة — انصراف/مغادرة استراحة/
 * إضافي خارجة.
 */
import type { PunchRow } from "./api";

/** صف «دخول ← خروج» واحد في العرض المدمج. */
export interface PunchPairRow {
  /** تاريخ الحركة YYYY-MM-DD. */
  date: string;
  /** وقت الدخول HH:MM:SS أو null إن كان الصف خروجاً فقط. */
  inTime: string | null;
  /** وقت الخروج HH:MM:SS أو null إن كان الصف دخولاً فقط. */
  outTime: string | null;
  /** نوع حركة الدخول كما سجّلها الجهاز (null إن لم يكن هناك دخول). */
  inType: number | null;
  /** نوع حركة الخروج كما سجّلها الجهاز (null إن لم يكن هناك خروج). */
  outType: number | null;
  /** الدقائق بين الدخول والخروج — null إن لم يكتمل الزوج. */
  minutes: number | null;
  /** رمز الجهاز الخاص بالدخول (raw/work) إن وُجد. */
  inDevice: string;
  /** رمز الجهاز الخاص بالخروج (raw/work) إن وُجد. */
  outDevice: string;
}

/** حركة داخلة: حضور (0) / عودة من استراحة (3) / إضافي (4). */
export const isInType = (t: number) => t === 0 || t === 3 || t === 4;
/** حركة خارجة: انصراف (1) / مغادرة استراحة (2) / إضافي (5). */
export const isOutType = (t: number) => t === 1 || t === 2 || t === 5;

/** HH:MM:SS من طابع ISO — أو null عند الغياب. */
const hhmmss = (iso: string | undefined | null): string | null => {
  const t = iso?.split("T")[1];
  return t ? t.slice(0, 8) : null;
};

/** «raw/work» لرمز الجهاز. */
const devCode = (p?: PunchRow) => (p ? `${p.raw_status}/${p.work_code}` : "");

/** بناء صف واحد من حركتين (أو حركة واحدة إن لم يكتمل الزوج). */
const makeRow = (inn?: PunchRow, out?: PunchRow): PunchPairRow => {
  let minutes: number | null = null;
  if (inn && out) {
    const a = Date.parse(inn.device_time);
    const b = Date.parse(out.device_time);
    if (!Number.isNaN(a) && !Number.isNaN(b) && b >= a)
      minutes = Math.round((b - a) / 60000);
  }
  return {
    date: (inn ?? out)!.device_time.slice(0, 10),
    inTime: hhmmss(inn?.device_time),
    outTime: hhmmss(out?.device_time),
    inType: inn?.punch_type ?? null,
    outType: out?.punch_type ?? null,
    minutes,
    inDevice: devCode(inn),
    outDevice: devCode(out),
  };
};

/** دمج الحركات في صفوف دخول/خروج — ترتيب تنازلي (الأحدث أولاً) كالواجهة.
 *
 * داخل كل يوم: كل دخول يقترن بأول خروج بعده؛ دخول سابق بلا خروج يُعرض في صف
 * مستقل، وكذلك خروج بلا دخول. لا يقترن دخول يوم بخروج يوم آخر. */
export function pairPunches(punches: PunchRow[]): PunchPairRow[] {
  const sorted = [...punches].sort((a, b) =>
    a.device_time < b.device_time ? -1 : a.device_time > b.device_time ? 1 : 0,
  );
  const rows: PunchPairRow[] = [];
  let pending: PunchRow | undefined;
  let day = "";
  for (const p of sorted) {
    const d = p.device_time.slice(0, 10);
    if (d !== day) {
      // يوم جديد: دخول معلّق من اليوم السابق يُعرض دون خروج (لا اقتران عبر الأيام)
      if (pending) rows.push(makeRow(pending));
      pending = undefined;
      day = d;
    }
    if (isInType(p.punch_type)) {
      if (pending) rows.push(makeRow(pending)); // دخول سابق بلا خروج
      pending = p;
    } else if (isOutType(p.punch_type)) {
      if (pending) {
        rows.push(makeRow(pending, p));
        pending = undefined;
      } else {
        rows.push(makeRow(undefined, p)); // خروج بلا دخول
      }
    }
    // أنواع حركة غير معروفة تُترك كما هي (لا تدخل العرض المدمج)
  }
  if (pending) rows.push(makeRow(pending));
  return rows.reverse(); // الأحدث أولاً
}

/** مدة الدقائق بالعربية: «4 س 30 د» / «45 د» / «—». */
export const fmtDur = (min: number | null): string => {
  if (min == null || Number.isNaN(min)) return "—";
  const m = Math.round(min);
  const h = Math.floor(m / 60);
  const r = Math.abs(m % 60);
  if (h === 0) return `${r} د`;
  if (r === 0) return `${h} س`;
  return `${h} س ${r} د`;
};
