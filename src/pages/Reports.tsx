import { useEffect, useState } from "react";
import {
  getPayroll,
  getDailyReport,
  getSettings,
  readLogoDataUri,
} from "../lib/api";
import type { PayrollResult, DailyRow, Settings } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

const now = new Date();
const thisPeriod = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;
const today = now.toISOString().slice(0, 10);

const n = (v: number) =>
  v.toLocaleString("ar-IQ", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
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

  const companyName = cfg.company_name || "اسم الشركة";
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
            <input
              type="month"
              className="input !w-44"
              value={period}
              onChange={(e) => setPeriod(e.target.value)}
            />
          ) : (
            <input
              type="date"
              className="input !w-44"
              value={date}
              onChange={(e) => setDate(e.target.value)}
            />
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
        {/* الترويسة */}
        <header className="flex items-center justify-between border-b-2 border-slate-800 pb-4 mb-5">
          <div className="flex items-center gap-4">
            {logo ? (
              <img src={logo} alt="شعار" className="w-16 h-16 object-contain" />
            ) : (
              <div className="w-16 h-16 rounded-lg bg-slate-200 grid place-content-center text-2xl">
                🏢
              </div>
            )}
            <div>
              <div className="text-xl font-extrabold">{companyName}</div>
              <div className="text-sm text-slate-600">نظام إدارة الحضور والرواتب</div>
            </div>
          </div>
          <div className="text-left text-sm">
            <div className="font-bold">
              {kind === "monthly" ? "كشف رواتب شهري" : "تقرير حضور يومي"}
            </div>
            <div className="text-slate-600">
              {kind === "monthly" ? `الفترة: ${period}` : `التاريخ: ${date}`}
            </div>
            <div className="text-slate-500 text-xs">تاريخ الطباعة: {today}</div>
          </div>
        </header>

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
                <th>الخصومات</th>
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
                  <td>{n(r.late_deduction + r.absence_deduction)}</td>
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
                    {r.absent
                      ? "غياب"
                      : r.status === "late"
                        ? "تأخير"
                        : r.first_in
                          ? "حضور"
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
      </div>
    </div>
  );
}

// ===== CSS مخصّص للطباعة (RTL) =====
const printCss = `
.report-sheet { direction: rtl; font-family: 'Tajawal', sans-serif; color: #0f172a; }
.report-table { width: 100%; border-collapse: collapse; margin-top: 8px; }
.report-table th {
  background: #f1f5f9; border: 1px solid #cbd5e1; padding: 8px 6px;
  font-size: 12px; text-align: right; font-weight: 700;
}
.report-table td { border: 1px solid #e2e8f0; padding: 7px 6px; font-size: 13px; text-align: center; }
.report-table td:nth-child(2) { text-align: right; }
.report-table .total-row td { background: #eef2ff; border-top: 2px solid #6366f1; }
.report-table tr { page-break-inside: avoid; }

@page { size: A4; margin: 12mm; }

@media print {
  body { background: #fff !important; }
  .print\\:hidden { display: none !important; }
  main { overflow: visible !important; }
  .report-sheet { box-shadow: none !important; max-width: none !important; padding: 0 !important; }
  .report-table thead { display: table-header-group; }
}
`;
