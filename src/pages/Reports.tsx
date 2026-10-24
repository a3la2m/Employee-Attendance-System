import { useEffect, useMemo, useState } from "react";
import {
  getPayroll,
  getDailyReport,
  getMonthDays,
  getSettings,
  listEmployees,
  listShifts,
  readLogoDataUri,
  salaryPayments,
} from "../lib/api";
import type {
  PayrollResult,
  DailyRow,
  AttendanceDay,
  Employee,
  Shift,
  Settings,
  SalaryPayment,
} from "../lib/types";
import { PrintHeader, PrintFooter } from "../components/PrintChrome";
import { printCss } from "../lib/print";
import { useToday, monthLocal } from "../lib/useToday";
import { dayStatusText, workedOnHoliday } from "../lib/labels";
import {
  buildEmpStats,
  empHourlyRate,
  otPay,
  hm,
} from "../lib/reportStats";

type Toast = (m: string, t?: "ok" | "err") => void;

const n = (v: number) =>
  v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
/** ساعات (بمنزلة واحدة عند الحاجة). */
const n1 = (v: number) =>
  v.toLocaleString("en-US", { maximumFractionDigits: 1 });
const tfmt = (iso: string | null) => (iso ? (iso.split("T")[1] || "").slice(0, 5) : "—");

/** أنواع التقارير المتاحة في الصفحة. */
type ReportKey =
  | "payroll"
  | "payments"
  | "daily"
  | "absence"
  | "late"
  | "overtime"
  | "leaves"
  | "staff";

const REPORTS: {
  key: ReportKey;
  label: string;
  icon: string;
  title: string;
  monthly: boolean;
}[] = [
  { key: "payroll", label: "كشف الرواتب الشهري", icon: "💰", title: "كشف الرواتب الشهري", monthly: true },
  { key: "payments", label: "المدفوعات", icon: "💵", title: "تقرير المدفوعات الشهرية", monthly: true },
  { key: "daily", label: "الحضور والانصراف اليومي", icon: "🗓️", title: "تقرير الحضور والانصراف اليومي", monthly: false },
  { key: "absence", label: "الغياب", icon: "❌", title: "تقرير أيام الغياب", monthly: true },
  { key: "late", label: "التأخير والانصراف المبكر", icon: "⏰", title: "تقرير التأخير والانصراف المبكر", monthly: true },
  { key: "overtime", label: "الساعات الإضافية", icon: "⏱️", title: "تقرير الساعات الإضافية", monthly: true },
  { key: "leaves", label: "الإجازات والعطل", icon: "🏖️", title: "تقرير الإجازات والعطل", monthly: true },
  { key: "staff", label: "الموظفون", icon: "👥", title: "قائمة الموظفين والورديات", monthly: false },
];

/** حالة اليوم في التقرير اليومي — نص عربي موحّد. */
const dailyStatus = (r: DailyRow): string => {
  if (r.status === "leave") return dayStatusText(r);
  if (r.status === "holiday") return "عطلة رسمية";
  if (r.status === "weekend") return "عطلة أسبوعية";
  if (r.absent) return "غياب";
  if (r.status === "incomplete") return "بصمة ناقصة";
  if (r.first_in)
    return r.last_out
      ? `حضور${workedOnHoliday(r) ? " (عطلة رسمية)" : ""}`
      : "حضور (قيد العمل)";
  return "—";
};

const statKinds: ReportKey[] = ["absence", "late", "overtime", "leaves"];

export default function Reports({ toast }: { toast: Toast }) {
  const { today: todayIso, month, rolledFrom } = useToday();
  const [kind, setKind] = useState<ReportKey>("payroll");
  const [period, setPeriod] = useState(month);
  const [date, setDate] = useState(todayIso);
  const [payroll, setPayroll] = useState<PayrollResult[]>([]);
  const [pays, setPays] = useState<SalaryPayment[]>([]);
  const [daily, setDaily] = useState<DailyRow[]>([]);
  const [days, setDays] = useState<AttendanceDay[]>([]);
  const [emps, setEmps] = useState<Employee[]>([]);
  const [shifts, setShifts] = useState<Shift[]>([]);
  const [cfg, setCfg] = useState<Settings>({});
  const [logo, setLogo] = useState<string>("");

  const meta = REPORTS.find((r) => r.key === kind) ?? REPORTS[0];

  // عند تبدّل اليوم/الشهر والتطبيق مفتوح: رحّل التاريخ المعروض إن كان مثبَّتاً
  // على اليوم/الشهر المنصرم.
  useEffect(() => {
    if (!rolledFrom) return;
    setDate((d) => (d === rolledFrom ? todayIso : d));
    setPeriod((p) => (p === monthLocal(new Date(rolledFrom)) ? month : p));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rolledFrom, todayIso, month]);

  useEffect(() => {
    getSettings()
      .then((s) => {
        setCfg(s);
        if (s.company_logo) {
          readLogoDataUri(s.company_logo)
            .then(setLogo)
            .catch(() => setLogo(""));
        }
      })
      .catch(() => {});
    listEmployees().then(setEmps).catch(() => {});
  }, []);

  useEffect(() => {
    if (kind === "payroll") getPayroll(period).then(setPayroll).catch(() => {});
    else if (kind === "payments") {
      // دفتر المدفوعات: كشف الشهر (للصافي والغير مدفوع) + دفعاته الفعلية
      getPayroll(period).then(setPayroll).catch(() => {});
      salaryPayments(period).then(setPays).catch((e) => toast(String(e), "err"));
    } else if (kind === "daily")
      getDailyReport(date).then(setDaily).catch(() => {});
    else if (kind === "staff") listShifts().then(setShifts).catch(() => {});
    else getMonthDays(period).then(setDays).catch((e) => toast(String(e), "err"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, period, date]);

  // إحصاءات التقارير الشهرية (غياب/تأخير/إضافي/إجازات) — للمنشط منها فقط
  const stats = useMemo(
    () => (statKinds.includes(kind) ? buildEmpStats(days, emps) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [kind, days, emps],
  );

  const rate = Number(cfg.hourly_rate ?? 10);
  const mult = Number(cfg.overtime_rate_mult ?? 1.5);
  const curSym = cfg.currency === "USD" ? "$" : cfg.currency === "JOD" ? "د.أ" : "₪";
  const defaultShift = shifts.find((s) => s.is_default);
  const shiftName = (e: Employee) =>
    e.shift_id == null
      ? defaultShift?.name || "—"
      : shifts.find((s) => s.id === e.shift_id)?.name || "—";

  // تنقّل بين الأشهر/الأيام — موحّد مع شاشتي الرواتب والحضور
  const shiftMonth = (delta: number) => {
    const [y, m] = period.split("-").map(Number);
    const d = new Date(y, m - 1 + delta, 1);
    setPeriod(`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`);
  };
  const shiftDay = (delta: number) => {
    const d = new Date(`${date}T00:00:00`);
    if (Number.isNaN(d.getTime())) return;
    d.setDate(d.getDate() + delta);
    setDate(
      `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`,
    );
  };

  const totalNet = payroll.reduce((a, r) => a + r.net_pay, 0);

  // دفتر المدفوعات لشهر العرض: من دُفع فعلاً ومن لم يُدفع + تفصيل حسب الطريقة
  const payOf = (empId: number) =>
    pays.find((p) => p.employee_id === empId) ?? null;
  const paidSum = pays.reduce((a, p) => a + p.net, 0);
  const paidCount = payroll.filter((r) => payOf(r.employee_id)).length;
  const unpaidNet = payroll
    .filter((r) => !payOf(r.employee_id))
    .reduce((a, r) => a + r.net_pay, 0);
  const methodGroups = (() => {
    const g = new Map<string, { count: number; sum: number }>();
    for (const p of pays) {
      const k = p.method || "غير محددة";
      const cur = g.get(k) ?? { count: 0, sum: 0 };
      cur.count += 1;
      cur.sum += p.net;
      g.set(k, cur);
    }
    return [...g.entries()].sort((a, b) => b[1].sum - a[1].sum);
  })();

  const hasData =
    (kind === "payroll" && payroll.length > 0) ||
    (kind === "payments" && payroll.length > 0) ||
    (kind === "daily" && daily.length > 0) ||
    (kind === "staff" && emps.length > 0) ||
    (statKinds.includes(kind) && stats.length > 0);

  const emptyRow = (cols: number) => (
    <tr>
      <td colSpan={cols} className="text-center text-slate-400">
        لا توجد بيانات لهذه الفترة.
      </td>
    </tr>
  );

  return (
    <div className="p-6 space-y-5">
      <style>{printCss}</style>

      {/* ===== شريط العنوان ونوع التقرير ===== */}
      <div className="print:hidden">
        <div className="flex items-center justify-between gap-3 flex-wrap mb-3">
          <h1 className="text-2xl font-extrabold">تقارير</h1>
          <button
            className="btn-primary"
            onClick={() =>
              hasData ? window.print() : toast("لا توجد بيانات لهذه الفترة", "err")
            }
          >
            🖨️ طباعة / حفظ PDF
          </button>
        </div>
        <div className="flex flex-wrap gap-2">
          {REPORTS.map((r) => (
            <button
              key={r.key}
              className={`px-3 py-1.5 rounded-full text-sm font-bold border transition ${
                kind === r.key
                  ? "bg-indigo-600 text-white border-indigo-600"
                  : "bg-white border-slate-300 text-slate-600 hover:border-indigo-400"
              }`}
              onClick={() => setKind(r.key)}
            >
              {r.icon} {r.label}
            </button>
          ))}
        </div>
      </div>

      {/* ===== أدوات الفترة (شهر/يوم) ===== */}
      {kind !== "staff" && (
        <div className="flex items-center gap-2 print:hidden">
          {meta.monthly ? (
            <>
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none"
                onClick={() => shiftMonth(-1)}
                title="الشهر السابق"
                aria-label="الشهر السابق"
              >
                ‹
              </button>
              <input
                type="month"
                className="input !w-44"
                value={period}
                onChange={(e) => e.target.value && setPeriod(e.target.value)}
              />
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
                onClick={() => shiftMonth(1)}
                disabled={period >= month}
                title="الشهر التالي"
                aria-label="الشهر التالي"
              >
                ›
              </button>
            </>
          ) : (
            <>
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none"
                onClick={() => shiftDay(-1)}
                title="اليوم السابق"
                aria-label="اليوم السابق"
              >
                ‹
              </button>
              <input
                type="date"
                className="input !w-44"
                value={date}
                max={todayIso}
                onChange={(e) => e.target.value && setDate(e.target.value)}
              />
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
                onClick={() => shiftDay(1)}
                disabled={date >= todayIso}
                title="اليوم التالي"
                aria-label="اليوم التالي"
              >
                ›
              </button>
            </>
          )}
        </div>
      )}

      {/* ===== ورقة التقرير القابلة للطباعة ===== */}
      <div className="report-sheet bg-white mx-auto w-full max-w-[820px] p-8 shadow">
        <PrintHeader
          cfg={cfg}
          logo={logo}
          title={meta.title}
          meta={
            <>
              <div className="text-slate-600">
                {meta.monthly
                  ? `الفترة: ${period}`
                  : kind === "staff"
                    ? `عدد الموظفين: ${emps.length}`
                    : `التاريخ: ${date}`}
              </div>
              {kind === "payments" && (
                <div className="text-slate-600">
                  المدفوع: {n(paidSum)} · غير المدفوع: {n(unpaidNet)} {curSym} ·
                  عدد الدفعات: {pays.length}
                </div>
              )}
              <div className="text-slate-500 text-xs">تاريخ الطباعة: {todayIso}</div>
            </>
          }
        />

        {/* 1) كشف الرواتب الشهري */}
        {kind === "payroll" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>أيام</th>
                <th>ساعات</th>
                <th>الأساسي</th>
                <th>البدلات</th>
                <th>العطلة الأسبوعية</th>
                <th>عمل عطلة/بدل</th>
                <th>الصافي</th>
              </tr>
            </thead>
            <tbody>
              {payroll.map((r, i) => {
                const holidayExtra =
                  (r.holiday_work_pay ?? 0) + (r.comp_off_pay ?? 0);
                return (
                  <tr key={r.employee_id}>
                    <td>{i + 1}</td>
                    <td className="font-medium">{r.employee_name}</td>
                    <td>{r.days_present}</td>
                    <td>{n(r.total_worked_hours)}</td>
                    <td>{n(r.base_pay)}</td>
                    <td>{n(r.allowances)}</td>
                    <td>{r.weekend_pay ? n(r.weekend_pay) : "—"}</td>
                    <td>
                      {holidayExtra
                        ? `${n(holidayExtra)}${
                            (r.comp_off_days ?? 0) > 0
                              ? ` (${r.comp_off_days} بدل)`
                              : ""
                          }`
                        : "—"}
                    </td>
                    <td className="font-bold">{n(r.net_pay)}</td>
                  </tr>
                );
              })}
              {payroll.length > 0 && (
                <tr className="total-row">
                  <td colSpan={8} className="font-extrabold">
                    الإجمالي العام
                  </td>
                  <td className="font-extrabold">{n(totalNet)}</td>
                </tr>
              )}
              {payroll.length === 0 && emptyRow(9)}
            </tbody>
          </table>
        )}

        {/* 1ب) دفتر المدفوعات الشهرية — من دُفع فعلاً ومن لم يُدفع */}
        {kind === "payments" && (
          <>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-5 text-center">
              {[
                ["إجمالي الكشف", `${n(totalNet)} ${curSym}`, "text-slate-900"],
                ["المدفوع", `${n(paidSum)} ${curSym}`, "text-emerald-700"],
                ["غير المدفوع", `${n(unpaidNet)} ${curSym}`, "text-rose-600"],
                [
                  "الموظفون: مدفوع / الكلي",
                  `${paidCount} / ${payroll.length}`,
                  "text-indigo-700",
                ],
              ].map(([k, v, cls]) => (
                <div key={k} className="rounded-lg border border-slate-300 p-3">
                  <div className="text-[11px] text-slate-500">{k}</div>
                  <div className={`font-extrabold ${cls}`}>{v}</div>
                </div>
              ))}
            </div>

            <table className="w-full report-table">
              <thead>
                <tr>
                  <th>#</th>
                  <th>الموظف</th>
                  <th>رقم البصمة</th>
                  <th>الصافي المحسوب</th>
                  <th>الحالة</th>
                  <th>المبلغ المدفوع</th>
                  <th>الطريقة</th>
                  <th>تاريخ الدفع</th>
                  <th>ملاحظة</th>
                </tr>
              </thead>
              <tbody>
                {payroll.map((r, i) => {
                  const p = payOf(r.employee_id);
                  return (
                    <tr key={r.employee_id}>
                      <td>{i + 1}</td>
                      <td className="font-medium">{r.employee_name}</td>
                      <td>{r.pin || "—"}</td>
                      <td>{n(r.net_pay)}</td>
                      <td>
                        {p ? (
                          <span className="font-bold text-emerald-700">
                            مدفوع ✓
                          </span>
                        ) : (
                          <span className="font-bold text-rose-600">
                            غير مدفوع
                          </span>
                        )}
                      </td>
                      <td className="font-bold">{p ? n(p.net) : "—"}</td>
                      <td>{p?.method ?? "—"}</td>
                      <td className="font-mono text-xs">
                        {p ? p.paid_at.slice(0, 10) : "—"}
                      </td>
                      <td className="text-xs">{p?.note ?? "—"}</td>
                    </tr>
                  );
                })}
                {payroll.length > 0 && (
                  <tr className="total-row">
                    <td colSpan={3} className="font-extrabold">
                      الإجمالي
                    </td>
                    <td className="font-extrabold">{n(totalNet)}</td>
                    <td>
                      {paidCount}/{payroll.length}
                    </td>
                    <td className="font-extrabold">{n(paidSum)}</td>
                    <td colSpan={3} />
                  </tr>
                )}
                {payroll.length === 0 && emptyRow(9)}
              </tbody>
            </table>

            {/* تفصيل حسب طريقة الدفع */}
            {pays.length > 0 && (
              <table className="w-full report-table mt-6">
                <thead>
                  <tr>
                    <th>تفصيل حسب طريقة الدفع</th>
                    <th>عدد الدفعات</th>
                    <th>الإجمالي ({curSym})</th>
                  </tr>
                </thead>
                <tbody>
                  {methodGroups.map(([k, g]) => (
                    <tr key={k}>
                      <td className="font-medium">{k}</td>
                      <td>{g.count}</td>
                      <td className="font-bold">{n(g.sum)}</td>
                    </tr>
                  ))}
                  <tr className="total-row">
                    <td className="font-extrabold">الإجمالي</td>
                    <td className="font-extrabold">{pays.length}</td>
                    <td className="font-extrabold">{n(paidSum)}</td>
                  </tr>
                </tbody>
              </table>
            )}
          </>
        )}

        {/* 2) الحضور والانصراف اليومي */}
        {kind === "daily" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>الحضور</th>
                <th>الانصراف</th>
                <th>الفعلي</th>
                <th>التأخير</th>
                <th>مبكر</th>
                <th>الحالة</th>
              </tr>
            </thead>
            <tbody>
              {daily.map((r, i) => (
                <tr key={r.id}>
                  <td>{i + 1}</td>
                  <td className="font-medium">{r.name || `#${r.pin}`}</td>
                  <td>{r.pin}</td>
                  <td>{tfmt(r.first_in)}</td>
                  <td>{tfmt(r.last_out)}</td>
                  <td>{hm(r.worked_min)}</td>
                  <td>{hm(r.late_min)}</td>
                  <td>{hm(r.early_leave_min)}</td>
                  <td>{dailyStatus(r)}</td>
                </tr>
              ))}
              {daily.length === 0 && emptyRow(9)}
            </tbody>
          </table>
        )}

        {/* 3) الغياب */}
        {kind === "absence" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>أيام الغياب</th>
                <th>التواريخ</th>
              </tr>
            </thead>
            <tbody>
              {stats
                .filter((s) => s.absentDates.length > 0)
                .map((s, i) => (
                  <tr key={s.emp.id}>
                    <td>{i + 1}</td>
                    <td className="font-medium">{s.emp.name}</td>
                    <td>{s.emp.pin || "—"}</td>
                    <td className="font-bold">{s.absentDates.length}</td>
                    <td className="text-right">
                      <span className="font-mono text-xs">
                        {s.absentDates.map((d) => d.slice(5)).join("، ")}
                      </span>
                    </td>
                  </tr>
                ))}
              {stats.every((s) => s.absentDates.length === 0) && emptyRow(5)}
            </tbody>
          </table>
        )}

        {/* 4) التأخير والانصراف المبكر */}
        {kind === "late" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>أيام التأخير</th>
                <th>إجمالي التأخير</th>
                <th>أطول تأخير</th>
                <th>أيام مبكر</th>
                <th>إجمالي المبكر</th>
              </tr>
            </thead>
            <tbody>
              {stats
                .filter((s) => s.lateDays > 0 || s.earlyDays > 0)
                .map((s, i) => (
                  <tr key={s.emp.id}>
                    <td>{i + 1}</td>
                    <td className="font-medium">{s.emp.name}</td>
                    <td>{s.emp.pin || "—"}</td>
                    <td>{s.lateDays}</td>
                    <td>{hm(s.lateMin)}</td>
                    <td>{hm(s.worstLateMin)}</td>
                    <td>{s.earlyDays}</td>
                    <td>{hm(s.earlyMin)}</td>
                  </tr>
                ))}
              {stats.every((s) => s.lateDays === 0 && s.earlyDays === 0) &&
                emptyRow(8)}
            </tbody>
          </table>
        )}

        {/* 5) الساعات الإضافية */}
        {kind === "overtime" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>ساعات الإضافي</th>
                <th>سعر الساعة</th>
                <th>المضاعف</th>
                <th>الأجر المستحق</th>
              </tr>
            </thead>
            <tbody>
              {stats
                .filter((s) => s.otMin > 0)
                .map((s, i) => {
                  const r = empHourlyRate(s.emp, rate);
                  return (
                    <tr key={s.emp.id}>
                      <td>{i + 1}</td>
                      <td className="font-medium">{s.emp.name}</td>
                      <td>{s.emp.pin || "—"}</td>
                      <td>{n1(s.otMin / 60)}</td>
                      <td>{n(r)}</td>
                      <td>{n(mult)}</td>
                      <td className="font-bold">{n(otPay(s.otMin, r, mult))}</td>
                    </tr>
                  );
                })}
              {(() => {
                const rows = stats.filter((s) => s.otMin > 0);
                if (rows.length === 0) return emptyRow(7);
                const total = rows.reduce(
                  (a, s) => a + otPay(s.otMin, empHourlyRate(s.emp, rate), mult),
                  0,
                );
                return (
                  <tr className="total-row">
                    <td colSpan={3} className="font-extrabold">
                      الإجمالي
                    </td>
                    <td className="font-extrabold">
                      {n1(rows.reduce((a, s) => a + s.otMin, 0) / 60)}
                    </td>
                    <td colSpan={2} />
                    <td className="font-extrabold">{n(total)}</td>
                  </tr>
                );
              })()}
            </tbody>
          </table>
        )}

        {/* 6) الإجازات والعطل */}
        {kind === "leaves" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>إجازات مدفوعة</th>
                <th>إجازات غير مدفوعة</th>
                <th>عطل تعويضية</th>
                <th>دوام عطلة رسمية</th>
              </tr>
            </thead>
            <tbody>
              {stats
                .filter(
                  (s) =>
                    s.paidLeaveDays + s.unpaidLeaveDays + s.compDays + s.holidayWorkDays >
                    0,
                )
                .map((s, i) => (
                  <tr key={s.emp.id}>
                    <td>{i + 1}</td>
                    <td className="font-medium">{s.emp.name}</td>
                    <td>{s.emp.pin || "—"}</td>
                    <td>{s.paidLeaveDays}</td>
                    <td>{s.unpaidLeaveDays}</td>
                    <td>{s.compDays}</td>
                    <td>{s.holidayWorkDays}</td>
                  </tr>
                ))}
              {stats.every(
                (s) =>
                  s.paidLeaveDays + s.unpaidLeaveDays + s.compDays + s.holidayWorkDays ===
                  0,
              ) && emptyRow(7)}
            </tbody>
          </table>
        )}

        {/* 7) قائمة الموظفين */}
        {kind === "staff" && (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>البصمة</th>
                <th>الاسم</th>
                <th>الوظيفة</th>
                <th>الوردية</th>
                <th>ساعات الدوام</th>
                <th>الاستراحة</th>
                <th>سعر الساعة</th>
                <th>البدلات</th>
                <th>الحالة</th>
              </tr>
            </thead>
            <tbody>
              {emps
                .slice()
                .sort((a, b) => a.name.localeCompare(b.name, "ar"))
                .map((e, i) => (
                  <tr key={e.id}>
                    <td>{i + 1}</td>
                    <td>{e.pin || "—"}</td>
                    <td className="font-medium">{e.name}</td>
                    <td>{e.job_title || "—"}</td>
                    <td>{shiftName(e)}</td>
                    <td>{n1(e.daily_hours)}</td>
                    <td>{hm(e.break_min)}</td>
                    <td>{n(empHourlyRate(e, rate))}</td>
                    <td>
                      {n((e.meal_enabled ? e.meal_allow : 0) + (e.transport_enabled ? e.transport_allow : 0))}
                    </td>
                    <td>
                      {e.active ? (
                        <span className="px-2 py-0.5 rounded-full text-xs font-bold bg-emerald-100 text-emerald-700">
                          نشط
                        </span>
                      ) : (
                        <span className="px-2 py-0.5 rounded-full text-xs font-bold bg-slate-200 text-slate-600">
                          موقوف
                        </span>
                      )}
                    </td>
                  </tr>
                ))}
              {emps.length === 0 && emptyRow(10)}
            </tbody>
          </table>
        )}

        {/* التذييل: توقيع */}
        <footer className="mt-8 pt-6 border-t border-slate-300 grid grid-cols-1 gap-6 text-sm">
          <div className="text-center">
            <div className="h-14 border-b border-dotted border-slate-400" />
            <div className="mt-2 font-bold">توقيع المدير</div>
          </div>
        </footer>

        {/* التذييل الموحد من الإعدادات */}
        <PrintFooter cfg={cfg} />
      </div>
    </div>
  );
}
