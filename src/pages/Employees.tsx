import { useEffect, useRef, useState } from "react";
import {
  listEmployees,
  upsertEmployee,
  deleteEmployee,
  getSettings,
  pushUserToDevice,
  employeePunches,
  listLeaves,
} from "../lib/api";
import type { PunchRow } from "../lib/api";
import { translitArabic } from "../lib/translit";
import type { Employee, Leave } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

const EMPTY: Employee = {
  id: 0,
  pin: "",
  name: "",
  name_en: "",
  job_title: "",
  phone: "",
  daily_hours: 8.5,
  break_min: 30,
  hourly_rate: 10,
  meal_allow: 7,
  transport_allow: 5,
  active: true,
  overtime_enabled: true,
  meal_enabled: true,
  transport_enabled: true,
};

// نوع الحركة كما يسجلها الجهاز (0=دخول 1=خروج …)
const PUNCH_LABELS: Record<number, string> = {
  0: "حضور (دخول)",
  1: "انصراف (خروج)",
  2: "مغادرة استراحة",
  3: "عودة من استراحة",
  4: "إضافي (دخول)",
  5: "إضافي (خروج)",
};

export default function Employees({ toast }: { toast: Toast }) {
  const [rows, setRows] = useState<Employee[]>([]);
  const [form, setForm] = useState<Employee | null>(null);
  const [q, setQ] = useState("");
  const [status, setStatus] = useState<"all" | "active" | "inactive">("all");
  const [cur, setCur] = useState("ILS");
  const [busyPush, setBusyPush] = useState(false);
  const [viewEmp, setViewEmp] = useState<Employee | null>(null);
  // آخر قيمة لاتينية ولّدها التطبيق من الاسم العربي (لتعرف إن عدّلها المستخدم)
  const lastAutoEn = useRef("");

  const load = () =>
    listEmployees()
      .then(setRows)
      .catch((e) => toast(String(e), "err"));

  useEffect(() => {
    load();
    getSettings()
      .then((s) => setCur(s.currency || "ILS"))
      .catch(() => {});
  }, []);

  const curSym = cur === "USD" ? "$" : cur === "JOD" ? "د.أ" : "₪";

  // فتح النافذة الداخلية للإضافة أو التعديل
  const openForm = (emp?: Employee) => {
    lastAutoEn.current = emp ? translitArabic(emp.name) : "";
    setForm(emp ? { ...emp } : { ...EMPTY });
  };

  // كتابة الاسم العربي يولّد اقتراحاً لاتينياً ما لم يعدّله المستخدم يدوياً
  const setName = (name: string) => {
    const auto = translitArabic(name);
    setForm((f) =>
      f && {
        ...f,
        name,
        name_en:
          !f.name_en || f.name_en === lastAutoEn.current ? auto : f.name_en,
      },
    );
    lastAutoEn.current = auto;
  };

  // إرسال الاسم اللاتيني إلى الجهاز فعلياً (الجهاز لا يدعم العربية)
  const pushToDevice = async (emp: Employee) => {
    const pin = (emp.pin ?? "").trim();
    const devName = (emp.name_en || "").trim() || translitArabic(emp.name);
    if (!pin) return toast("أدخل رقم البصمة أولاً", "err");
    if (!devName) return toast("لا يوجد اسم لاتيني للإرسال", "err");
    setBusyPush(true);
    try {
      const msg = await pushUserToDevice(pin, devName);
      toast(msg, "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyPush(false);
    }
  };

  const save = async () => {
    if (!form) return;
    if (!form.name.trim()) return toast("الاسم مطلوب", "err");
    try {
      await upsertEmployee(form);
      // عند الحفظ يُرسل الاسم إلى الجهاز مباشرة إن توفر اتصال
      const pin = (form.pin ?? "").trim();
      const devName =
        (form.name_en || "").trim() || translitArabic(form.name).trim();
      let pushed = false;
      let pushErr = "";
      if (pin && devName && form.active) {
        try {
          await pushUserToDevice(pin, devName);
          pushed = true;
        } catch (e2) {
          pushErr = String(e2);
        }
      }
      if (pushed) toast("حُفظ الموظف وأُرسل اسمه إلى الجهاز");
      else if (pushErr)
        toast(`حُفظ الموظف — لكن تعذّر الإرسال إلى الجهاز: ${pushErr}`, "err");
      else toast(form.id ? "تم تحديث الموظف" : "تمت إضافة الموظف");
      setForm(null);
      load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const remove = async (emp: Employee) => {
    if (!confirm(`حذف الموظف «${emp.name}» مع سجلاته؟`)) return;
    try {
      await deleteEmployee(emp.id);
      toast("تم الحذف");
      load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const filtered = rows.filter(
    (r) =>
      (status === "all" ||
        (status === "active" ? r.active : !r.active)) &&
      (r.name.includes(q) ||
        (r.name_en ?? "").toLowerCase().includes(q.toLowerCase()) ||
        (r.pin ?? "").includes(q) ||
        r.job_title.includes(q)),
  );

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">الموظفون</h1>
        <div className="flex gap-2 items-center">
          <input
            className="input !w-56"
            placeholder="🔎 بحث بالاسم أو رقم البصمة…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <button className="btn-primary" onClick={() => openForm()}>
            + موظف جديد
          </button>
        </div>
      </div>

      {/* ترشيح الحالة */}
      <div className="flex gap-1 items-center">
        {(
          [
            ["all", "الكل"],
            ["active", "نشط"],
            ["inactive", "موقوف"],
          ] as const
        ).map(([k, lbl]) => (
          <button
            key={k}
            className={`px-3 py-1 rounded-full text-xs font-bold border ${
              status === k
                ? "bg-indigo-600 text-white border-indigo-600"
                : "bg-white text-slate-600 border-slate-300 hover:bg-slate-50"
            }`}
            onClick={() => setStatus(k)}
          >
            {lbl}
          </button>
        ))}
        <span className="text-xs text-slate-400 mr-2">
          {filtered.length} من {rows.length}
        </span>
      </div>

      {/* ===== النافذة الداخلية: إضافة/تعديل موظف ===== */}
      {form && (
        <div
          className="fixed inset-0 z-40 bg-slate-900/50 flex items-start justify-center p-4 overflow-y-auto"
          onClick={() => setForm(null)}
        >
          <div
            className="card w-full max-w-2xl mt-10 bg-white shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <h2 className="font-bold text-lg mb-4">
              {form.id ? "✏️ تعديل موظف" : "➕ إضافة موظف جديد"}
            </h2>
            <div className="grid sm:grid-cols-2 gap-4">
              <div>
                <label className="label">الاسم الكامل (عربي) *</label>
                <input
                  className="input"
                  value={form.name}
                  onChange={(e) => setName(e.target.value)}
                />
              </div>
              <div>
                <label className="label">الاسم بالإنجليزية (على الجهاز) 🔤</label>
                <input
                  className="input text-left"
                  dir="ltr"
                  placeholder="يُولَّد تلقائياً من الاسم العربي"
                  value={form.name_en}
                  onChange={(e) =>
                    setForm({ ...form, name_en: e.target.value })
                  }
                />
                <div className="text-[11px] text-slate-500 mt-1 leading-relaxed">
                  شاشة البصمة لا تدعم العربية — يُرسل هذا الاسم إلى الجهاز عند
                  الحفظ، ويمكنك تعديله يدوياً.
                </div>
              </div>
              <div>
                <label className="label">رقم البصمة (PIN على الجهاز)</label>
                <input
                  className="input text-left"
                  dir="ltr"
                  value={form.pin ?? ""}
                  placeholder="مثال: 9"
                  onChange={(e) => setForm({ ...form, pin: e.target.value })}
                />
              </div>
              <div>
                <label className="label">الوظيفة</label>
                <input
                  className="input"
                  value={form.job_title}
                  onChange={(e) =>
                    setForm({ ...form, job_title: e.target.value })
                  }
                />
              </div>
              <div>
                <label className="label">الهاتف</label>
                <input
                  className="input text-left"
                  dir="ltr"
                  value={form.phone}
                  onChange={(e) => setForm({ ...form, phone: e.target.value })}
                />
              </div>
              <div>
                <label className="label">ساعات العمل اليومية</label>
                <input
                  type="number"
                  step="0.25"
                  className="input text-left"
                  dir="ltr"
                  value={form.daily_hours}
                  onChange={(e) =>
                    setForm({ ...form, daily_hours: Number(e.target.value) })
                  }
                />
              </div>
              <div>
                <label className="label">الاستراحة (دقيقة)</label>
                <input
                  type="number"
                  className="input text-left"
                  dir="ltr"
                  value={form.break_min}
                  onChange={(e) =>
                    setForm({ ...form, break_min: Number(e.target.value) })
                  }
                />
              </div>
              <div>
                <label className="label">سعر الساعة ({curSym})</label>
                <input
                  type="number"
                  step="0.5"
                  className="input text-left"
                  dir="ltr"
                  value={form.hourly_rate}
                  onChange={(e) =>
                    setForm({ ...form, hourly_rate: Number(e.target.value) })
                  }
                />
              </div>
              <div>
                <label className="label">البدلات اليومية ({curSym})</label>
                <div className="flex gap-2">
                  <input
                    type="number"
                    step="0.5"
                    className="input text-left"
                    dir="ltr"
                    title="بدل الفطور"
                    value={form.meal_allow}
                    onChange={(e) =>
                      setForm({ ...form, meal_allow: Number(e.target.value) })
                    }
                  />
                  <input
                    type="number"
                    step="0.5"
                    className="input text-left"
                    dir="ltr"
                    title="بدل المواصلات"
                    value={form.transport_allow}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        transport_allow: Number(e.target.value),
                      })
                    }
                  />
                </div>
                <div className="flex justify-between text-[11px] text-slate-500 mt-1 px-0.5">
                  <span>فطور</span>
                  <span>مواصلات</span>
                </div>
              </div>
            </div>

            {/* مفاتيح التفعيل: العمل الإضافي + الاستثناء من كل بدل على حدة */}
            <div className="mt-4 p-3 rounded-lg bg-slate-50 border border-slate-200">
              <div className="text-xs font-bold text-slate-500 mb-2">
                تفعيل/استثناء
              </div>
              <div className="flex flex-wrap gap-x-5 gap-y-2 text-sm">
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={form.overtime_enabled}
                    onChange={(e) =>
                      setForm({ ...form, overtime_enabled: e.target.checked })
                    }
                  />
                  <span>العمل الإضافي بمضاعفه</span>
                  {!form.overtime_enabled && (
                    <span className="text-amber-600 text-xs">
                      (بالسعر العادي)
                    </span>
                  )}
                </label>
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={form.meal_enabled}
                    onChange={(e) =>
                      setForm({ ...form, meal_enabled: e.target.checked })
                    }
                  />
                  <span>بدل الفطور</span>
                </label>
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={form.transport_enabled}
                    onChange={(e) =>
                      setForm({ ...form, transport_enabled: e.target.checked })
                    }
                  />
                  <span>بدل المواصلات</span>
                </label>
              </div>
            </div>

            {/* حالة الموظف: تنشيط/إلغاء تنشيط واضحة */}
            <div className="mt-4 p-3 rounded-lg border border-slate-200 flex items-center justify-between">
              <div>
                <div className="font-bold text-sm">حالة الموظف</div>
                <div className="text-xs text-slate-500">
                  الموقوف يُستثنى من الرواتب والتقارير الجديدة
                </div>
              </div>
              <div className="flex gap-2">
                <button
                  type="button"
                  className={`px-4 py-1.5 rounded-lg text-sm font-bold border ${
                    form.active
                      ? "bg-emerald-600 text-white border-emerald-600"
                      : "bg-white text-slate-500 border-slate-300"
                  }`}
                  onClick={() => setForm({ ...form, active: true })}
                >
                  ✔ نشط
                </button>
                <button
                  type="button"
                  className={`px-4 py-1.5 rounded-lg text-sm font-bold border ${
                    !form.active
                      ? "bg-rose-600 text-white border-rose-600"
                      : "bg-white text-slate-500 border-slate-300"
                  }`}
                  onClick={() => setForm({ ...form, active: false })}
                >
                  ⏸ موقوف
                </button>
              </div>
            </div>

            <div className="flex items-center gap-2 mt-5 justify-end">
              <button
                className="btn-ghost"
                onClick={() => pushToDevice(form)}
                disabled={busyPush}
                title="يحدّث اسم الموظف على الجهاز ليدخل عند البصم"
              >
                {busyPush ? "جارٍ الإرسال…" : "📤 إرسال الاسم إلى الجهاز"}
              </button>
              <button className="btn-ghost" onClick={() => setForm(null)}>
                إلغاء
              </button>
              <button className="btn-primary" onClick={save}>
                💾 حفظ
              </button>
            </div>
          </div>
        </div>
      )}

      {/* الجدول */}
      <div className="card !p-0 overflow-hidden">
        <table className="w-full">
          <thead>
            <tr>
              <th className="th">الاسم</th>
              <th className="th">رقم البصمة</th>
              <th className="th">الوظيفة</th>
              <th className="th">ساعات/يوم</th>
              <th className="th">سعر الساعة</th>
              <th className="th">الحالة</th>
              <th className="th">إجراءات</th>
            </tr>
          </thead>
          <tbody>
            {filtered.length === 0 && (
              <tr>
                <td className="td text-center text-slate-400 py-8" colSpan={7}>
                  لا يوجد موظفون — اضغط «+ موظف جديد» أو اسحب من الجهاز.
                </td>
              </tr>
            )}
            {filtered.map((r) => (
              <tr
                key={r.id}
                className={`hover:bg-slate-50 ${r.active ? "" : "opacity-60"}`}
              >
                <td className="td font-medium">
                  {r.name}
                  {r.name_en && (
                    <span className="block text-[11px] text-slate-400" dir="ltr">
                      {r.name_en}
                    </span>
                  )}
                </td>
                <td className="td font-mono">{r.pin || "—"}</td>
                <td className="td">{r.job_title || "—"}</td>
                <td className="td">{r.daily_hours}</td>
                <td className="td">{r.hourly_rate}</td>
                <td className="td">
                  <span
                    className={`badge ${
                      r.active
                        ? "bg-emerald-100 text-emerald-700"
                        : "bg-rose-100 text-rose-600"
                    }`}
                  >
                    {r.active ? "نشط" : "موقوف"}
                  </span>
                </td>
                <td className="td">
                  <div className="flex gap-2">
                    <button
                      className="text-emerald-700 hover:underline text-xs"
                      onClick={() => setViewEmp(r)}
                      title="عرض كل الحركات والإجازات"
                    >
                      👁 عرض
                    </button>
                    <button
                      className="text-indigo-600 hover:underline text-xs"
                      onClick={() => openForm(r)}
                    >
                      تعديل
                    </button>
                    <button
                      className="text-rose-600 hover:underline text-xs"
                      onClick={() => remove(r)}
                    >
                      حذف
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* نافذة عرض حركات الموظف */}
      {viewEmp && (
        <PunchesModal emp={viewEmp} onClose={() => setViewEmp(null)} toast={toast} />
      )}
    </div>
  );
}

/** نافذة عرض كل حركات موظف (دخول/خروج/استراحة/إضافي) + إجازاته، مع بحث. */
function PunchesModal({
  emp,
  onClose,
  toast,
}: {
  emp: Employee;
  onClose: () => void;
  toast: Toast;
}) {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [qtime, setQtime] = useState(""); // بحث بالوقت HH:MM أو جزء التاريخ
  const [punches, setPunches] = useState<PunchRow[]>([]);
  const [leaves, setLeaves] = useState<Leave[]>([]);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    setBusy(true);
    try {
      setPunches(
        await employeePunches(emp.id, from || undefined, to || undefined),
      );
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    load();
    listLeaves()
      .then((ls) => setLeaves(ls.filter((l) => l.employee_id === emp.id)))
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const shown = punches.filter((p) => !qtime || p.device_time.includes(qtime));

  return (
    <div
      className="fixed inset-0 z-50 bg-slate-900/50 flex items-start justify-center p-4 overflow-y-auto"
      onClick={onClose}
    >
      <div
        className="card w-full max-w-3xl mt-8 bg-white shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="font-bold text-lg">
            🧾 حركات الموظف: {emp.name}
            {emp.name_en && (
              <span className="text-slate-400 font-normal" dir="ltr">
                {" "}
                — {emp.name_en}
              </span>
            )}
            <span className="text-sm text-slate-500 font-normal">
              {" "}
              (رقم {emp.pin || "—"})
            </span>
          </h2>
          <button className="btn-ghost" onClick={onClose}>
            ✕ إغلاق
          </button>
        </div>

        {/* الترشيحات: من تاريخ، إلى تاريخ، بحث بالوقت */}
        <div className="flex flex-wrap gap-3 items-end p-3 bg-slate-50 rounded-lg border border-slate-200 mb-4">
          <div>
            <label className="label">من تاريخ</label>
            <input
              type="date"
              className="input !w-40"
              value={from}
              onChange={(e) => setFrom(e.target.value)}
            />
          </div>
          <div>
            <label className="label">إلى تاريخ</label>
            <input
              type="date"
              className="input !w-40"
              value={to}
              onChange={(e) => setTo(e.target.value)}
            />
          </div>
          <div>
            <label className="label">بحث بالوقت/التاريخ</label>
            <input
              className="input !w-40 text-left"
              dir="ltr"
              placeholder="08:30 أو 2026-10"
              value={qtime}
              onChange={(e) => setQtime(e.target.value)}
            />
          </div>
          <button className="btn-primary" onClick={load} disabled={busy}>
            {busy ? "جارٍ…" : "🔎 عرض"}
          </button>
          {(from || to || qtime) && (
            <button
              className="btn-ghost"
              onClick={() => {
                setFrom("");
                setTo("");
                setQtime("");
              }}
            >
              مسح الترشيح
            </button>
          )}
        </div>

        <div className="text-xs text-slate-500 mb-2">
          عدد الحركات: {shown.length}
        </div>

        <div className="overflow-auto max-h-80 border border-slate-200 rounded-lg">
          <table className="w-full">
            <thead className="sticky top-0">
              <tr>
                <th className="th">التاريخ</th>
                <th className="th">الوقت</th>
                <th className="th">النوع</th>
                <th className="th">رمز الجهاز</th>
              </tr>
            </thead>
            <tbody>
              {shown.length === 0 && (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={4}>
                    لا حركات ضمن هذا الترشيح.
                  </td>
                </tr>
              )}
              {shown.map((p, i) => {
                const [d, t] = p.device_time.split("T");
                const label =
                  PUNCH_LABELS[p.punch_type] ?? `نوع ${p.punch_type}`;
                const isIn = p.punch_type === 0;
                return (
                  <tr key={i} className="hover:bg-slate-50">
                    <td className="td font-mono">{d}</td>
                    <td className="td font-mono">{(t ?? "").slice(0, 8)}</td>
                    <td className="td">
                      <span
                        className={`badge ${
                          isIn
                            ? "bg-emerald-100 text-emerald-700"
                            : "bg-sky-100 text-sky-700"
                        }`}
                      >
                        {label}
                      </span>
                    </td>
                    <td className="td text-xs text-slate-400 font-mono">
                      {p.raw_status}/{p.work_code}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>

        {/* الإجازات/المغادرات المسجلة */}
        <h3 className="font-bold mt-5 mb-2">🌴 الإجازات المسجلة</h3>
        {leaves.length === 0 ? (
          <div className="text-sm text-slate-400">
            لا إجازات مسجلة لهذا الموظف.
          </div>
        ) : (
          <ul className="space-y-1 text-sm">
            {leaves.map((l) => (
              <li
                key={l.id}
                className="flex gap-2 items-center p-2 bg-amber-50 rounded border border-amber-200"
              >
                <span className="badge bg-amber-100 text-amber-700">
                  {l.leave_type}
                </span>
                <span className="font-mono">
                  {l.from_date} ← {l.to_date}
                </span>
                {l.note && <span className="text-slate-500">({l.note})</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
