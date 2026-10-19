import { useEffect, useRef, useState } from "react";
import Modal from "../components/Modal";
import {
  listEmployees,
  upsertEmployee,
  deleteEmployee,
  getSettings,
  pushUserToDevice,
  disableUserOnDevice,
  employeePunches,
  listLeaves,
  addLeave,
  deleteLeave,
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
  weekend_paid: true,
  annual_leave_balance: 0,
  annual_leave_days: 14,
  leave_accrual_month: null,
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
  // هل لمس المستخدم حقل الاسم الإنجليزي يدوياً؟ (يتوقف التوليد التلقائي بعده)
  const enTouched = useRef(false);

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
    lastAutoEn.current = "";
    enTouched.current = false;
    setForm(emp ? { ...emp } : { ...EMPTY });
  };

  // كتابة الاسم العربي: يولّد اقتراحاً لاتينياً فقط عند إضافة موظف جديد ما دام
  // المستخدم لم يلمس الحقل الإنجليزي — لا يمسّ حقل موظف قائم إطلاقاً.
  const setName = (name: string) => {
    const auto = translitArabic(name);
    setForm((f) => {
      if (!f) return f;
      const canAuto =
        !f.id &&
        !enTouched.current &&
        (!f.name_en || f.name_en === lastAutoEn.current);
      return { ...f, name, name_en: canAuto ? auto : f.name_en };
    });
    lastAutoEn.current = auto;
  };

  // توليد يدوي صريح من زر 🔄 (يعمل للإضافة والتعديل معاً)
  const genEn = () => {
    if (!form) return;
    const auto = translitArabic(form.name);
    lastAutoEn.current = auto;
    setForm({ ...form, name_en: auto });
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
      let deactivated = false;
      let pushErr = "";
      if (pin && devName && form.active) {
        try {
          await pushUserToDevice(pin, devName);
          pushed = true;
        } catch (e2) {
          pushErr = String(e2);
        }
      } else if (pin && !form.active) {
        // إيقاف الموظف = حذفه من الجهاز أيضاً حتى لا يبقى يبصم عليه
        try {
          await disableUserOnDevice(pin);
          deactivated = true;
        } catch (e2) {
          // «غير موجود» = كان معطّلاً أصلاً على الجهاز — ليس خطأ
          if (String(e2).includes("غير موجود")) deactivated = true;
          else pushErr = String(e2);
        }
      }
      if (pushed) toast("حُفظ الموظف وأُرسل اسمه إلى الجهاز");
      else if (deactivated)
        toast("حُفظ الموظف وأُوقف على جهاز البصمة — لم يعد يقبل بصمته");
      else if (pushErr)
        toast(`حُفظ الموظف — لكن تعذّر تحديث الجهاز: ${pushErr}`, "err");
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
        <Modal
          onClose={() => setForm(null)}
          title={form.id ? "✏️ تعديل موظف" : "➕ إضافة موظف جديد"}
          panelClass="max-w-2xl"
          backdropClass="z-40"
        >
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
                <label className="label flex items-center justify-between">
                  <span>الاسم بالإنجليزية (على الجهاز) 🔤</span>
                  <button
                    type="button"
                    className="text-xs font-bold text-indigo-600 hover:underline"
                    onClick={genEn}
                    title="توليد الاسم الإنجليزي من الاسم العربي"
                  >
                    🔄 توليد من الاسم العربي
                  </button>
                </label>
                <input
                  className="input text-left"
                  dir="ltr"
                  placeholder="يُولَّد تلقائياً عند الإضافة — أو اضغط 🔄"
                  value={form.name_en}
                  onChange={(e) => {
                    enTouched.current = true; // لا يتوقف التوليد التلقائي بعد هذا اللمس
                    setForm({ ...form, name_en: e.target.value });
                  }}
                />
                <div className="text-[11px] text-slate-500 mt-1 leading-relaxed">
                  شاشة البصمة لا تدعم العربية — يُرسل هذا الاسم إلى الجهاز عند
                  الحفظ. التوليد التلقائي يجري عند إضافة موظف جديد فقط ولا يتغيّر
                  بتعديل الاسم العربي لموظف موجود — استخدم زر 🔄 متى شئت.
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
              <div>
                <label className="label">رصيد الإجازات السنوية (أيام)</label>
                <input
                  type="number"
                  step="0.5"
                  className="input text-left"
                  dir="ltr"
                  title="يُخصم تلقائياً عند تسجيل إجازة سنوية ويعود عند حذفها"
                  value={form.annual_leave_balance}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      annual_leave_balance: Number(e.target.value),
                    })
                  }
                />
              </div>
              <div>
                <label className="label">الاستحقاق السنوي (أيام/سنة)</label>
                <input
                  type="number"
                  step="0.5"
                  min={0}
                  className="input text-left"
                  dir="ltr"
                  title="يُضاف منه 1/12 تلقائياً كل شهر مكتمل إلى الرصيد (الافتراضي 14)"
                  value={form.annual_leave_days}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      annual_leave_days: Number(e.target.value),
                    })
                  }
                />
                <div className="text-[11px] text-slate-500 mt-1 px-0.5">
                  يُضاف شهرياً{" "}
                  {(form.annual_leave_days / 12).toFixed(2)} يوماً تلقائياً
                  {form.leave_accrual_month
                    ? ` — آخر احتساب: ${form.leave_accrual_month}`
                    : ""}
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
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={form.weekend_paid}
                    onChange={(e) =>
                      setForm({ ...form, weekend_paid: e.target.checked })
                    }
                  />
                  <span>العطلة الأسبوعية مدفوعة</span>
                  {!form.weekend_paid && (
                    <span className="text-amber-600 text-xs">
                      (عطلة بدون أجر له)
                    </span>
                  )}
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
                disabled={busyPush || !form.active}
                title={
                  form.active
                    ? "يحدّث اسم الموظف على الجهاز ليدخل عند البصم"
                    : "الموظف موقوف — لن يُعاد إلى الجهاز قبل تفعيله"
                }
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
        </Modal>
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
        <PunchesModal
          emp={viewEmp}
          onClose={() => setViewEmp(null)}
          toast={toast}
          onChanged={load}
        />
      )}
    </div>
  );
}

/** عناوين أنواع الإجازات بالعربية. */
const LEAVE_LABELS: Record<string, string> = {
  annual: "إجازة سنوية",
  sick: "إجازة مرضية",
};

/** عدد أيام المدى [من، إلى] بما فيه اليومان (0 إن كان التاريخ غير صالح). */
const dayCount = (a: string, b: string): number => {
  const t1 = Date.parse(a);
  const t2 = Date.parse(b);
  if (Number.isNaN(t1) || Number.isNaN(t2)) return 0;
  return Math.round((t2 - t1) / 86400000) + 1;
};

/** نافذة عرض كل حركات موظف (دخول/خروج/استراحة/إضافي) + إجازاته، مع بحث. */
function PunchesModal({
  emp,
  onClose,
  toast,
  onChanged,
}: {
  emp: Employee;
  onClose: () => void;
  toast: Toast;
  /** يُستدعى بعد تغيير الإجازات لتحديث قائمة الموظفين (الرصيد في الجدول). */
  onChanged?: () => void;
}) {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [qtime, setQtime] = useState(""); // بحث بالوقت HH:MM أو جزء التاريخ
  const [punches, setPunches] = useState<PunchRow[]>([]);
  const [leaves, setLeaves] = useState<Leave[]>([]);
  const [busy, setBusy] = useState(false);

  // رصيد الإجازات السنوية (يتحدث محلياً فورياً بعد تسجيل/حذف إجازة سنوية)
  const [balance, setBalance] = useState(emp.annual_leave_balance);
  // نموذج إجازة جديدة
  const [lvType, setLvType] = useState("annual");
  const [lvPaid, setLvPaid] = useState(true);
  const [lvFrom, setLvFrom] = useState("");
  const [lvTo, setLvTo] = useState("");
  const [lvNote, setLvNote] = useState("");
  const [lvBusy, setLvBusy] = useState(false);

  // جلب كامل بلا فلاتر — والترشيح يتم فورياً داخل الواجهة
  const load = async () => {
    setBusy(true);
    try {
      setPunches(await employeePunches(emp.id, undefined, undefined, 5000));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const reloadLeaves = () =>
    listLeaves()
      .then((ls) => setLeaves(ls.filter((l) => l.employee_id === emp.id)))
      .catch(() => {});

  useEffect(() => {
    load();
    reloadLeaves();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** تسجيل إجازة جديدة (السنوية تُخصم من الرصيد). */
  const saveLeave = async () => {
    if (!lvFrom || !lvTo) return toast("حدد تاريخ البداية والنهاية", "err");
    const days = dayCount(lvFrom, lvTo);
    if (days <= 0) return toast("تاريخ الانتهاء يجب أن يكون بعد البداية", "err");
    setLvBusy(true);
    try {
      await addLeave(emp.id, lvFrom, lvTo, lvType, lvPaid, lvNote);
      if (lvType === "annual") setBalance((b) => b - days);
      toast(
        `سُجّلت ${LEAVE_LABELS[lvType] ?? "الإجازة"}: ${days} يوم ${
          lvPaid ? "(مدفوعة)" : "(غير مدفوعة)"
        }`,
      );
      setLvFrom("");
      setLvTo("");
      setLvNote("");
      reloadLeaves();
      onChanged?.();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setLvBusy(false);
    }
  };

  /** حذف إجازة — تعود أيامها إلى الرصيد إن كانت سنوية. */
  const removeLeave = async (l: Leave) => {
    const label = LEAVE_LABELS[l.leave_type] ?? l.leave_type;
    const restore = l.leave_type === "annual";
    if (
      !window.confirm(
        `حذف ${label} (${l.from_date} ← ${l.to_date})؟${
          restore ? " تُستعاد أيامها إلى الرصيد." : ""
        }`,
      )
    )
      return;
    try {
      await deleteLeave(l.id);
      if (restore) setBalance((b) => b + dayCount(l.from_date, l.to_date));
      toast("حُذفت الإجازة");
      reloadLeaves();
      onChanged?.();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // ترشيح فوري: من/إلى تاريخ + بحث بالوقت
  const shown = punches.filter(
    (p) =>
      (!from || p.device_time.slice(0, 10) >= from) &&
      (!to || p.device_time.slice(0, 10) <= to) &&
      (!qtime || p.device_time.includes(qtime)),
  );
  const filtersOn = Boolean(from || to || qtime);
  const oldest = punches.length
    ? punches[punches.length - 1].device_time.slice(0, 10)
    : "—";
  const newest = punches.length
    ? punches[0].device_time.slice(0, 10)
    : "—";

  return (
    <Modal
      onClose={onClose}
      title={
        <span>
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
        </span>
      }
      panelClass="max-w-3xl"
    >

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
            {busy ? "جارٍ…" : "🔄 تحديث"}
          </button>
          {filtersOn && (
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

        {/* نطاق الحركات الفعلي المسجّل على النظام */}
        <div className="text-xs text-slate-500 mb-2">
          الإجمالي: {punches.length} حركة — الأقدم: {oldest} — الأحدث: {newest}
        </div>

        {/* كل الحركات خارج الفترة المحددة؟ سبب واضح + مسح فوري */}
        {punches.length > 0 && shown.length === 0 && (
          <div className="p-3 rounded-lg bg-amber-50 border border-amber-300 text-sm text-amber-800 mb-3 leading-relaxed">
            ⚠️ لا حركات ضمن الفترة المحددة. كل حركات هذا الموظف مختومة بين{" "}
            <b>{oldest}</b> و <b>{newest}</b> — وكانت ساعة الجهاز خاطئة وقت
            تسجيلها فظهرت بتواريخ قديمة. اضغط «مسح الترشيح» لعرضها كلها.
          </div>
        )}

        <div className="text-xs text-slate-500 mb-2">
          المعروض: {shown.length} من {punches.length}
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
                // اتجاه الدخول: حضور/عودة من استراحة/دخول إضافي
                const isIn = [0, 3, 4].includes(p.punch_type);
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

        {/* رصيد الإجازات السنوية + تسجيل إجازة جديدة */}
        <div className="flex items-center justify-between mt-5 mb-2">
          <h3 className="font-bold">🌴 الإجازات السنوية والمرضية</h3>
          <span className="badge bg-sky-100 text-sky-700 text-xs">
            الرصيد المتاح: {balance} يوم
          </span>
        </div>

        <div className="bg-slate-50 p-3 rounded-lg border border-slate-200 space-y-2 text-sm mb-3">
          <div className="grid sm:grid-cols-4 gap-2">
            <div>
              <label className="label">النوع</label>
              <select
                className="input"
                value={lvType}
                onChange={(e) => setLvType(e.target.value)}
              >
                <option value="annual">إجازة سنوية</option>
                <option value="sick">إجازة مرضية</option>
              </select>
            </div>
            <div>
              <label className="label">من تاريخ</label>
              <input
                type="date"
                className="input"
                value={lvFrom}
                onChange={(e) => setLvFrom(e.target.value)}
              />
            </div>
            <div>
              <label className="label">إلى تاريخ</label>
              <input
                type="date"
                className="input"
                value={lvTo}
                onChange={(e) => setLvTo(e.target.value)}
              />
            </div>
            <div>
              <label className="label">ملاحظة</label>
              <input
                className="input"
                placeholder="اختياري"
                value={lvNote}
                onChange={(e) => setLvNote(e.target.value)}
              />
            </div>
          </div>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={lvPaid}
              onChange={(e) => setLvPaid(e.target.checked)}
            />
            <span>مدفوعة — تُحتسب ضمن بدل العطلة كيوم دوام</span>
            {!lvPaid && (
              <span className="text-amber-600 text-xs">(بلا أجر)</span>
            )}
          </label>
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs text-slate-500">
              أيام الطلب: <b>{lvFrom && lvTo ? dayCount(lvFrom, lvTo) : "—"}</b>
              {lvType === "annual" && lvFrom && lvTo && (
                <>
                  {" · "}
                  {balance - dayCount(lvFrom, lvTo) < 0 ? (
                    <span className="text-rose-600 font-bold">
                      ⚠️ الرصيد غير كافٍ
                    </span>
                  ) : (
                    <>يتبقّى بعد الخصم {balance - dayCount(lvFrom, lvTo)} يوم</>
                  )}
                </>
              )}
            </span>
            <button className="btn-primary" onClick={saveLeave} disabled={lvBusy}>
              ➕ تسجيل الإجازة
            </button>
          </div>
        </div>

        <h3 className="font-bold mb-2">📋 الإجازات المسجلة</h3>
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
                <span className="badge bg-sky-100 text-sky-700">
                  {LEAVE_LABELS[l.leave_type] ?? l.leave_type}
                </span>
                <span className="font-mono">
                  {l.from_date} ← {l.to_date} ({dayCount(l.from_date, l.to_date)} يوم)
                </span>
                <span
                  className={`badge ${
                    l.paid
                      ? "bg-emerald-100 text-emerald-700"
                      : "bg-rose-100 text-rose-700"
                  }`}
                >
                  {l.paid ? "مدفوعة" : "غير مدفوعة"}
                </span>
                {l.note && <span className="text-slate-500">({l.note})</span>}
                <button
                  className="mr-auto text-rose-500 hover:text-rose-700"
                  title="حذف الإجازة (تعود أيامها للرصيد إن كانت سنوية)"
                  onClick={() => removeLeave(l)}
                >
                  🗑️
                </button>
              </li>
            ))}
          </ul>
        )}
    </Modal>
  );
}
