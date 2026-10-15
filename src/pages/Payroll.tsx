import { useEffect, useState } from "react";
import { getPayroll, archivePayroll, recomputeDays } from "../lib/api";
import type { PayrollResult } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

const now = new Date();
const thisPeriod = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;

const n = (v: number) =>
  v.toLocaleString("ar-IQ", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

export default function Payroll({ toast }: { toast: Toast }) {
  const [period, setPeriod] = useState(thisPeriod);
  const [rows, setRows] = useState<PayrollResult[]>([]);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      setRows(await getPayroll(period));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    load();
  }, [period]);

  const recalc = async () => {
    setBusy(true);
    const [y, m] = period.split("-").map(Number);
    const last = new Date(y, m, 0).getDate();
    try {
      await recomputeDays(`${period}-01`, `${period}-${String(last).padStart(2, "0")}`);
      await load();
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
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const totalNet = rows.reduce((a, r) => a + r.net_pay, 0);
  const totalBase = rows.reduce((a, r) => a + r.base_pay, 0);
  const totalAllow = rows.reduce((a, r) => a + r.allowances, 0);
  const totalDeduct =
    rows.reduce((a, r) => a + r.late_deduction + r.absence_deduction, 0);

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">كشف الرواتب</h1>
        <div className="flex gap-2 items-center">
          <input
            type="month"
            className="input !w-44"
            value={period}
            onChange={(e) => setPeriod(e.target.value)}
          />
          <button className="btn-ghost" onClick={recalc} disabled={busy}>
            🔄 تحديث
          </button>
          <button className="btn-primary" onClick={archive} disabled={busy}>
            🗃️ أرشفة
          </button>
        </div>
      </div>

      {/* مجاميع */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي الأساسي</div>
          <div className="text-xl font-extrabold">{n(totalBase)}</div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي البدلات</div>
          <div className="text-xl font-extrabold text-emerald-600">
            {n(totalAllow)}
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي الخصومات</div>
          <div className="text-xl font-extrabold text-rose-600">{n(totalDeduct)}</div>
        </div>
        <div className="card border-indigo-200 bg-indigo-50">
          <div className="text-xs text-indigo-600">إجمالي الصافي</div>
          <div className="text-xl font-extrabold text-indigo-700">{n(totalNet)}</div>
        </div>
      </div>

      <div className="card !p-0 overflow-hidden">
        <table className="w-full">
          <thead>
            <tr>
              <th className="th">الموظف</th>
              <th className="th">أيام الحضور</th>
              <th className="th">ساعات فعلية</th>
              <th className="th">إضافي</th>
              <th className="th">الأساسي</th>
              <th className="th">البدلات</th>
              <th className="th">تأخير</th>
              <th className="th">غياب</th>
              <th className="th">الصافي</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && (
              <tr>
                <td className="td text-center text-slate-400 py-8" colSpan={9}>
                  لا بيانات رواتب — اضغط «تحديث» بعد سحب الحضور.
                </td>
              </tr>
            )}
            {rows.map((r) => (
              <tr key={r.employee_id} className="hover:bg-slate-50">
                <td className="td font-medium">
                  {r.employee_name}
                  <span className="text-slate-400 font-mono text-xs mr-1">
                    {r.pin ? `#${r.pin}` : ""}
                  </span>
                </td>
                <td className="td">{r.days_present}</td>
                <td className="td">{n(r.total_worked_hours)}</td>
                <td className="td">{n(r.overtime_hours)}</td>
                <td className="td">{n(r.base_pay)}</td>
                <td className="td text-emerald-600">{n(r.allowances)}</td>
                <td className="td text-rose-600">
                  {r.late_deduction ? n(r.late_deduction) : "—"}
                </td>
                <td className="td text-rose-600">
                  {r.absence_deduction ? n(r.absence_deduction) : "—"}
                </td>
                <td className="td font-extrabold">{n(r.net_pay)}</td>
              </tr>
            ))}
            {rows.length > 0 && (
              <tr className="bg-slate-50 font-extrabold">
                <td className="td" colSpan={8}>
                  الإجمالي العام
                </td>
                <td className="td text-indigo-700">{n(totalNet)}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="text-xs text-slate-500 leading-relaxed">
        المعادلات: العمل الفعلي = (الانصراف − الحضور) − الاستراحة · الأساسي = الساعات الفعلية ×
        سعر الساعة · البدلات = (فطور + مواصلات) × أيام الحضور · الخصومات = (ساعات تأخير +
        غياب) × سعر الساعة · الإضافي بمضاعف · الصافي = الأساسي + البدلات − الخصومات. جميع
        القيم قابلة للتعديل من الإعدادات.
      </p>
    </div>
  );
}
