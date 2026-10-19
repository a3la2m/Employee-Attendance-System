import { useEffect, useState } from "react";
import {
  getPayroll,
  archivePayroll,
  payrollMonths,
  employeePayrollHistory,
  recomputeDays,
  getSettings,
  getMonthDays,
  listEmployees,
  readLogoDataUri,
} from "../lib/api";
import type {
  EmployeePayrollRun,
  PayrollMonth,
  PayrollResult,
  AttendanceDay,
  Employee,
  Settings,
} from "../lib/types";
import { PrintHeader, PrintFooter } from "../components/PrintChrome";
import { printCss, arabicDay, hhmm, hrs, todayISO } from "../lib/print";

type Toast = (m: string, t?: "ok" | "err") => void;

const now = new Date();
const thisPeriod = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;

const n = (v: number) =>
  v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const AR_MONTHS = [
  "يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو",
  "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر",
];

/** "2026-10" ← "أكتوبر 2026" (للعرض في التنقّل وسجل الموظف). */
const pLabel = (p: string) => {
  const [y, m] = p.split("-");
  return `${AR_MONTHS[Number(m) - 1] ?? m} ${y}`;
};

/** شارة الحصالة: نص + ألوان. التأخير يبقى معلومة في الإشعارات فقط فلا يظهر
 *  في الرواتب (العمل قد يكون في أي ساعة) — اليوم الحاضر يُعرض «حاضر». */
const stInfo = (d: AttendanceDay): [string, string] =>
  d.status === "leave"
    ? [
        `إجازة ${d.leave_type === "sick" ? "مرضية" : "سنوية"}${
          d.leave_paid === false ? " (غير مدفوعة)" : ""
        }`,
        "bg-sky-100 text-sky-700",
      ]
    : d.status === "holiday"
      ? ["عطلة رسمية", "bg-violet-100 text-violet-700"]
      : d.status === "weekend"
        ? ["عطلة أسبوعية", "bg-slate-200 text-slate-700"]
        : d.absent || d.status === "absent"
        ? ["غياب", "bg-rose-100 text-rose-700"]
        : d.status === "incomplete"
        ? ["بصمة ناقصة", "bg-amber-100 text-amber-700"]
        : d.first_in
          ? ["حاضر", "bg-emerald-100 text-emerald-700"]
          : ["—", "text-slate-400"];

export default function Payroll({ toast }: { toast: Toast }) {
  const [period, setPeriod] = useState(thisPeriod);
  const [rows, setRows] = useState<PayrollResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [cfg, setCfg] = useState<Settings>({});
  const [logo, setLogo] = useState("");
  const [emps, setEmps] = useState<Employee[]>([]);
  const [selId, setSelId] = useState<number | "">("");
  const [days, setDays] = useState<AttendanceDay[]>([]);
  const [slipOpen, setSlipOpen] = useState(false);
  // الأشهر المؤرشفة + سجل رواتب موظف بعينه
  const [months, setMonths] = useState<PayrollMonth[]>([]);
  const [histOpen, setHistOpen] = useState(false);
  const [hist, setHist] = useState<EmployeePayrollRun[]>([]);
  const [histName, setHistName] = useState("");
  const [histEmpId, setHistEmpId] = useState<number | "">("");

  const cur = cfg.currency || "ILS";
  const curSym = cur === "USD" ? "$" : cur === "JOD" ? "د.أ" : "₪";
  const isArchived = months.some((m) => m.period === period);

  const loadMonths = () =>
    payrollMonths().then(setMonths).catch(() => {});

  // تنقّل بين الأشهر (لا تتجاوز الشهر الحالي)
  const shiftMonth = (delta: number) => {
    const [y, m] = period.split("-").map(Number);
    const d = new Date(y, m - 1 + delta, 1);
    const p = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
    if (p > thisPeriod) return;
    setPeriod(p);
  };

  // فتح سجل رواتب موظف في كل الأشهر المؤرشفة
  const openHist = async (empId: number) => {
    const emp = emps.find((e) => e.id === empId);
    const inRows = rows.find((r) => r.employee_id === empId);
    setHistEmpId(empId);
    setHistName(emp?.name ?? inRows?.employee_name ?? `#${empId}`);
    setHist([]);
    setHistOpen(true);
    try {
      setHist(await employeePayrollHistory(empId));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const load = async () => {
    try {
      setRows(await getPayroll(period));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    load();
    loadMonths();
    getSettings()
      .then((s) => {
        setCfg(s);
        if (s.company_logo) {
          readLogoDataUri(s.company_logo).then(setLogo).catch(() => setLogo(""));
        }
      })
      .catch(() => {});
    listEmployees().then(setEmps).catch(() => {});
  }, [period]);

  // أيام دوام الموظف المحدّد (حضور/انصراف لكل يوم في الفترة)
  useEffect(() => {
    if (selId === "") {
      setDays([]);
      return;
    }
    getMonthDays(period)
      .then((all) =>
        setDays(
          all
            .filter((d) => d.employee_id === selId)
            .sort((a, b) => a.date.localeCompare(b.date)),
        ),
      )
      .catch((e) => toast(String(e), "err"));
  }, [selId, period]);

  const recalc = async () => {
    setBusy(true);
    const [y, m] = period.split("-").map(Number);
    const last = new Date(y, m, 0).getDate();
    try {
      await recomputeDays(`${period}-01`, `${period}-${String(last).padStart(2, "0")}`);
      await load();
      if (selId !== "") {
        const all = await getMonthDays(period);
        setDays(all.filter((d) => d.employee_id === selId));
      }
      toast("تم تحديث الرواتب من سجلات الحضور");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const archive = async () => {
    setBusy(true);
    try {
      const c = await archivePayroll(period);
      toast(`أُرشف راتب ${c} موظف لشهر ${period}`);
      loadMonths(); // يظهر الشهر فوراً في قائمة الأشهر المؤرشفة
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  // عرض الكشف: مفلتراً لموظف واحد إن رُشّح، وإلا للجميع
  const viewRows =
    selId === "" ? rows : rows.filter((r) => r.employee_id === selId);
  const sel = selId === "" ? null : (rows.find((r) => r.employee_id === selId) ?? null);
  const selEmp = selId === "" ? null : (emps.find((e) => e.id === selId) ?? null);
  const selName = sel?.employee_name ?? selEmp?.name ?? `#${selId}`;
  const selJob = selEmp?.job_title || "";

  const totalNet = viewRows.reduce((a, r) => a + r.net_pay, 0);
  const totalBase = viewRows.reduce((a, r) => a + r.base_pay, 0);
  const totalAllow = viewRows.reduce((a, r) => a + r.allowances, 0);
  const totalWeekend = viewRows.reduce((a, r) => a + r.weekend_pay, 0);

  /** طباعة الورقة: slip=true لقسيمة الموظف وحدها، وإلا الكشف الكامل. */
  const printSheet = (slip: boolean) => {
    if (slip) document.body.classList.add("print-slip");
    try {
      window.print();
    } finally {
      document.body.classList.remove("print-slip");
    }
  };

  // أيام الدوام المطبوعة في القسيمة: الحضور الفعلي + الإجازات + العطل الرسمية
  // + العطل الأسبوعية (تُعرض كعطلة أسبوعية لا كغياب)
  const slipDays = days.filter(
    (d) =>
      d.first_in ||
      d.status === "leave" ||
      d.status === "holiday" ||
      d.status === "weekend",
  );

  return (
    <div className="p-6 space-y-5">
      <style>{printCss}</style>

      <div className="flex items-center justify-between print:hidden">
        <h1 className="text-2xl font-extrabold">كشف الرواتب</h1>
        <div className="flex gap-2 items-center flex-wrap">
          {/* تنقّل بين الأشهر: ‹ السابق | حقل الشهر | التالي › */}
          <div className="flex items-center gap-1">
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none"
              onClick={() => shiftMonth(-1)}
              title="الشهر السابق"
            >
              ‹
            </button>
            <input
              type="month"
              className="input !w-40"
              value={period}
              max={thisPeriod}
              onChange={(e) => e.target.value && setPeriod(e.target.value)}
            />
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
              onClick={() => shiftMonth(1)}
              disabled={period >= thisPeriod}
              title="الشهر التالي"
            >
              ›
            </button>
          </div>
          {period !== thisPeriod && (
            <button className="btn-ghost" onClick={() => setPeriod(thisPeriod)}>
              📅 هذا الشهر
            </button>
          )}
          {/* الأشهر المؤرشفة — اضغط للانتقال إليها */}
          <select
            className="input !w-60"
            value={isArchived ? period : ""}
            onChange={(e) => e.target.value && setPeriod(e.target.value)}
            title="الأشهر المؤرشفة — اختر واحداً لعرضه"
          >
            <option value="">🗃️ الأشهر المؤرشفة ({months.length})</option>
            {months.map((m) => (
              <option key={m.period} value={m.period}>
                {pLabel(m.period)} — {m.employees} موظف · {n(m.net_total)}{" "}
                {curSym}
              </option>
            ))}
          </select>
          {isArchived && (
            <span className="text-xs font-bold text-emerald-600 bg-emerald-50 px-2 py-1 rounded-full">
              ✅ مؤرشف
            </span>
          )}
          <button className="btn-ghost" onClick={recalc} disabled={busy}>
            🔄 تحديث
          </button>
          <button className="btn-ghost" onClick={archive} disabled={busy}>
            🗃️ أرشفة
          </button>
          <button
            className="btn-primary"
            onClick={() => {
              if (rows.length === 0)
                return toast("لا بيانات رواتب — اضغط «تحديث» أولاً", "err");
              printSheet(false);
            }}
          >
            🖨️ طباعة الكشف
          </button>
        </div>
      </div>

      {/* اختيار موظف: عرض الأيام والقسيمة لشخص واحد */}
      <div className="card flex flex-wrap items-center gap-3 print:hidden">
        <span className="text-sm font-bold">📄 الموظف:</span>
        <select
          className="input !w-72"
          value={selId}
          onChange={(e) =>
            setSelId(e.target.value === "" ? "" : Number(e.target.value))
          }
        >
          <option value="">— كل الموظفين (كشف عام) —</option>
          {(emps.length > 0
            ? emps.map((e) => ({
                id: e.id,
                label: `${e.name}${e.pin ? ` (#${e.pin})` : ""}${e.active ? "" : " — موقوف"}`,
              }))
            : rows.map((r) => ({
                id: r.employee_id,
                label: `${r.employee_name}${r.pin ? ` (#${r.pin})` : ""}`,
              }))
          ).map((o) => (
            <option key={o.id} value={o.id}>
              {o.label}
            </option>
          ))}
        </select>
        {selId !== "" && (
          <button
            className="btn-primary"
            onClick={() => setSlipOpen(true)}
            title="عرض قسيمة الراتب على الشاشة قبل الطباعة"
          >
            👁️ عرض قسيمة الراتب
          </button>
        )}
        {selId !== "" && (
          <button
            className="btn-ghost"
            onClick={() => openHist(selId)}
            title="سجل رواتب هذا الموظف في كل الأشهر المؤرشفة"
          >
            📜 سجل الشهور
          </button>
        )}
        {selId !== "" && (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-slate-500">شهر القسيمة:</span>
            <input
              type="month"
              className="input !w-40"
              value={period}
              onChange={(e) => setPeriod(e.target.value)}
            />
          </div>
        )}
        <button
          className="btn-primary"
          onClick={() => {
            if (selId === "") return toast("اختر موظفاً أولاً لطباعة قسيمته", "err");
            printSheet(true);
          }}
          title="طباعة قسيمة الراتب للموظف المحدّد برأس المؤسسة وشعارها"
        >
          📄 طباعة قسيمة الراتب
        </button>
        {selId !== "" && (
          <button className="btn-ghost" onClick={() => setSelId("")}>
            ✕ عرض الكل
          </button>
        )}
      </div>

      {/* مجاميع */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4 pay-main">
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي الأساسي</div>
          <div className="text-xl font-extrabold">
            {n(totalBase)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي البدلات</div>
          <div className="text-xl font-extrabold text-emerald-600">
            {n(totalAllow)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">
            إجمالي بدل العطلة والأيام المدفوعة
          </div>
          <div className="text-xl font-extrabold text-violet-600">
            {n(totalWeekend)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card border-indigo-200 bg-indigo-50">
          <div className="text-xs text-indigo-600">إجمالي الصافي</div>
          <div className="text-xl font-extrabold text-indigo-700">
            {n(totalNet)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
      </div>

      {/* ===== بطاقة تفصيل الموظف: أيام الدوام بدخولها وخروجها ===== */}
      {selId !== "" && (
        <div className="card space-y-4 pay-main">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="font-extrabold text-lg">
                {selName}
                {selJob && (
                  <span className="text-sm font-normal text-slate-500">
                    {" "}
                    — {selJob}
                  </span>
                )}
              </div>
              <div className="text-xs text-slate-500">
                رقم البصمة: {sel?.pin || selEmp?.pin || "—"} · الفترة: {period}
              </div>
            </div>
            {sel && (
              <div className="text-left">
                <div className="text-xs text-slate-500">صافي الراتب</div>
                <div className="text-2xl font-extrabold text-indigo-700">
                  {n(sel.net_pay)}{" "}
                  <span className="text-base opacity-70">{curSym}</span>
                </div>
              </div>
            )}
          </div>

          {sel ? (
            <>
            <div className="grid grid-cols-2 lg:grid-cols-6 gap-3">
              {[
                ["أيام الحضور", String(sel.days_present), ""],
                ["ساعات فعلية", n(sel.total_worked_hours), ""],
                ["إضافي (ساعة)", n(sel.overtime_hours), ""],
                ["الأساسي", n(sel.base_pay), "text-slate-900"],
                ["البدلات", n(sel.allowances), "text-emerald-600"],
                [
                  "بدل العطلة الأسبوعية",
                  sel.weekend_pay ? n(sel.weekend_pay) : "—",
                  "text-violet-600",
                ],
              ].map(([label, val, cls]) => (
                <div key={label} className="rounded-lg border border-slate-200 p-3">
                  <div className="text-[11px] text-slate-500">{label}</div>
                  <div className={`font-extrabold ${cls}`}>
                    {val} {curSym}
                  </div>
                </div>
              ))}
            </div>
            {(sel.paid_leave_days > 0 ||
              sel.unpaid_leave_days > 0 ||
              sel.holiday_days > 0) && (
              <div className="text-xs text-slate-500">
                إجازات مدفوعة: <b>{sel.paid_leave_days}</b> يوم · غير مدفوعة:{" "}
                <b>{sel.unpaid_leave_days}</b> · إجازات رسمية:{" "}
                <b>{sel.holiday_days}</b>
              </div>
            )}
            {sel.weekend_days_paid > 0 && (
              <div className="text-xs text-slate-500">
                بدل العطلة الأسبوعية: <b>{n(sel.weekend_pay)}</b> عن{" "}
                <b>{sel.weekend_days_paid}</b> يوم عطلة · أيام الدوام المدفوعة:{" "}
                <b>{sel.paid_workdays}</b>
              </div>
            )}
            </>
          ) : (
            <div className="p-3 rounded-lg bg-amber-50 border border-amber-300 text-sm text-amber-800">
              لا يوجد كشف راتب لهذا الموظف في الفترة {period} (قد يكون موقوفاً أو
              أُضيف حديثاً) — أيام الدوام أدناه تُعرض من سجلات الحضور مباشرة.
            </div>
          )}

          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="bg-slate-100 text-xs">
                  <th className="th">#</th>
                  <th className="th">اليوم والتاريخ</th>
                  <th className="th">الحضور</th>
                  <th className="th">الانصراف</th>
                  <th className="th">الساعات</th>
                  <th className="th">الحالة</th>
                </tr>
              </thead>
              <tbody>
                {days.length === 0 && (
                  <tr>
                    <td
                      className="td text-center text-slate-400 py-6"
                      colSpan={6}
                    >
                      لا توجد أيام محسوبة لهذه الفترة — اضغط «🔄 تحديث»
                      لاحتسابها من سجلات الحضور.
                    </td>
                  </tr>
                )}
                {days.map((d, i) => {
                  const [label, cls] = stInfo(d);
                  return (
                    <tr key={d.id} className="hover:bg-slate-50">
                      <td className="td">{i + 1}</td>
                      <td className="td text-right">
                        <div className="font-medium">{arabicDay(d.date)}</div>
                        <div className="text-[11px] text-slate-400 font-mono">
                          {d.date}
                        </div>
                      </td>
                      <td className="td font-mono">{hhmm(d.first_in)}</td>
                      <td className="td font-mono">{hhmm(d.last_out)}</td>
                      <td className="td">{hrs(d.worked_min)}</td>
                      <td className="td">
                        <span
                          className={`px-2 py-0.5 rounded-full text-xs font-bold ${cls}`}
                        >
                          {label}
                        </span>
                        {d.overtime_min > 0 && (
                          <span className="text-[10px] text-indigo-500 mr-1">
                            +{d.overtime_min} د إضافي
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      <div className="card !p-0 overflow-hidden pay-main">
        <table className="w-full">
          <thead>
            <tr>
              <th className="th">الموظف</th>
              <th className="th">أيام الحضور</th>
              <th className="th">ساعات فعلية</th>
              <th className="th">إضافي</th>
              <th className="th">الأساسي ({curSym})</th>
              <th className="th">البدلات ({curSym})</th>
              <th className="th">بدل العطلة ({curSym})</th>
              <th className="th">الصافي ({curSym})</th>
            </tr>
          </thead>
          <tbody>
            {viewRows.length === 0 && (
              <tr>
                <td className="td text-center text-slate-400 py-8" colSpan={8}>
                  {selId !== ""
                    ? "لا راتب محسوب لهذا الموظف في هذه الفترة."
                    : "لا بيانات رواتب — اضغط «تحديث» بعد سحب الحضور."}
                </td>
              </tr>
            )}
            {viewRows.map((r) => (
              <tr key={r.employee_id} className="hover:bg-slate-50">
                <td className="td font-medium">
                  <button
                    className="hover:text-indigo-600 underline decoration-dotted underline-offset-4"
                    onClick={() => setSelId(r.employee_id)}
                    title="عرض أيام الدوام وقسيمة هذا الموظف"
                  >
                    {r.employee_name}
                  </button>
                  <span className="text-slate-400 font-mono text-xs mr-1">
                    {r.pin ? `#${r.pin}` : ""}
                  </span>
                  <button
                    className="text-xs text-slate-400 hover:text-indigo-600 mr-1"
                    onClick={() => openHist(r.employee_id)}
                    title="سجل رواتب هذا الموظف في كل الأشهر المؤرشفة"
                  >
                    📜
                  </button>
                </td>
                <td className="td">{r.days_present}</td>
                <td className="td">{n(r.total_worked_hours)}</td>
                <td className="td">{n(r.overtime_hours)}</td>
                <td className="td">{n(r.base_pay)}</td>
                <td className="td text-emerald-600">{n(r.allowances)}</td>
                <td className="td text-violet-600">
                  {r.weekend_days_paid
                    ? `${n(r.weekend_pay)} (${r.weekend_days_paid} يوم عطلة)`
                    : "—"}
                </td>
                <td className="td font-extrabold">{n(r.net_pay)}</td>
              </tr>
            ))}
            {viewRows.length > 0 && (
              <tr className="bg-slate-50 font-extrabold">
                <td className="td" colSpan={7}>
                  {selId === "" ? "الإجمالي العام" : "إجمالي الموظف"}
                </td>
                <td className="td text-indigo-700">{n(totalNet)}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="text-xs text-slate-500 leading-relaxed print:hidden">
        المعادلات: العمل الفعلي = (الانصراف − الحضور) − الاستراحة · الأساسي = الساعات الفعلية ×
        سعر الساعة · البدلات = (فطور + مواصلات) × أيام الحضور · الإضافي يبدأ بعد تجاوز الساعات
        الفعلية (بعد الاستراحة) لساعات الدوام الأساسية (8.5 − 0.5 = 8 ساعات مثلاً) ويُدفع بمضاعف ·
        بدل العطلة (الطريقة الأسبوعية الافتراضية): الأسبوع من السبت إلى الجمعة والجمعة عطلة
        أسبوعية مدفوعة تُسجَّل تلقائياً — مبلغ الأسبوع (62) × (أيام الدوام المستحق من السبت–الخميس
        ÷ أيام الدوام المتاحة بالأسبوع)، فالغياب يُخصم نصيبه (62 ÷ 6) ودوام الجمعة يُضاف عادياً؛
        وإن اخترت «لكل يوم عطلة» من الإعدادات يُدفع 62 عن كل يوم عطلة مستحق · الإجازة المدفوعة/
        غير المدفوعة تُحدَّد عند تسجيلها من بطاقة الموظف · التأخير والانصراف المبكر معلومة في
        الإشعارات فقط ولا يؤثران على الرواتب ·
        الصافي = الأساسي + البدلات + بدل العطلة (لا خصومات إطلاقاً).
      </p>

      {/* ===== سجل رواتب موظف بعينه في الأشهر المؤرشفة ===== */}
      {histOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-50 overflow-y-auto p-4 print:hidden"
          onClick={() => setHistOpen(false)}
        >
          <div
            className="card max-w-5xl mx-auto mt-10"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <h2 className="font-bold text-lg">
                📜 سجل الرواتب — {histName}
                <span className="text-sm font-normal text-slate-500 mr-2">
                  (الأشهر المؤرشفة — الأحدث أولاً)
                </span>
              </h2>
              <button
                className="text-slate-400 hover:text-slate-700 text-sm"
                onClick={() => setHistOpen(false)}
                aria-label="إغلاق سجل الرواتب"
                title="إغلاق (Esc)"
              >
                ✕
              </button>
            </div>
            {hist.length === 0 ? (
              <p className="text-sm text-slate-400 py-8 text-center">
                لا يوجد أرشيف رواتب لهذا الموظف بعد — افتح أي شهر من أعلى
                الصفحة واضغط «🗃️ أرشفة» ليُحفظ في هذا السجل.
              </p>
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-100 text-xs">
                      <th className="th">الشهر</th>
                      <th className="th">أيام الحضور</th>
                      <th className="th">ساعات فعلية</th>
                      <th className="th">إضافي</th>
                      <th className="th">الأساسي ({curSym})</th>
                      <th className="th">البدلات ({curSym})</th>
                      <th className="th">بدل العطلة ({curSym})</th>
                      <th className="th">الصافي ({curSym})</th>
                      <th className="th"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {hist.map((h) => (
                      <tr key={h.period} className="hover:bg-slate-50">
                        <td className="td font-bold">{pLabel(h.period)}</td>
                        <td className="td">{h.days_present}</td>
                        <td className="td">{n(h.total_worked_hours)}</td>
                        <td className="td">{n(h.overtime_hours)}</td>
                        <td className="td">{n(h.base_pay)}</td>
                        <td className="td text-emerald-600">
                          {n(h.allowances)}
                        </td>
                        <td className="td text-violet-600">
                          {n(h.weekend_pay)}
                        </td>
                        <td className="td font-extrabold">{n(h.net_pay)}</td>
                        <td className="td">
                          <button
                            className="text-xs text-indigo-600 hover:underline font-bold"
                            onClick={() => {
                              setPeriod(h.period);
                              setSelId(histEmpId);
                              setHistOpen(false);
                            }}
                            title="عرض كشف هذا الشهر للموظف"
                          >
                            عرض ↩
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      )}

      {/* ===== قسيمة الراتب (نسخة الطباعة — مخفية على الشاشة) ===== */}
      {selId !== "" && (
        <div className="report-sheet pay-slip p-8">
          <SlipSheet
            cfg={cfg}
            logo={logo}
            period={period}
            selName={selName}
            selJob={selJob}
            pin={sel?.pin || selEmp?.pin || "—"}
            sel={sel}
            slipDays={slipDays}
          />
        </div>
      )}

      {/* ===== عرض قسيمة الراتب على الشاشة ===== */}
      {slipOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-50 overflow-y-auto p-4 print:hidden"
          onClick={() => setSlipOpen(false)}
        >
          <div
            className="max-w-3xl mx-auto my-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2 mb-2">
              <div className="flex items-center gap-2">
                <span className="text-sm font-bold text-white">شهر القسيمة:</span>
                <input
                  type="month"
                  className="input !w-40 bg-white"
                  value={period}
                  onChange={(e) => setPeriod(e.target.value)}
                />
              </div>
              <div className="flex gap-2">
                <button className="btn-primary" onClick={() => printSheet(true)}>
                  🖨️ طباعة
                </button>
                <button
                  className="btn-ghost bg-white"
                  onClick={() => setSlipOpen(false)}
                  aria-label="إغلاق عرض كشف الراتب"
                >
                  ✕ إغلاق
                </button>
              </div>
            </div>
            <div className="slip-overlay">
              <div className="report-sheet pay-slip p-8 bg-white shadow-2xl rounded-xl">
                <SlipSheet
                  cfg={cfg}
                  logo={logo}
                  period={period}
                  selName={selName}
                  selJob={selJob}
                  pin={sel?.pin || selEmp?.pin || "—"}
                  sel={sel}
                  slipDays={slipDays}
                />
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** محتوى قسيمة الراتب — مشترك بين الطباعة والعرض على الشاشة. */
function SlipSheet({
  cfg,
  logo,
  period,
  selName,
  selJob,
  pin,
  sel,
  slipDays,
}: {
  cfg: Settings;
  logo: string;
  period: string;
  selName: string;
  selJob: string;
  pin: string;
  sel: PayrollResult | null;
  slipDays: AttendanceDay[];
}) {
  const curSym =
    cfg.currency === "USD" ? "$" : cfg.currency === "JOD" ? "د.أ" : "₪";
  return (
    <>
          <PrintHeader
            cfg={cfg}
            logo={logo}
            title="قسيمة راتب"
            meta={
              <>
                <div className="text-slate-600">الفترة: {period}</div>
                <div className="text-slate-500 text-xs">
                  تاريخ الطباعة: {todayISO()}
                </div>
              </>
            }
          />

          <div className="grid grid-cols-2 gap-3 text-sm mb-4">
            <div className="border border-slate-200 rounded p-3">
              <div className="text-slate-500 text-xs">الموظف</div>
              <div className="font-extrabold">{selName}</div>
              <div className="text-slate-600">
                رقم البصمة: {pin}
                {selJob ? ` · ${selJob}` : ""}
              </div>
            </div>
            <div className="border border-slate-200 rounded p-3">
              <div className="text-slate-500 text-xs">الفترة</div>
              <div className="font-extrabold">{period}</div>
              <div className="text-slate-600">
                {sel ? `${sel.days_present} يوم حضور · ${n(sel.total_worked_hours)} ساعة فعلية` : "—"}
              </div>
            </div>
          </div>

          <div className="text-sm font-bold mb-1">
            أيام الدوام وأوقات الدخول والخروج
          </div>
          <table className="report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>اليوم والتاريخ</th>
                <th>الحضور</th>
                <th>الانصراف</th>
                <th>الساعات</th>
                <th>الحالة</th>
              </tr>
            </thead>
            <tbody>
              {slipDays.length === 0 && (
                <tr>
                  <td colSpan={6} className="text-center text-slate-400">
                    لا توجد أيام حضور محسوبة لهذه الفترة.
                  </td>
                </tr>
              )}
              {slipDays.map((d, i) => {
                const [label, cls] = stInfo(d);
                return (
                  <tr key={d.id}>
                    <td>{i + 1}</td>
                    <td>
                      <span className="font-medium">{arabicDay(d.date)}</span>{" "}
                      <span className="text-slate-400 font-mono text-xs">
                        {d.date}
                      </span>
                    </td>
                    <td className="font-mono">{hhmm(d.first_in)}</td>
                    <td className="font-mono">{hhmm(d.last_out)}</td>
                    <td>{hrs(d.worked_min)}</td>
                    <td>
                      <span
                        className={`px-2 py-0.5 rounded-full text-xs font-bold ${cls}`}
                      >
                        {label}
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>

          {sel && (
            <table className="report-table mt-4">
              <tbody>
                <tr>
                  <td className="font-bold text-right w-1/2">الأساسي</td>
                  <td>
                    {n(sel.base_pay)} {curSym}
                  </td>
                </tr>
                <tr>
                  <td className="font-bold text-right">البدلات (فطور + مواصلات)</td>
                  <td>
                    {n(sel.allowances)} {curSym}
                  </td>
                </tr>
                <tr>
                  <td className="font-bold text-right">
                    بدل العطلة والأيام المدفوعة ({sel.paid_workdays} يوم دوام)
                  </td>
                  <td className="text-violet-600">
                    {sel.weekend_pay ? `+ ${n(sel.weekend_pay)} ${curSym}` : "—"}
                  </td>
                </tr>
                {(sel.paid_leave_days > 0 ||
                  sel.unpaid_leave_days > 0 ||
                  sel.holiday_days > 0) && (
                  <tr>
                    <td className="text-right text-xs text-slate-500">
                      إجازات: مدفوعة {sel.paid_leave_days} · غير مدفوعة{" "}
                      {sel.unpaid_leave_days} · رسمية {sel.holiday_days} (المدفوعة
                      محسوبة ضمن البدل أعلاه)
                    </td>
                    <td className="text-xs text-slate-500">—</td>
                  </tr>
                )}
                <tr className="total-row">
                  <td className="font-extrabold text-right">الصافي المستحق</td>
                  <td className="font-extrabold text-base">
                    {n(sel.net_pay)} {curSym}
                  </td>
                </tr>
              </tbody>
            </table>
          )}

          <div className="grid grid-cols-2 gap-8 mt-8 text-sm text-center">
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">توقيع المستلم</div>
            </div>
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">مسؤول الرواتب</div>
            </div>
          </div>

          <PrintFooter cfg={cfg} date={todayISO()} />
    </>
  );
}
