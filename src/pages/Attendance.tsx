import { useEffect, useMemo, useState } from "react";
import { getMonthDays, recomputeDays, getDailyReport, syncDevice, listEmployees } from "../lib/api";
import type { AttendanceDay, DailyRow } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

const now = new Date();
const thisPeriod = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;

function hm(min: number) {
  const h = Math.floor(min / 60);
  const m = min % 60;
  return `${h}س ${String(m).padStart(2, "0")}د`;
}

function tfmt(iso: string | null) {
  if (!iso) return "—";
  const p = iso.split("T")[1];
  return p ? p.slice(0, 5) : iso;
}

export default function Attendance({ toast }: { toast: Toast }) {
  const [period, setPeriod] = useState(thisPeriod);
  const [days, setDays] = useState<AttendanceDay[]>([]);
  const [busy, setBusy] = useState(false);
  const [dailyDate, setDailyDate] = useState(
    `${period}-${String(now.getDate()).padStart(2, "0")}`,
  );
  const [daily, setDaily] = useState<DailyRow[]>([]);
  const [empNames, setEmpNames] = useState<Record<number, string>>({});

  useEffect(() => {
    listEmployees()
      .then((list) =>
        setEmpNames(Object.fromEntries(list.map((e) => [e.id, e.name]))),
      )
      .catch(() => {});
  }, []);

  const load = async () => {
    try {
      setDays(await getMonthDays(period));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    load();
  }, [period]);

  const loadDaily = async () => {
    try {
      setDaily(await getDailyReport(dailyDate));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    loadDaily();
  }, [dailyDate]);

  const recompute = async () => {
    setBusy(true);
    const [y, m] = period.split("-").map(Number);
    const last = new Date(y, m, 0).getDate();
    const from = `${period}-01`;
    const to = `${period}-${String(last).padStart(2, "0")}`;
    try {
      const n = await recomputeDays(from, to);
      toast(`أُعيد حساب ${n} يوم حضور`);
      await load();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const fullSync = async () => {
    setBusy(true);
    try {
      const r = await syncDevice();
      toast(`سحب: ${r.new_punches} بصمة — أُعيد حساب ${r.days_recomputed} يوم`);
      await load();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  // تجميع لكل موظف
  const byEmp = useMemo(() => {
    const map = new Map<
      number,
      { name: string; rows: AttendanceDay[] }
    >();
    for (const d of days) {
      if (!map.has(d.employee_id))
        map.set(d.employee_id, { name: "", rows: [] });
      map.get(d.employee_id)!.rows.push(d);
    }
    return [...map.values()];
  }, [days]);

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">سجل الحضور</h1>
        <div className="flex gap-2 items-center">
          <input
            type="month"
            className="input !w-44"
            value={period}
            onChange={(e) => setPeriod(e.target.value)}
          />
          <button className="btn-ghost" onClick={recompute} disabled={busy}>
            🔄 إعادة احتساب
          </button>
          <button className="btn-primary" onClick={fullSync} disabled={busy}>
            🔐 سحب من الجهاز
          </button>
        </div>
      </div>

      {/* التقرير اليومي */}
      <div className="card">
        <div className="flex items-center justify-between mb-3">
          <h2 className="font-bold">📋 التقرير اليومي</h2>
          <input
            type="date"
            className="input !w-44"
            value={dailyDate}
            onChange={(e) => setDailyDate(e.target.value)}
          />
        </div>
        <div className="overflow-auto">
          <table className="w-full">
            <thead>
              <tr>
                <th className="th">الموظف</th>
                <th className="th">الحضور</th>
                <th className="th">الانصراف</th>
                <th className="th">الوقت الفعلي</th>
                <th className="th">التأخير</th>
                <th className="th">الإضافي</th>
                <th className="th">الحالة</th>
              </tr>
            </thead>
            <tbody>
              {daily.length === 0 && (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={7}>
                    لا بيانات لهذا اليوم (أعد الحساب أو اسحب من الجهاز).
                  </td>
                </tr>
              )}
              {daily.map((r) => (
                <tr key={r.id} className="hover:bg-slate-50">
                  <td className="td font-medium">{r.name || `#${r.pin}`}</td>
                  <td className="td font-mono">{tfmt(r.first_in)}</td>
                  <td className="td font-mono">{tfmt(r.last_out)}</td>
                  <td className="td">{hm(r.worked_min)}</td>
                  <td className="td">
                    {r.late_min ? (
                      <span className="text-rose-600 font-bold">
                        {hm(r.late_min)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="td">
                    {r.overtime_min ? (
                      <span className="text-emerald-600 font-bold">
                        {hm(r.overtime_min)}
                      </span>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="td">
                    {r.absent ? (
                      <span className="badge bg-rose-100 text-rose-700">غياب</span>
                    ) : r.status === "late" ? (
                      <span className="badge bg-amber-100 text-amber-700">تأخير</span>
                    ) : (
                      <span className="badge bg-emerald-100 text-emerald-700">حضور</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* الملخص الشهري */}
      <div className="card !p-0 overflow-hidden">
        <div className="px-5 py-3 border-b border-slate-200 font-bold">
          📅 ملخص الشهر ({period})
        </div>
        <div className="overflow-auto max-h-[28rem]">
          <table className="w-full">
            <thead className="sticky top-0">
              <tr>
                <th className="th">الموظف</th>
                <th className="th">أيام حضور</th>
                <th className="th">إجمالي العمل</th>
                <th className="th">تأخير</th>
                <th className="th">إضافي</th>
                <th className="th">أيام غياب</th>
              </tr>
            </thead>
            <tbody>
              {byEmp.length === 0 && (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={6}>
                    لا أيام محسوبة لهذا الشهر بعد.
                  </td>
                </tr>
              )}
              {byEmp.map((emp, i) => {
                const rows = emp.rows;
                const present = rows.filter(
                  (r) => r.first_in && !r.absent,
                ).length;
                const absent = rows.filter((r) => r.absent).length;
                const worked = rows.reduce((a, r) => a + r.worked_min, 0);
                const late = rows.reduce((a, r) => a + r.late_min, 0);
                const ot = rows.reduce((a, r) => a + r.overtime_min, 0);
                const empId = rows[0]?.employee_id;
                const name = empNames[empId] || `موظف #${empId}`;
                return (
                  <tr key={i} className="hover:bg-slate-50">
                    <td className="td font-medium">{name}</td>
                    <td className="td">{present}</td>
                    <td className="td">{hm(worked)}</td>
                    <td className="td">
                      {late ? (
                        <span className="text-rose-600 font-bold">{hm(late)}</span>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="td">
                      {ot ? (
                        <span className="text-emerald-600 font-bold">{hm(ot)}</span>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="td">{absent}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
