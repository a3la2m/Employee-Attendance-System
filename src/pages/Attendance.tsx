import { useEffect, useMemo, useState } from "react";
import {
  getMonthDays,
  recomputeDays,
  getDailyReport,
  syncDevice,
  listEmployees,
  clockNote,
  setDayManual,
  setDayFlags,
  getSettings,
  holidaysList,
  assignCompOff,
  setHolidayWorkChoice,
} from "../lib/api";
import type { AttendanceDay, DailyRow, Employee, Settings, Holiday } from "../lib/types";
import Modal from "../components/Modal";
import { TableSkeleton } from "../components/Skeleton";
import { useToday, monthLocal } from "../lib/useToday";
import { leaveLabel, workedOnHoliday } from "../lib/labels";

type Toast = (m: string, t?: "ok" | "err") => void;

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

/** تاريخ محلي بـ YYYY-MM-DD (بلا إزاحة UTC). */
const isoLocal = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

/** أقل حقول يحتاجها نافذة معلومات العطلة (تشمل صفوف التقرير اليومي وشهور الحضور). */
export interface HolidayInfoRow {
  name?: string;
  worked_min: number;
  holiday_work_choice?: string | null;
  comp_off?: string | null;
  comp_for?: string | null;
  status?: string;
  leave_type?: string | null;
}

/** طلب فتح الحضور على تاريخ معيّن (يُمرَّر من App عند الضغط على تنبيه «يوم بدل»). */
export interface AttendanceFocusProp {
  date: string;
  employeeId?: number;
  openComp?: boolean;
}

export default function Attendance({
  toast,
  focus,
  onFocusHandled,
}: {
  toast: Toast;
  focus?: AttendanceFocusProp | null;
  onFocusHandled?: () => void;
}) {
  const { today, rolledFrom } = useToday();
  const [period, setPeriod] = useState(monthLocal(new Date()));
  const [days, setDays] = useState<AttendanceDay[]>([]);
  const [loadingM, setLoadingM] = useState(true);
  const [busy, setBusy] = useState(false);
  const [dailyDate, setDailyDate] = useState(today);
  const [daily, setDaily] = useState<DailyRow[]>([]);
  const [loadingD, setLoadingD] = useState(true);
  const [emps, setEmps] = useState<Employee[]>([]);
  const [editing, setEditing] = useState<DailyRow | null>(null);
  const [compOff, setCompOff] = useState<DailyRow | null>(null);
  // يوم دوام في عطلة رسمية تُعرض له تفاصيل التعويض (زر ⓘ)
  const [infoDay, setInfoDay] = useState<{
    row: HolidayInfoRow;
    date: string;
    assignRow?: DailyRow;
  } | null>(null);
  const [choiceBusy, setChoiceBusy] = useState<number | null>(null);
  const [cfg, setCfg] = useState<Settings>({});
  const [holidays, setHolidays] = useState<Holiday[]>([]);
  // طلب مُعلَّق من تنبيه «يوم بدل»: ننتظر تحميل التقرير اليومي ثم نفتح النافذة
  const [pendingComp, setPendingComp] = useState<{ date: string; employeeId?: number } | null>(
    null,
  );

  useEffect(() => {
    listEmployees()
      .then(setEmps)
      .catch(() => {});
    getSettings().then(setCfg).catch(() => {});
    holidaysList().then(setHolidays).catch(() => {});
  }, []);

  // وصول طلب من تنبيه «يوم بدل»: افتح التقرير على يوم العطلة ثم نافذة الإسناد
  useEffect(() => {
    if (!focus) return;
    setDailyDate(focus.date);
    setPeriod(focus.date.slice(0, 7));
    if (focus.openComp) {
      setPendingComp({ date: focus.date, employeeId: focus.employeeId });
    }
    onFocusHandled?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);

  // بعد تحميل التقرير اليومي ليوم العطلة: افتح نافذة «يوم بدل» للموظف المطلوب
  useEffect(() => {
    if (!pendingComp || loadingD) return;
    if (pendingComp.date !== dailyDate) return;
    const row = pendingComp.employeeId
      ? daily.find((r) => r.employee_id === pendingComp.employeeId)
      : daily.find((r) => workedOnHoliday(r));
    if (row) setCompOff(row);
    setPendingComp(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingComp, loadingD, daily, dailyDate]);

  // تبدّل اليوم والتطبيق مفتوح ليلاً: رحّل اليوم المعروض والشهر إن كانا
  // مثبَّتين على اليوم/الشهر المنصرم — بلا حاجة لإعادة تشغيل التطبيق.
  useEffect(() => {
    if (!rolledFrom) return;
    setDailyDate((d) => (d === rolledFrom ? today : d));
    setPeriod((p) => (p === monthLocal(new Date(rolledFrom)) ? monthLocal(new Date()) : p));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rolledFrom, today]);

  const empNames = useMemo(
    () => Object.fromEntries(emps.map((e) => [e.id, e.name])),
    [emps],
  );

  const load = async () => {
    setLoadingM(true);
    try {
      setDays(await getMonthDays(period));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setLoadingM(false);
    }
  };

  useEffect(() => {
    load();
  }, [period]);

  const loadDaily = async () => {
    setLoadingD(true);
    try {
      setDaily(await getDailyReport(dailyDate));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setLoadingD(false);
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
      await Promise.all([load(), loadDaily()]);
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
      toast(`سحب: ${r.new_punches} بصمة — أُعيد حساب ${r.days_recomputed} يوم${clockNote(r.clock)}`);
      await Promise.all([load(), loadDaily()]);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  // تبديل خيار التعويض عن دوام العطلة الرسمية لهذا اليوم/الموظف
  const chooseHoliday = async (row: DailyRow, choice: "money" | "comp") => {
    setChoiceBusy(row.employee_id);
    try {
      await setHolidayWorkChoice(row.employee_id, dailyDate, choice);
      toast(
        choice === "money"
          ? `اختير «نقدي» — يُدفع ${cfg.weekend_allow || 62} + الساعات ×1.5 ✅`
          : `اختير «يوم بدل» — ساعات اليوم عادية، ويُنبّهك الجرس لِتُسنده 🎁`,
      );
      await Promise.all([load(), loadDaily()]);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setChoiceBusy(null);
    }
  };

  // تنقّل بين الأشهر (‹ السابق | حقل الشهر | التالي ›) — موحّد مع الرواتب
  const shiftMonth = (delta: number) => {
    const [y, m] = period.split("-").map(Number);
    const d = new Date(y, m - 1 + delta, 1);
    setPeriod(`${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`);
  };

  // تنقّل بين الأيام في التقرير اليومي (مقيّد باليوم الحالي)
  const shiftDay = (delta: number) => {
    const d = new Date(`${dailyDate}T00:00:00`);
    if (Number.isNaN(d.getTime())) return;
    d.setDate(d.getDate() + delta);
    setDailyDate(isoLocal(d));
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
              onChange={(e) => e.target.value && setPeriod(e.target.value)}
            />
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
              onClick={() => shiftMonth(1)}
              disabled={period >= monthLocal(new Date())}
              title="الشهر التالي"
              aria-label="الشهر التالي"
            >
              ›
            </button>
          </div>
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
              value={dailyDate}
              max={today}
              onChange={(e) => e.target.value && setDailyDate(e.target.value)}
            />
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
              onClick={() => shiftDay(1)}
              disabled={dailyDate >= today}
              title="اليوم التالي"
              aria-label="اليوم التالي"
            >
              ›
            </button>
          </div>
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
                <th className="th">إجراء</th>
              </tr>
            </thead>
            <tbody>
              {loadingD ? (
                <TableSkeleton rows={5} cols={8} />
              ) : daily.length === 0 ? (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={8}>
                    لا بيانات لهذا اليوم (أعد الحساب أو اسحب من الجهاز).
                  </td>
                </tr>
              ) : (
                daily.map((r) => {
                  // دوام جارٍ: بصمة دخول بلا انصراف ولم تنقضِ مهلته ← الموظف ما زال يعمل
                  const ongoing =
                    !r.absent && r.status === "present" && !!r.first_in && !r.last_out;
                  return (
                  <tr key={r.id} className="hover:bg-slate-50">
                    <td className="td font-medium">{r.name || `#${r.pin}`}</td>
                    <td className="td font-mono">{tfmt(r.first_in)}</td>
                    <td className="td font-mono">{tfmt(r.last_out)}</td>
                    <td className="td">
                      {ongoing ? (
                        <span
                          className="text-slate-400"
                          title="لم يُسجَّل انصراف بعد — تُحتسب الساعات عند تسجيل الخروج"
                        >
                          قيد العمل
                        </span>
                      ) : (
                        hm(r.worked_min)
                      )}
                    </td>
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
                      {r.status === "leave" ? (
                        <span
                          className={`badge ${
                            r.leave_type === "comp"
                              ? "bg-fuchsia-100 text-fuchsia-700"
                              : "bg-sky-100 text-sky-700"
                          }`}
                        >
                          {leaveLabel(r.leave_type, r.leave_paid)}
                        </span>
                      ) : r.status === "holiday" ? (
                        <span className="badge bg-violet-100 text-violet-700">
                          عطلة رسمية
                        </span>
                      ) : r.status === "weekend" ? (
                        <span className="badge bg-slate-200 text-slate-700">
                          عطلة أسبوعية
                        </span>
                      ) : r.absent ? (
                        <span className="badge bg-rose-100 text-rose-700">غياب</span>
                      ) : r.status === "incomplete" ? (
                        <span
                          className="badge bg-amber-100 text-amber-700"
                          title="بصمة ناقصة: خروج بلا دخول، أو دخول بلا انصراف مضى عليه 14 ساعة — صحّح اليوم بزر ✏️"
                        >
                          بصمة ناقصة
                        </span>
                      ) : (
                        <span className="badge bg-emerald-100 text-emerald-700">
                          حضور
                        </span>
                      )}
                      {workedOnHoliday(r) && (
                        <>
                          <span
                            className="badge bg-violet-100 text-violet-700 mr-1"
                            title="دوّم في عطلة رسمية — اختر له تعويضاً: نقدي أو يوم بدل"
                          >
                            دوام عطلة
                          </span>
                          <span
                            className={`badge mr-1 ${
                              r.holiday_work_choice === "comp"
                                ? "bg-fuchsia-100 text-fuchsia-700"
                                : "bg-amber-100 text-amber-700"
                            }`}
                            title="التعويض المختار لهذا اليوم"
                          >
                            {r.holiday_work_choice === "comp" ? "🎁 بدل" : "💰 نقدي"}
                          </span>
                        </>
                      )}
                      {r.comp_off && (
                        <span
                          className="badge bg-fuchsia-100 text-fuchsia-700 mr-1"
                          title={`أُسند له يوم بدل عن هذه العطلة بتاريخ ${r.comp_off}`}
                        >
                          يوم بدل: {r.comp_off}
                        </span>
                      )}
                      {r.comp_for && (
                        <span
                          className="badge bg-fuchsia-50 text-fuchsia-600 mr-1"
                          title={`عطلة تعويضية بدل دوام عطلة رسمية ${r.comp_for}`}
                        >
                          بدل {r.comp_for}
                        </span>
                      )}
                      {ongoing && (
                        <span
                          className="badge bg-emerald-50 text-emerald-700 mr-1"
                          title="الموظف في دوامه الآن (بصمة دخول بلا انصراف بعد)"
                        >
                          قيد العمل
                        </span>
                      )}
                      {r.manual && (
                        <span
                          className="badge bg-indigo-100 text-indigo-700 mr-1"
                          title="تعديل يدوي من المسؤول"
                        >
                          ✏️ يدوي
                        </span>
                      )}
                      {r.no_break && (
                        <span
                          className="badge bg-emerald-50 text-emerald-700 mr-1"
                          title="أُلغي خصم الاستراحة لهذا اليوم"
                        >
                          بلا استراحة
                        </span>
                      )}
                      {r.no_meal && (
                        <span
                          className="badge bg-orange-50 text-orange-700 mr-1"
                          title="أُزيل بدل الفطور لهذا اليوم"
                        >
                          بلا فطور
                        </span>
                      )}
                    </td>
                    <td className="td">
                      <button
                        className="btn-ghost !py-1 !px-2 text-xs"
                        title="تعديل الحضور/الغياب لهذا اليوم (نسيان بصمة أو عطل الجهاز)"
                        aria-label={`تعديل يوم ${r.name}`}
                        onClick={() => setEditing(r)}
                      >
                        ✏️ تعديل
                      </button>
                      {workedOnHoliday(r) && (
                        <>
                          <button
                            className="btn-ghost !py-1 !px-2 text-xs mr-1"
                            title="لماذا صار هذا اليوم عطلة/بدلاً؟ اشرح لي"
                            aria-label={`معلومات تعويض العطلة ${r.name}`}
                            onClick={() => setInfoDay({ row: r, date: dailyDate, assignRow: r })}
                          >
                            ⓘ معلومات
                          </button>
                          {r.comp_off ? (
                            <span
                              className="badge bg-fuchsia-100 text-fuchsia-700 ml-1"
                              title={`أُسند يوم بدل بتاريخ ${r.comp_off} — احذفه من بطاقة الموظف لتغيير الخيار`}
                            >
                              بدل: {r.comp_off}
                            </span>
                          ) : (
                            <span className="inline-flex gap-1 mr-1 align-middle">
                              <button
                                className={`!py-1 !px-2 text-xs rounded-md border ${
                                  r.holiday_work_choice === "comp"
                                    ? "border-fuchsia-400 bg-fuchsia-50 text-fuchsia-700"
                                    : "border-slate-200 text-slate-500"
                                }`}
                                disabled={choiceBusy === r.employee_id}
                                title={`اختر «يوم بدل»: ساعات اليوم عادية، ويُمنح ${cfg.weekend_allow || 62} كيوم راحة`}
                                onClick={() => chooseHoliday(r, "comp")}
                              >
                                🎁 بدل
                              </button>
                              <button
                                className={`!py-1 !px-2 text-xs rounded-md border ${
                                  r.holiday_work_choice !== "comp"
                                    ? "border-amber-400 bg-amber-50 text-amber-700"
                                    : "border-slate-200 text-slate-500"
                                }`}
                                disabled={choiceBusy === r.employee_id}
                                title={`اختر «نقدي»: يُدفع ${cfg.weekend_allow || 62} + الساعات ×1.5`}
                                onClick={() => chooseHoliday(r, "money")}
                              >
                                💰 نقدي
                              </button>
                            </span>
                          )}
                        </>
                      )}
                    </td>
                  </tr>
                  );
                })
              )}
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
                <th className="th">عطلة رسمية / بدل</th>
              </tr>
            </thead>
            <tbody>
              {loadingM ? (
                <TableSkeleton rows={5} cols={7} />
              ) : byEmp.length === 0 ? (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={7}>
                    لا أيام محسوبة لهذا الشهر بعد.
                  </td>
                </tr>
              ) : (
                byEmp.map((emp, i) => {
                  const rows = emp.rows;
                  const present = rows.filter(
                    (r) => r.first_in && !r.absent,
                  ).length;
                  const absent = rows.filter((r) => r.absent).length;
                  const worked = rows.reduce((a, r) => a + r.worked_min, 0);
                  const late = rows.reduce((a, r) => a + r.late_min, 0);
                  const ot = rows.reduce((a, r) => a + r.overtime_min, 0);
                  const holWork = rows.filter(
                    (r) => r.holiday && !!r.first_in && !r.absent,
                  );
                  const compDays = rows.filter(
                    (r) => r.status === "leave" && r.leave_type === "comp",
                  );
                  const affected = holWork.length + compDays.length;
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
                      <td className="td">
                        {affected === 0 ? (
                          "—"
                        ) : (
                          <span className="inline-flex flex-wrap items-center gap-1">
                            {holWork.length > 0 && (
                              <span className="badge bg-violet-100 text-violet-700">
                                دوام عطلة: {holWork.length}
                              </span>
                            )}
                            {compDays.length > 0 && (
                              <span className="badge bg-fuchsia-100 text-fuchsia-700">
                                بدل: {compDays.length}
                              </span>
                            )}
                            <button
                              className="btn-ghost !py-0.5 !px-1.5 text-xs"
                              title="اشرح لي هذه الأيام: لماذا صارت عطلة/بدلاً وعن أي يوم"
                              aria-label={`معلومات أيام العطلة ${name}`}
                              onClick={() =>
                                setInfoDay({
                                  row: {
                                    name,
                                    worked_min: (holWork[0] ?? compDays[0]).worked_min,
                                    holiday_work_choice:
                                      (holWork[0] ?? compDays[0]).holiday_work_choice,
                                    status: (holWork[0] ?? compDays[0]).status,
                                    leave_type: (holWork[0] ?? compDays[0]).leave_type,
                                  },
                                  date: (holWork[0] ?? compDays[0]).date,
                                })
                              }
                            >
                              ⓘ
                            </button>
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* نافذة التعديل اليدوي */}
      {editing && (
        <EditDayModal
          row={editing}
          date={dailyDate}
          balance={
            emps.find((e) => e.id === editing.employee_id)
              ?.annual_leave_balance ?? 0
          }
          toast={toast}
          onClose={() => setEditing(null)}
          onSaved={() => {
            load();
            loadDaily();
            listEmployees().then(setEmps).catch(() => {});
          }}
        />
      )}

      {/* نافذة «يوم بدل»: إسناد عطلة تعويضية عن دوام في عطلة رسمية */}
      {compOff && (
        <CompOffModal
          row={compOff}
          holidayDate={dailyDate}
          holidays={holidays}
          weekendAllow={Number(cfg.weekend_allow || 62)}
          toast={toast}
          onClose={() => setCompOff(null)}
          onSaved={() => {
            load();
            loadDaily();
          }}
        />
      )}

      {/* نافذة معلومات: لماذا صار اليوم عطلة/بدلاً وعن أي يوم */}
      {infoDay && (
        <HolidayInfoModal
          row={infoDay.row}
          date={infoDay.date}
          weekendAllow={Number(cfg.weekend_allow || 62)}
          onClose={() => setInfoDay(null)}
          onAssign={() => {
            if (infoDay.assignRow) setCompOff(infoDay.assignRow);
            setInfoDay(null);
          }}
        />
      )}
    </div>
  );
}

/** نافذة تعديل يدوي ليوم حضور: ضبط الأوقات / غياب / إرجاع تلقائي +
 *  تحويل اليوم إلى إجازة (خصم/إعادة الرصيد) + زرا الاستراحة والفطور. */
export function EditDayModal({
  row,
  date,
  balance,
  toast,
  onClose,
  onSaved,
}: {
  row: DailyRow;
  date: string;
  balance: number;
  toast: Toast;
  onClose: () => void;
  onSaved: () => void;
}) {
  const t = (s: string | null) => (s ? (s.split("T")[1] || "").slice(0, 5) : "");
  const [mode, setMode] = useState<"present" | "absent" | "auto">("present");
  const [firstIn, setFirstIn] = useState(t(row.first_in));
  const [lastOut, setLastOut] = useState(t(row.last_out));
  const [busy, setBusy] = useState(false);
  const [noBreak, setNoBreak] = useState(row.no_break ?? false);
  const [noMeal, setNoMeal] = useState(row.no_meal ?? false);

  const MODES = [
    { m: "present" as const, l: "حاضر — ضبط أوقات الحضور والانصراف" },
    { m: "absent" as const, l: "غائب — مهما تقول البصمات" },
    { m: "auto" as const, l: "تلقائي — إرجاع اليوم لاحتساب البصمات" },
  ];

  const submit = async () => {
    setBusy(true);
    try {
      await setDayManual(
        row.employee_id,
        date,
        mode,
        mode === "present" ? firstIn : null,
        mode === "present" ? lastOut : null,
      );
      toast(
        mode === "present"
          ? "حُفظ التعديل اليدوي لليوم ✅"
          : mode === "absent"
            ? "عُلّم اليوم غياباً يدوياً ✅"
            : "أُرجع اليوم للاحتساب التلقائي ✅",
      );
      onSaved();
      onClose();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  /** تحويل اليوم إلى إجازة من يوم واحد (السنوية تخصم يوماً من الرصيد). */
  const convertLeave = async (m: "annual" | "sick" | "unpaid") => {
    if (m === "annual" && balance < 1)
      return toast("رصيد الإجازات السنوية غير كافٍ لهذا التحويل", "err");
    setBusy(true);
    try {
      await setDayManual(row.employee_id, date, m, null, null);
      toast(
        m === "annual"
          ? "حُوّل اليوم إلى إجازة سنوية — خُصم يوم من الرصيد ✅"
          : m === "sick"
            ? "حُوّل اليوم إلى إجازة مرضية (بلا خصم) ✅"
            : "حُوّل اليوم إلى إجازة بدون أجر ✅",
      );
      onSaved();
      onClose();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  /** زرا اليوم: يُطبّقان فوراً على صف اليوم (بلا حفظ). */
  const toggleFlag = async (kind: "break" | "meal") => {
    const nb = kind === "break" ? !noBreak : noBreak;
    const nm = kind === "meal" ? !noMeal : noMeal;
    if (kind === "break") setNoBreak(nb);
    else setNoMeal(nm);
    try {
      await setDayFlags(row.employee_id, date, nb, nm);
      toast(
        kind === "break"
          ? nb
            ? "أُلغي خصم الاستراحة لهذا اليوم ✅"
            : "عُاد خصم الاستراحة لهذا اليوم ✅"
          : nm
            ? "أُزيل بدل الفطور لهذا اليوم ✅"
            : "عُاد بدل الفطور لهذا اليوم ✅",
      );
      onSaved();
    } catch (e) {
      if (kind === "break") setNoBreak(!nb);
      else setNoMeal(!nm);
      toast(String(e), "err");
    }
  };

  const leaveBtn = (cls: string) =>
    `btn !py-2 !px-1 flex-col gap-0.5 text-xs leading-tight border ${cls}`;

  return (
    <Modal
      onClose={onClose}
      title="✏️ تعديل الحضور"
      panelClass="max-w-md"
    >
      <div className="text-sm text-slate-500 -mt-1">
        <b>{row.name}</b> · اليوم: <b className="font-mono">{date}</b> · البصمة #
        <span className="font-mono">{row.pin || "—"}</span>
      </div>

      <div className="space-y-2">
        {MODES.map(({ m, l }) => (
          <label
            key={m}
            className={`flex items-center gap-2 p-2.5 rounded-lg border cursor-pointer text-sm ${
              mode === m
                ? "border-indigo-400 bg-indigo-50 font-bold"
                : "border-slate-200"
            }`}
          >
            <input
              type="radio"
              name="day-mode"
              checked={mode === m}
              onChange={() => setMode(m)}
              className="accent-indigo-600"
            />
            {l}
          </label>
        ))}
      </div>

      {mode === "present" && (
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="label" htmlFor="day-first-in">
              وقت الحضور
            </label>
            <input
              id="day-first-in"
              type="text"
              dir="ltr"
              placeholder="HH:MM"
              className="input font-mono"
              value={firstIn}
              onChange={(e) => setFirstIn(e.target.value)}
            />
          </div>
          <div>
            <label className="label" htmlFor="day-last-out">
              وقت الانصراف
            </label>
            <input
              id="day-last-out"
              type="text"
              dir="ltr"
              placeholder="HH:MM"
              className="input font-mono"
              value={lastOut}
              onChange={(e) => setLastOut(e.target.value)}
            />
          </div>
          <p className="col-span-2 text-[11px] text-slate-400 leading-relaxed">
            أمثلة: نسي الدخول — ضع وقت حضوره الحقيقي؛ نسي الخروج — اترك وقت
            الانصراف فارغاً ليُؤخذ آخر خروج محسوب؛ أو اضبط الوقتين معاً إذا لم
            يعمل الجهاز. لدوام يعبر منتصف الليل (مثل 22:00 ← 02:00) اجعل وقت
            الانصراف أصغر من وقت الحضور — يُنسب ليوم الغد بحدٍّ أقصى 14 ساعة.
          </p>
        </div>
      )}
      {mode === "absent" && (
        <p className="text-xs text-rose-500 leading-relaxed">
          سيُعلَّم اليوم غياباً — يبقى مثبتاً حتى ترجعه لتلقائي.
        </p>
      )}
      {mode === "auto" && (
        <p className="text-xs text-slate-500 leading-relaxed">
          يُلغي أي تعديل يدوي ويُعيد احتساب اليوم من بصمات الجهاز.
        </p>
      )}

      {/* تحويل اليوم إلى إجازة */}
      <div className="pt-3 border-t border-slate-100 space-y-2">
        <p className="text-sm font-bold">أو حوّل اليوم إلى إجازة:</p>
        <div className="grid grid-cols-3 gap-2">
          <button
            type="button"
            className={leaveBtn(
              "border-sky-200 bg-sky-50 text-sky-800 hover:bg-sky-100",
            )}
            onClick={() => convertLeave("sick")}
            disabled={busy}
            title="إجازة مرضية — مدفوعة بلا خصم من الرصيد"
          >
            😷 مرضية
            <span className="text-[10px] font-normal opacity-70">بلا خصم</span>
          </button>
          <button
            type="button"
            className={leaveBtn(
              "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100 disabled:opacity-40",
            )}
            onClick={() => convertLeave("annual")}
            disabled={busy || balance < 1}
            title="إجازة سنوية — تخصم يوماً من رصيد الموظف وتعود عند الرجوع"
          >
            🌴 سنوية
            <span className="text-[10px] font-normal opacity-70">
              خصم يوم · الرصيد {balance}
            </span>
          </button>
          <button
            type="button"
            className={leaveBtn(
              "border-slate-200 bg-slate-50 text-slate-700 hover:bg-slate-100",
            )}
            onClick={() => convertLeave("unpaid")}
            disabled={busy}
            title="إجازة بدون أجر — بلا خصم من الرصيد"
          >
            📄 بدون أجر
            <span className="text-[10px] font-normal opacity-70">بلا خصم</span>
          </button>
        </div>
        <p className="text-[11px] text-slate-400 leading-relaxed">
          اليوم الموجود داخل إجازة متعددة الأيام يُرفض ويُعدّل من بطاقة الموظف في
          شاشة «الموظفون». إرجاع اليوم (حاضر/تلقائي) يعيد الرصيد المخصوم.
        </p>
      </div>

      {/* خيارات اليوم السريعة */}
      <div className="pt-3 border-t border-slate-100 space-y-2">
        <p className="text-sm font-bold">خيارات سريعة لهذا اليوم:</p>
        <div className="flex gap-2">
          <button
            type="button"
            aria-pressed={noBreak}
            onClick={() => toggleFlag("break")}
            className={`btn !py-1.5 !px-3 text-xs border ${
              noBreak
                ? "bg-emerald-600 text-white hover:bg-emerald-700 border-emerald-600"
                : "bg-slate-100 text-slate-700 hover:bg-slate-200 border-slate-200"
            }`}
            title="لا تخصم الاستراحة القياسية (30 د) من هذا اليوم"
          >
            {noBreak ? "✓ " : ""}🚫 إلغاء خصم الاستراحة
          </button>
          <button
            type="button"
            aria-pressed={noMeal}
            onClick={() => toggleFlag("meal")}
            className={`btn !py-1.5 !px-3 text-xs border ${
              noMeal
                ? "bg-orange-500 text-white hover:bg-orange-600 border-orange-500"
                : "bg-slate-100 text-slate-700 hover:bg-slate-200 border-slate-200"
            }`}
            title="استبعاد بدل الفطور (7 ₪) من راتب هذا اليوم"
          >
            {noMeal ? "✓ " : ""}🍽️ إزالة بدل الفطور
          </button>
        </div>
        <p className="text-[11px] text-slate-400 leading-relaxed">
          يُطبَّق الزر فوراً على هذا اليوم (دون الحاجة لزر الحفظ) وينعكس على
          ساعات اليوم ورواتب الشهر.
        </p>
      </div>

      <div className="flex justify-end gap-2 pt-1">
        <button className="btn-ghost" onClick={onClose}>
          إلغاء
        </button>
        <button className="btn-primary" onClick={submit} disabled={busy}>
          {busy ? "جارٍ الحفظ…" : "💾 حفظ"}
        </button>
      </div>
    </Modal>
  );
}

/** نافذة «يوم بدل» — عطلة تعويضية عن دوام الموظف في عطلة رسمية.
 *  تُشرح الخياران (نقدي / يوم بدل) كما في الإعدادات، ثم يُسند تاريخ يوم
 *  البدل (يمكن أن يكون في الشهر التالي) فيُسجَّل عطلة تعويضية مدفوعة
 *  بأجر يوم العطلة (62). */
export function CompOffModal({
  row,
  holidayDate,
  holidays,
  weekendAllow,
  toast,
  onClose,
  onSaved,
}: {
  row: DailyRow;
  holidayDate: string;
  holidays: Holiday[];
  weekendAllow: number;
  toast: Toast;
  onClose: () => void;
  onSaved: () => void;
}) {
  // الافتراضي: أول يوم بعد العطلة (غالباً الشهر التالي) — يعدّله المسؤول
  const defaultDate = (() => {
    const d = new Date(`${holidayDate}T00:00:00`);
    d.setDate(d.getDate() + 1);
    return isoLocal(d);
  })();
  // الحد الأدنى للتاريخ: اليوم التالي لعطلة العمل (لا يصح بدل قبل يومها أو في يومها)
  const minDate = defaultDate;
  const [compDate, setCompDate] = useState(defaultDate);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!compDate) return toast("اختر تاريخ يوم البدل", "err");
    if (compDate <= holidayDate)
      return toast(
        `يوم البدل يجب أن يكون بعد تاريخ العطلة ${holidayDate} (يوم استراحة في المستقبل)`,
        "err",
      );
    setBusy(true);
    try {
      await assignCompOff(row.employee_id, holidayDate, compDate);
      toast(`أُسند يوم بدل بتاريخ ${compDate} — يُدفع له ${weekendAllow} ✅`);
      onSaved();
      onClose();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const holidaysNext = holidays.filter((h) => h.date > holidayDate).slice(0, 3);

  return (
    <Modal onClose={onClose} title="🎁 يوم بدل (عطلة تعويضية)" panelClass="max-w-md">
      <div className="text-sm text-slate-500 -mt-1">
        <b>{row.name}</b> دوّم يوم <b className="font-mono">{holidayDate}</b> — وقد
        كان <b>عطلة رسمية</b>.
      </div>

      {/* شرح الخيارين (يُختاران من صف اليوم في التقرير اليومي) */}
      <div className="space-y-2 text-xs leading-relaxed">
        <div className="p-2.5 rounded-lg border border-slate-200 bg-slate-50">
          <b>الخيار الأول — نقدي:</b> يُدفع له أجر يوم العطلة ({weekendAllow}) زائد
          ساعات عمله ×1.5 × سعر الساعة، بدل أجر الساعات العادي لذلك اليوم، في راتبه
          مباشرةً (زر «💰 نقدي» بجانب صف اليوم).
        </div>
        <div className="p-2.5 rounded-lg border border-fuchsia-200 bg-fuchsia-50">
          <b>الخيار الثاني — يوم بدل:</b> تُحتسب له ساعات اليوم عادياً، ويُمنح
          <b> يوماً بديلاً إجازةً مدفوعة</b> بأجر يوم العطلة ({weekendAllow}) — يمكن أن
          يكون في الشهر التالي. اختر تاريخه أدناه.
        </div>
      </div>

      <div>
        <label className="label" htmlFor="comp-date">
          تاريخ يوم البدل (يجب أن يكون بعد العطلة)
        </label>
        <input
          id="comp-date"
          type="date"
          className="input !w-52"
          value={compDate}
          min={minDate}
          onChange={(e) => setCompDate(e.target.value)}
        />
        <p className="text-[11px] text-slate-400 mt-1">
          يوم استراحة في <b>مستقبل</b> بدلاً من دوام يوم العطلة — لا يُخصم من رصيد
          الإجازات السنوية، ويظهر في الحضور كـ«عطلة تعويضية (يوم بدل)» مع ملاحظة
          «بدلاً من دوام عطلة رسمية {holidayDate}»، ويُدفع له أجر يوم العطلة.
        </p>
      </div>

      {holidaysNext.length > 0 && (
        <p className="text-[11px] text-slate-400 leading-relaxed">
          أعياد قادمة: {holidaysNext.map((h) => `${h.date} (${h.name})`).join(" · ")}
        </p>
      )}

      <div className="flex justify-end gap-2 pt-1">
        <button className="btn-ghost" onClick={onClose}>
          إلغاء
        </button>
        <button className="btn-primary" onClick={submit} disabled={busy}>
          {busy ? "جارٍ الإسناد…" : "🎁 إسناد يوم البدل"}
        </button>
      </div>
    </Modal>
  );
}

/** نافذة معلومات ليوم دوام في عطلة رسمية: تشرح لماذا هذا اليوم مؤثّر،
 *  والخيار المختار له، وكيف يُحتسب، وعن أي يوم صار بدلاً إن أُسند. */
export function HolidayInfoModal({
  row,
  date,
  weekendAllow,
  onClose,
  onAssign,
}: {
  row: HolidayInfoRow;
  date: string;
  weekendAllow: number;
  onClose: () => void;
  onAssign: () => void;
}) {
  const isComp = row.holiday_work_choice === "comp";
  const isCompLeave = row.status === "leave" && row.leave_type === "comp";
  const hours = (row.worked_min / 60).toFixed(2);
  return (
    <Modal onClose={onClose} title="ⓘ معلومات يوم العطلة الرسمية" panelClass="max-w-md">
      <div className="space-y-3 text-sm leading-relaxed">
        {isCompLeave ? (
          <div className="p-3 rounded-lg border border-fuchsia-200 bg-fuchsia-50">
            يوم <b className="font-mono">{date}</b> هو <b>🎁 عطلة تعويضية (يوم بدل)</b>
            {" "}لـ<b>{row.name}</b>
            {row.comp_for ? (
              <>
                {" "}بدلاً من دوامه في عطلة <b className="font-mono">{row.comp_for}</b>.
              </>
            ) : (
              <> — بدل دوامه في عطلة رسمية.</>
            )}{" "}
            يُدفع له أجر يوم العطلة (<b>{weekendAllow}</b>) ولا يُخصم من رصيد الإجازات.
          </div>
        ) : (
          <>
            <div className="p-3 rounded-lg border border-violet-200 bg-violet-50">
              <b>{row.name}</b> سجّل دواماً يوم{" "}
              <b className="font-mono">{date}</b>، وهذا التاريخ <b>عطلة رسمية</b> مسجّلة
              في النظام — لذلك يُعامل معاملة خاصة (لا كيوم دوام عادي).
            </div>

            <div>
              <div className="text-xs text-slate-500 mb-1">
                الخيار المختار الآن لهذا الموظف:
              </div>
              {isComp ? (
                <div className="p-3 rounded-lg border border-fuchsia-300 bg-fuchsia-50">
                  <b>🎁 يوم بدل</b> — تُحتسب ساعات هذا اليوم <b>عادياً</b> (×1)، ويُمنح
                  الموظف <b>يوماً راحة تعويضياً مدفوعاً</b> بأجر يوم العطلة (
                  {weekendAllow})، يمكن أن يكون <b>في الشهر التالي</b>.
                  {row.comp_off ? (
                    <div className="mt-1 text-fuchsia-700">
                      ✅ أُسند يوم البدل بتاريخ{" "}
                      <b className="font-mono">{row.comp_off}</b> — فيُحتسب هناك{" "}
                      {weekendAllow} ويظهر «عطلة تعويضية».
                    </div>
                  ) : (
                    <div className="mt-1 text-fuchsia-700">
                      ⏳ لم يُسند بعد — يُنبّهك الجرس حتى تمنحه يوماً بديلاً عبر زر «🎁 يوم بدل».
                    </div>
                  )}
                </div>
              ) : (
                <div className="p-3 rounded-lg border border-amber-300 bg-amber-50">
                  <b>💰 نقدي</b> — يُدفع للموظف في راتبه: <b>{weekendAllow}</b> (أجر يوم
                  العطلة) + <b>{hours} سا</b> × 1.5 × سعر الساعة، <b>بدل</b> أجر ساعاته
                  العادي لذلك اليوم. تظهر في الراتب تحت «دوام في عطلة رسمية».
                </div>
              )}
            </div>
          </>
        )}

        <div className="text-xs text-slate-500 leading-relaxed">
          يمكنك تغيير الخيار من الأزرار بجانب صف اليوم في التقرير اليومي
          (<b>💰 نقدي</b> / <b>🎁 بدل</b>). إن أُسند يوم بدل فلا يمكن تغيير الخيار
          إلا بعد حذف يوم البدل من بطاقة الموظف.
        </div>

        <div className="flex justify-end gap-2 pt-1">
          {isComp && !row.comp_off && (
            <button className="btn-primary" onClick={onAssign}>
              🎁 إسناد يوم البدل
            </button>
          )}
          <button className="btn-ghost" onClick={onClose}>
            إغلاق
          </button>
        </div>
      </div>
    </Modal>
  );
}
