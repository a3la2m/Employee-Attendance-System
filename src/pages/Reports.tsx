import { useEffect, useState } from "react";
import {
  getPayroll,
  getDailyReport,
  getSettings,
  readLogoDataUri,
} from "../lib/api";
import type { PayrollResult, DailyRow, Settings } from "../lib/types";
import { PrintHeader, PrintFooter } from "../components/PrintChrome";
import { printCss } from "../lib/print";

type Toast = (m: string, t?: "ok" | "err") => void;

const now = new Date();
const thisPeriod = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;
const today = now.toISOString().slice(0, 10);

const n = (v: number) =>
  v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const tfmt = (iso: string | null) => (iso ? (iso.split("T")[1] || "").slice(0, 5) : "—");
const hm = (m: number) => (m ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}` : "—");

export default function Reports({ toast }: { toast: Toast }) {
  const [kind, setKind] = useState<"monthly" | "daily">("monthly");
  const [period, setPeriod] = useState(thisPeriod);
  const [date, setDate] = useState(today);
  const [payroll, setPayroll] = useState<PayrollResult[]>([]);
  const [daily, setDaily] = useState<DailyRow[]>([]);
  const [cfg, setCfg] = useState<Settings>({});
  const [logo, setLogo] = useState<string>("");

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
  }, []);

  useEffect(() => {
    if (kind === "monthly") getPayroll(period).then(setPayroll).catch(() => {});
    else getDailyReport(date).then(setDaily).catch(() => {});
  }, [kind, period, date]);

  const print = () => {
    window.print();
  };

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

  return (
    <div className="p-6 space-y-5">
      <style>{printCss}</style>

      <div className="flex items-center justify-between print:hidden">
        <h1 className="text-2xl font-extrabold">التقارير والطباعة</h1>
        <div className="flex gap-2 items-center">
          <select
            className="input !w-44"
            value={kind}
            onChange={(e) => setKind(e.target.value as "monthly" | "daily")}
          >
            <option value="monthly">كشف رواتب شهري</option>
            <option value="daily">تقرير حضور يومي</option>
          </select>
          {kind === "monthly" ? (
            <div className="flex items-center gap-1">
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
                onChange={(e) => setPeriod(e.target.value)}
              />
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
                onClick={() => shiftMonth(1)}
                disabled={period >= thisPeriod}
                title="الشهر التالي"
                aria-label="الشهر التالي"
              >
                ›
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-1">
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
                max={today}
                onChange={(e) => setDate(e.target.value)}
              />
              <button
                className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
                onClick={() => shiftDay(1)}
                disabled={date >= today}
                title="اليوم التالي"
                aria-label="اليوم التالي"
              >
                ›
              </button>
            </div>
          )}
          <button
            className="btn-primary"
            onClick={() => {
              if (kind === "monthly" && payroll.length === 0)
                return toast("لا بيانات — حدّث الرواتب أولاً", "err");
              print();
            }}
          >
            🖨️ طباعة / حفظ PDF
          </button>
        </div>
      </div>

      {/* ===== ورقة التقرير القابلة للطباعة ===== */}
      <div className="report-sheet bg-white mx-auto w-full max-w-[820px] p-8 shadow">
        {/* الترويسة الموحدة من الإعدادات (الشعار + الرأس) */}
        <PrintHeader
          cfg={cfg}
          logo={logo}
          title={kind === "monthly" ? "كشف رواتب شهري" : "تقرير حضور يومي"}
          meta={
            <>
              <div className="text-slate-600">
                {kind === "monthly" ? `الفترة: ${period}` : `التاريخ: ${date}`}
              </div>
              <div className="text-slate-500 text-xs">تاريخ الطباعة: {today}</div>
            </>
          }
        />

        {/* الجدول */}
        {kind === "monthly" ? (
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
                <th>الصافي</th>
              </tr>
            </thead>
            <tbody>
              {payroll.map((r, i) => (
                <tr key={r.employee_id}>
                  <td>{i + 1}</td>
                  <td className="font-medium">{r.employee_name}</td>
                  <td>{r.days_present}</td>
                  <td>{n(r.total_worked_hours)}</td>
                  <td>{n(r.base_pay)}</td>
                  <td>{n(r.allowances)}</td>
                  <td>{r.weekend_pay ? n(r.weekend_pay) : "—"}</td>
                  <td className="font-bold">{n(r.net_pay)}</td>
                </tr>
              ))}
              {payroll.length > 0 && (
                <tr className="total-row">
                  <td colSpan={7} className="font-extrabold">
                    الإجمالي العام
                  </td>
                  <td className="font-extrabold">{n(totalNet)}</td>
                </tr>
              )}
              {payroll.length === 0 && (
                <tr>
                  <td colSpan={8} className="text-center text-slate-400">
                    لا توجد بيانات لهذه الفترة.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        ) : (
          <table className="w-full report-table">
            <thead>
              <tr>
                <th>#</th>
                <th>الموظف</th>
                <th>رقم البصمة</th>
                <th>الحضور</th>
                <th>الانصراف</th>
                <th>الوقت الفعلي</th>
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
                  <td>
                    {r.status === "leave"
                      ? `${
                          r.leave_type === "sick"
                            ? "إجازة مرضية"
                            : "إجازة سنوية"
                        }${r.leave_paid === false ? " (غير مدفوعة)" : ""}`
                      : r.status === "holiday"
                        ? "عطلة رسمية"
                        : r.status === "weekend"
                          ? "عطلة أسبوعية"
                          : r.absent
                          ? "غياب"
                          : r.status === "incomplete"
                            ? "بصمة ناقصة"
                            : r.first_in
                              ? r.last_out
                                ? "حضور"
                                : "حضور (قيد العمل)"
                              : "—"}
                  </td>
                </tr>
              ))}
              {daily.length === 0 && (
                <tr>
                  <td colSpan={7} className="text-center text-slate-400">
                    لا توجد بيانات لهذا اليوم.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}

        {/* التذييل: توقيع + ختم + تاريخ */}
        <footer className="mt-10 pt-6 border-t border-slate-300 grid grid-cols-3 gap-6 text-sm">
          <div className="text-center">
            <div className="h-16 border-b border-dotted border-slate-400" />
            <div className="mt-2 font-bold">توقيع المدير</div>
          </div>
          <div className="text-center">
            <div className="h-16 grid place-content-center text-slate-300 text-4xl">
              ⭕
            </div>
            <div className="mt-2 font-bold">الختم الرسمي</div>
          </div>
          <div className="text-center">
            <div className="h-16 border-b border-dotted border-slate-400" />
            <div className="mt-2 font-bold">التاريخ والتوقيع</div>
          </div>
        </footer>

        {/* التذييل الموحد من الإعدادات */}
        <PrintFooter cfg={cfg} date={today} />
      </div>
    </div>
  );
}

