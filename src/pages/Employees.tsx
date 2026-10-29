import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import Modal from "../components/Modal";
import { useConfirm } from "../components/Confirm";
import { PrintHeader, PrintFooter } from "../components/PrintChrome";
import {
  listEmployees,
  upsertEmployee,
  deleteEmployee,
  getSettings,
  readLogoDataUri,
  pushUserToDevice,
  disableUserOnDevice,
  setDevicePrivilege,
  getNextDevicePin,
  employeePunches,
  listLeaves,
  addLeave,
  deleteLeave,
  holidaysList,
  assignCompOff,
  listShifts,
  reassignShiftRecompute,
  listEmployeeAdvances,
  addEmployeeAdvance,
  toggleAdvanceInstallment,
  deleteEmployeeAdvance,
} from "../lib/api";
import type { PunchRow } from "../lib/api";
import { translitArabic } from "../lib/translit";
import { printCss, todayISO } from "../lib/print";
import { pairPunches, fmtDur } from "../lib/punchPairs";
import type { PunchPairRow } from "../lib/punchPairs";
import type {
  Employee,
  EmployeeAdvance,
  Leave,
  Holiday,
  Shift,
  Settings,
} from "../lib/types";

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
  shift_id: null,
  device_privilege: 0,
  bot_allowed: true,
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

/** عدد صفوف الصفحة الواحدة (A4) في ورقة طباعة الحركات. */
const ROWS_PER_PAGE = 27;

/** تقسيم مصفوفة إلى صفحات بحجم ثابت (لطباعة A4 بعدة صفحات). */
function chunk<T>(arr: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < arr.length; i += size) out.push(arr.slice(i, i + size));
  return out;
}

/** تحويل قيمة إلى خلية CSV مع تهريب الفواصل والاقتباسات. */
const csvCell = (v: string | number) => {
  const s = String(v ?? "");
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
};

/** لصق صلاحية الجهاز بالعربية (تُعرض في رسائل الحفظ وشاشات العرض). */
const privLabel = (p: number) =>
  p === 14 ? "سوبر أدمن (أدمن كامل)" : p === 6 ? "مدير" : "مستخدم عادي";

/** الشهر الحالي: [أوله، آخره] — يملأ نطاق حركات الموظف تلقائياً. */
const curMonth = (): [string, string] => {
  const d = new Date();
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const last = new Date(y, d.getMonth() + 1, 0).getDate();
  return [`${y}-${m}-01`, `${y}-${m}-${String(last).padStart(2, "0")}`];
};

export default function Employees({ toast }: { toast: Toast }) {
  // تأكيد «نعم/لا» داخلي — لا يستجيب للنقرة العابرة كـ window.confirm
  const confirm = useConfirm();
  const [rows, setRows] = useState<Employee[]>([]);
  const [form, setForm] = useState<Employee | null>(null);
  const [q, setQ] = useState("");
  const [status, setStatus] = useState<"all" | "active" | "inactive">("all");
  const [cur, setCur] = useState("ILS");
  const [viewEmp, setViewEmp] = useState<Employee | null>(null);
  const [advEmp, setAdvEmp] = useState<Employee | null>(null);
  const [shifts, setShifts] = useState<Shift[]>([]);
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
    listShifts().then(setShifts).catch(() => {});
  }, []);

  const curSym = cur === "USD" ? "$" : cur === "JOD" ? "د.أ" : "₪";

  // فتح النافذة الداخلية للإضافة أو التعديل
  const openForm = (emp?: Employee) => {
    lastAutoEn.current = "";
    enTouched.current = false;
    if (emp) {
      // لا نضع علامة «لمس يدوي»: الخروج من حقل الاسم العربي يعيد التوليد دائماً
      // (ما لم يعدّل المستخدم الحقل الإنجليزي بنفسه).
      setForm({ ...emp, name_en: emp.name_en ?? "" });
      lastAutoEn.current = translitArabic(emp.name);
    } else {
      setForm({ ...EMPTY, daily_hours: 8.5, break_min: 30, shift_id: null });
      // تعبئة رقم البصمة تلقائياً (أعلى رقم معروف + 1) — بلا زر ولا خطوة يدوية
      suggestNextPin();
    }
  };

  // كتابة الاسم العربي: يولّد اقتراحاً لاتينياً بعد توقف الكتابة أو عند الخروج من الحقل
  const setName = (name: string) => {
    const auto = translitArabic(name);
    setForm((f) => {
      if (!f) return f;
      const canAuto =
        !enTouched.current &&
        (!f.name_en || f.name_en === lastAutoEn.current);
      return { ...f, name, name_en: canAuto ? auto : f.name_en };
    });
    lastAutoEn.current = auto;
  };

  // توليد فوري لحظة الخروج من حقل الاسم العربي — يعاد كتابة الاسم الإنجليزي
  // من آخر قيمة عربية، إلا إذا عدّل المستخدم الحقل الإنجليزي بنفسه (enTouched).
  const autoGenEnOnBlur = () => {
    setForm((f) => {
      if (!f || enTouched.current) return f;
      const auto = translitArabic(f.name);
      if (!auto.trim()) return f;
      lastAutoEn.current = auto;
      return { ...f, name_en: auto };
    });
  };

  /** تعبئة رقم البصمة لموظف جديد تلقائياً: أعلى رقم على الجهاز أو في البرنامج
   * + 1 (مثال: آخر رقم 30 ← يُملأ 31). تحديث وظيفي يتجاهل أي تعديل يدوي. */
  const suggestNextPin = () => {
    getNextDevicePin()
      .then((p) => {
        if (!p) return;
        setForm((f) => (f && !f.id ? { ...f, pin: p } : f));
      })
      .catch(() => {});
  };

  // إرسال الاسم اللاتيني إلى الجهاز يجري تلقائياً عند الحفظ (بلا زر منفصل)

  const save = async () => {
    if (!form) return;
    if (!form.name.trim()) return toast("الاسم مطلوب", "err");
    const prev = rows.find((r) => r.id === form.id);
    const prevShift = prev?.shift_id ?? null;
    const shiftChanged = form.id > 0 && prevShift !== form.shift_id;
    const prevActive = prev ? prev.active : true; // الموظف الجديد يُعامَل كنشط
    const prevPriv = prev?.device_privilege ?? 0;
    const reactivated = form.id > 0 && !prevActive && form.active;
    // إيقاف موظف نشط: تأكيد «نعم/لا» قبل أي حفظ — يشرح أثره على جهاز البصمة
    if (form.id > 0 && prevActive && !form.active) {
      const ok = await confirm(
        `إيقاف الموظف «${form.name}» سيحذف بصمه من جهاز البصمة فلا يعود يُبصم.\n` +
          `سجل حضوره السابق يبقى كاملاً في البرنامج.\n` +
          `وعند إعادة تفعيله لاحقاً يجب إعادة تعريفه على الجهاز وتسجيل بصماته من جديد.\n\n` +
          `هل تريد المتابعة؟`,
        { title: "إيقاف موظف", yes: "نعم، أوقفه" },
      );
      if (!ok) return; // إلغاء: لا حفظ محلي ولا أي تعديل على الجهاز
    }
    try {
      await upsertEmployee(form);
      // عند الحفظ يُرسل الموظف إلى الجهاز تلقائياً (إن توفر اتصال)
      const pin = (form.pin ?? "").trim();
      const devName =
        (form.name_en || "").trim() || translitArabic(form.name).trim();
      let pushed = false;
      let suspended: "" | "removed" | "absent" = "";
      let privApplied = false;
      let errNote = "";
      if (pin && devName && form.active) {
        try {
          await pushUserToDevice(pin, devName);
          pushed = true;
        } catch (e2) {
          errNote = `تعذّر إرساله إلى الجهاز: ${e2}`;
        }
        // صلاحية الجهاز (سوبر أدمن/مدير): تُرسل عند تغييرها فقط — لا تمسّ البصمات،
        // وكذلك بعد إعادة التفعيل (الحذف من الجهاز يُسقط الامتياز القديم)
        if (
          pushed &&
          (form.device_privilege !== prevPriv ||
            (reactivated && form.device_privilege > 0))
        ) {
          try {
            await setDevicePrivilege(pin, form.device_privilege);
            privApplied = true;
          } catch (e2) {
            errNote += `${errNote ? " — " : ""}تعذّر تحديث صلاحيته على الجهاز: ${e2}`;
          }
        }
      } else if (pin && !form.active) {
        // إيقاف على الجهاز: «removed» = حُذف سجله الآن، «absent» = غير موجود أصلاً
        try {
          await disableUserOnDevice(pin);
          suspended = "removed";
        } catch (e2) {
          if (String(e2).includes("غير موجود")) suspended = "absent";
          else errNote = `تعذّر إيقافه على الجهاز: ${e2}`;
        }
      }
      // تغيّرت الوردية؟ أعد احتساب حضوره (آخر ٦٠ يوماً) ليُطبَّق التوقيت الجديد.
      if (shiftChanged) {
        const today = new Date();
        const from = new Date(today.getTime() - 60 * 86400000)
          .toISOString()
          .slice(0, 10);
        const to = today.toISOString().slice(0, 10);
        try {
          await reassignShiftRecompute(from, to);
          toast("حُفظ الموظف وأُعيد احتساب حضوره حسب الوردية الجديدة ✅", "ok");
          setForm(null);
          load();
          return;
        } catch (e2) {
          toast(`حُفظ الموظف — لكن تعذّر إعادة الاحتساب: ${String(e2)}`, "err");
          setForm(null);
          load();
          return;
        }
      }
      if (errNote) {
        toast(`حُفظ الموظف محلياً — لكن ${errNote}`, "err");
      } else {
        const msgs: string[] = [];
        if (suspended === "removed")
          msgs.push(
            prevActive
              ? "أُوقف على جهاز البصمة (حُذف سجله منه — لم يعد يُبصم) ✅"
              : "أُزيل من جهاز البصمة الآن (كان موقوفاً في البرنامج فقط) ✅",
          );
        else if (suspended === "absent")
          msgs.push("موقوف — وغير موجود على جهاز البصمة ✔");
        else if (reactivated && pushed)
          msgs.push(
            "أُعيد تعريفه على الجهاز — سجّل بصماته من جديد على الجهاز 🔁",
          );
        else if (pushed) msgs.push("أُرسل إلى الجهاز تلقائياً");
        if (privApplied)
          msgs.push(`الصلاحية على الجهاز: ${privLabel(form.device_privilege)} 👑`);
        if (msgs.length) toast(`حُفظ الموظف — ${msgs.join(" · ")}`);
        else toast(form.id ? "تم تحديث الموظف" : "تمت إضافة الموظف");
      }
      setForm(null);
      load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const remove = async (emp: Employee) => {
    const ok = await confirm(`حذف الموظف «${emp.name}» مع سجلاته؟`, {
      title: "حذف موظف",
      yes: "نعم، احذف",
    });
    if (!ok) return;
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
                  onBlur={autoGenEnOnBlur}
                  onKeyDown={(e) => {
                    if (e.key === "Tab") {
                      setTimeout(autoGenEnOnBlur, 50);
                    }
                  }}
                />
              </div>
              <div>
                <label className="label">الاسم بالإنجليزية (على الجهاز) 🔤</label>
                <input
                  className="input text-left"
                  dir="ltr"
                  placeholder="يُولَّد تلقائياً عند الخروج من الاسم العربي"
                  value={form.name_en}
                  onChange={(e) => {
                    enTouched.current = true; // تعديل يدوي: يتوقف التوليد التلقائي
                    setForm({ ...form, name_en: e.target.value });
                  }}
                />
                <div className="text-[11px] text-slate-500 mt-1 leading-relaxed">
                  شاشة البصمة لا تدعم العربية — يُرسل هذا الاسم إلى الجهاز عند
                  الحفظ. يُولَّد تلقائياً من الاسم العربي فور الخروج من حقله؛
                  عدّله يدوياً متى شئت وسيتوقف التوليد التلقائي بعده.
                </div>
              </div>
              <div>
                <label className="label">رقم البصمة (PIN على الجهاز)</label>
                <input
                  className="input text-left"
                  dir="ltr"
                  value={form.pin ?? ""}
                  placeholder="يُملأ تلقائياً: أعلى رقم معروف + 1"
                  onChange={(e) => setForm({ ...form, pin: e.target.value })}
                />
                <div className="text-[11px] text-slate-500 mt-1 leading-relaxed">
                  يُعبَّأ وحده عند فتح «+ موظف جديد» بأعلى رقم موجود على الجهاز
                  أو في البرنامج + 1 — ويمكن تعديله يدوياً.
                </div>
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
                <label className="label">الوردية (ساعات الدوام)</label>
                <select
                  className="input"
                  value={form.shift_id ?? ""}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      shift_id: e.target.value ? Number(e.target.value) : null,
                    })
                  }
                >
                  <option value="">
                    — الوردية الرئيسية (الافتراضية) —
                  </option>
                  {shifts
                    .filter((s) => s.active || s.id === form.shift_id)
                    .map((s) => (
                      <option key={s.id} value={s.id}>
                        {s.name} · {s.work_start}–{s.work_end}
                        {s.is_default ? " ⭐" : ""}
                        {!s.active ? " (مهجورة)" : ""}
                      </option>
                    ))}
                </select>
                <p className="text-xs text-slate-500 mt-1">
                  تحدّد الوردية بداية/نهاية الدوام والساعات والاستراحة المحتسبة.
                </p>
              </div>
              <div>
                <label className="label">👑 صلاحية على جهاز البصمة</label>
                <select
                  className="input"
                  value={form.device_privilege}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      device_privilege: Number(e.target.value),
                    })
                  }
                >
                  <option value={0}>مستخدم عادي (يُبصم فقط)</option>
                  <option value={6}>مدير — Manager</option>
                  <option value={14}>أدمن كامل (سوبر أدمن) — Admin</option>
                </select>
                <p className="text-xs text-slate-500 mt-1 leading-relaxed">
                  تُرسل إلى الجهاز عند الحفظ — الأدمن يفتح قوائم إعدادات الجهاز
                  بإدخال بصمته. لا تغيير في الصلاحية يمسّ بصماته.
                </p>
              </div>
              {/* ساعات العمل والاستراحة مأخوذة من الوردية المسنَدة */}
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

            {/* السماح باستخدام بوت تلجرام — نفس القيمة في جدول الموظفين المرتبطين */}
            <div className="mt-3 p-3 rounded-lg bg-slate-50 border border-slate-200 flex items-center justify-between gap-3">
              <div>
                <div className="font-bold text-sm">بوت تلجرام</div>
                <div className="text-xs text-slate-500">
                  المنع يوقف استخدام الأزرار والأوامر فقط، ولا يؤثر على الحضور
                  أو الراتب.
                </div>
              </div>
              <label className="flex items-center gap-2 text-sm shrink-0">
                <input
                  type="checkbox"
                  checked={form.bot_allowed}
                  onChange={(e) =>
                    setForm({ ...form, bot_allowed: e.target.checked })
                  }
                />
                <span
                  className={
                    form.bot_allowed
                      ? "text-emerald-700 font-bold"
                      : "text-rose-700 font-bold"
                  }
                >
                  {form.bot_allowed ? "مسموح" : "ممنوع"}
                </span>
              </label>
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
              <span className="text-xs text-slate-500 mr-auto">
                💡 يُرسَل الموظف إلى جهاز البصمة تلقائياً عند الحفظ (لا حاجة لزر
                إرسال منفصل).
              </span>
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
              <th className="th">صلاحية الجهاز</th>
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
                <td className="td text-center text-slate-400 py-8" colSpan={8}>
                  لا يوجد موظفون — اضغط «+ موظف جديد» أو اسحب من الجهاز.
                </td>
              </tr>
            )}
            {filtered.map((r) => (
              <tr
                key={r.id}
                className={`hover:bg-slate-50 cursor-pointer ${
                  r.active ? "" : "opacity-60"
                }`}
                onClick={() => setViewEmp(r)}
                title="عرض حركات الموظف"
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
                <td className="td">
                  {/* صلاحيته الفعلية على جهاز البصمة (تُحدَّث مع كل سحب من الجهاز) */}
                  {r.device_privilege > 0 ? (
                    <span
                      className={`badge ${
                        r.device_privilege === 14
                          ? "bg-amber-100 text-amber-700"
                          : "bg-indigo-100 text-indigo-700"
                      }`}
                      title={`صلاحية ${privLabel(r.device_privilege)} على جهاز البصمة`}
                    >
                      👑 {privLabel(r.device_privilege)}
                    </span>
                  ) : (
                    <span className="text-slate-300" title="مستخدم عادي على الجهاز">
                      —
                    </span>
                  )}
                </td>
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
                  {!r.bot_allowed && (
                    <span
                      className="badge bg-rose-100 text-rose-700 mr-1"
                      title="ممنوع من استخدام بوت تلجرام"
                    >
                      🚫 البوت
                    </span>
                  )}
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
                      className="text-amber-700 hover:underline text-xs font-bold"
                      onClick={(e) => {
                        e.stopPropagation();
                        setAdvEmp(r);
                      }}
                      title="إدارة وعرض سلف الموظف وأقساطها"
                    >
                      💵 سلف
                    </button>
                    <button
                      className="text-indigo-600 hover:underline text-xs"
                      onClick={(e) => {
                        e.stopPropagation();
                        openForm(r);
                      }}
                    >
                      تعديل
                    </button>
                    <button
                      className="text-rose-600 hover:underline text-xs"
                      onClick={(e) => {
                        e.stopPropagation();
                        remove(r);
                      }}
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

      {/* نافذة عرض وإدارة سلف الموظف */}
      {advEmp && (
        <AdvancesModal
          emp={advEmp}
          curSym={curSym}
          onClose={() => setAdvEmp(null)}
          toast={toast}
        />
      )}
    </div>
  );
}

/** الشهر الحالي بصيغة YYYY-MM (الافتراضي لبداية خصم السلفة). */
const curPeriod = (): string => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

const fmtMoney = (v: number) =>
  v.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

/** نافذة إدارة سلف الموظف: عرض السلف الفعالة التي لم تخصم بعد، والسلف السابقة،
 *  مع إمكانية تعليم أي سلفة/قسط أنها خُصمت، وإنشاء سلفة جديدة (دفعة واحدة على الراتب القادم أو تقسيط على الرواتب القادمة). */
export function AdvancesModal({
  emp,
  curSym,
  onClose,
  toast,
}: {
  emp: Employee;
  curSym: string;
  onClose: () => void;
  toast: Toast;
}) {
  const confirm = useConfirm();
  const [advances, setAdvances] = useState<EmployeeAdvance[]>([]);
  const [busy, setBusy] = useState(false);

  // حقول إنشاء سلفة جديدة
  const [amount, setAmount] = useState("");
  const [mode, setMode] = useState<"single" | "installments">("single");
  const [instCount, setInstCount] = useState(2);
  const [startPeriod, setStartPeriod] = useState(() => curPeriod());
  const [note, setNote] = useState("");
  const [addBusy, setAddBusy] = useState(false);

  const load = async () => {
    setBusy(true);
    try {
      setAdvances(await listEmployeeAdvances(emp.id));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [emp.id]);

  const createAdvance = async () => {
    const amt = Number(amount);
    if (!Number.isFinite(amt) || amt <= 0) {
      toast("أدخل مبلغ سلفة صحيح أكبر من صفر", "err");
      return;
    }
    if (!startPeriod) {
      toast("حدد شهر بداية الخصم", "err");
      return;
    }
    const cnt = mode === "single" ? 1 : Math.max(2, Math.floor(instCount));
    setAddBusy(true);
    try {
      await addEmployeeAdvance(emp.id, amt, mode, cnt, startPeriod, note);
      toast(
        mode === "single"
          ? `تم تسجيل سلفة بمبلغ ${fmtMoney(amt)} ${curSym} تُخصم على راتب ${startPeriod} ✅`
          : `تم تسجيل سلفة بمبلغ ${fmtMoney(amt)} ${curSym} مقسّطة على ${cnt} رواتب ابتداءً من ${startPeriod} ✅`,
      );
      setAmount("");
      setNote("");
      await load();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setAddBusy(false);
    }
  };

  const toggleInst = async (instId: number, currentDeducted: boolean, period: string) => {
    try {
      await toggleAdvanceInstallment(instId, !currentDeducted);
      toast(
        !currentDeducted
          ? `تم تعليم سلفة/قسط شهر ${period} أنه خُصم ✓`
          : `أُعيدت سلفة/قسط شهر ${period} إلى فعّالة (لم تُخصم بعد)`,
      );
      await load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const removeAdvance = async (adv: EmployeeAdvance) => {
    const ok = await confirm(
      `حذف السلفة البالغة ${fmtMoney(adv.amount)} ${curSym} وجميع أقساطها نهائياً؟`,
      { title: "حذف سلفة", yes: "نعم، احذف" },
    );
    if (!ok) return;
    try {
      await deleteEmployeeAdvance(adv.id);
      toast("تم حذف السلفة");
      await load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const activeAdvances = advances.filter((a) => a.active);
  const totalRemaining = advances.reduce((s, a) => s + a.remaining_amount, 0);
  const totalDeducted = advances.reduce((s, a) => s + a.deducted_amount, 0);

  const previewPerMonth =
    mode === "installments" && Number(amount) > 0 && instCount >= 2
      ? fmtMoney(Number(amount) / instCount)
      : null;

  return (
    <Modal
      onClose={onClose}
      title={
        <span>
          💵 سلف الموظف: {emp.name}
          <span className="text-sm text-slate-500 font-normal">
            {" "}
            (رقم {emp.pin || "—"})
          </span>
        </span>
      }
      panelClass="max-w-3xl"
    >
      {/* ملخص السلف */}
      <div className="grid grid-cols-3 gap-3 mb-4">
        <div className="rounded-lg border border-amber-200 bg-amber-50 p-3">
          <div className="text-xs text-amber-700 font-bold">
            السلف الفعالة (المتبقي غير المخصوم)
          </div>
          <div className="text-xl font-extrabold text-amber-800 mt-0.5">
            {fmtMoney(totalRemaining)} {curSym}
          </div>
          <div className="text-[11px] text-amber-600">
            {activeAdvances.length} سلفة فعالة لم تُخصم بالكامل
          </div>
        </div>
        <div className="rounded-lg border border-emerald-200 bg-emerald-50 p-3">
          <div className="text-xs text-emerald-700 font-bold">
            إجمالي ما تم خصمه
          </div>
          <div className="text-xl font-extrabold text-emerald-800 mt-0.5">
            {fmtMoney(totalDeducted)} {curSym}
          </div>
        </div>
        <div className="rounded-lg border border-slate-200 bg-slate-50 p-3">
          <div className="text-xs text-slate-500 font-bold">
            إجمالي السلف المسجلة
          </div>
          <div className="text-xl font-extrabold text-slate-800 mt-0.5">
            {advances.length} سلفة
          </div>
        </div>
      </div>

      {/* نموذج إنشاء سلفة جديدة */}
      <div className="p-4 rounded-xl bg-slate-50 border border-slate-200 space-y-3 mb-5">
        <div className="font-extrabold text-sm text-slate-800">
          ➕ إنشاء سلفة جديدة
        </div>
        <div className="grid sm:grid-cols-3 gap-3">
          <div>
            <label className="label">مبلغ السلفة ({curSym}) *</label>
            <input
              type="number"
              min={0}
              step="10"
              className="input text-left"
              dir="ltr"
              placeholder="مثال: 500"
              value={amount}
              onChange={(e) => setAmount(e.target.value)}
            />
          </div>
          <div>
            <label className="label">طريقة خصم السلفة *</label>
            <select
              className="input"
              value={mode}
              onChange={(e) =>
                setMode(e.target.value as "single" | "installments")
              }
            >
              <option value="single">خصم كامل على الراتب القادم</option>
              <option value="installments">تقسيط على الرواتب القادمة</option>
            </select>
          </div>
          <div>
            <label className="label">
              {mode === "single" ? "شهر الخصم (الراتب)" : "أول شهر يبدأ فيه الخصم"}
            </label>
            <input
              type="month"
              className="input"
              value={startPeriod}
              onChange={(e) => setStartPeriod(e.target.value)}
            />
          </div>
        </div>

        {mode === "installments" && (
          <div className="grid sm:grid-cols-2 gap-3 items-end pt-1">
            <div>
              <label className="label">عدد الأقساط (الأشهر) *</label>
              <input
                type="number"
                min={2}
                max={60}
                className="input text-left"
                dir="ltr"
                value={instCount}
                onChange={(e) =>
                  setInstCount(Math.max(2, Number(e.target.value) || 2))
                }
              />
            </div>
            <div className="text-xs text-indigo-700 bg-indigo-50 border border-indigo-200 rounded-lg p-2.5">
              {previewPerMonth ? (
                <>
                  قيمة القسط الشهري:{" "}
                  <b>
                    {previewPerMonth} {curSym}
                  </b>{" "}
                  لمدة <b>{instCount}</b> أشهر ابتداءً من <b>{startPeriod}</b>
                </>
              ) : (
                "أدخل مبلغ السلفة لمعاينة قيمة القسط الشهري"
              )}
            </div>
          </div>
        )}

        <div className="flex flex-wrap items-end gap-3">
          <div className="flex-1 min-w-[200px]">
            <label className="label">ملاحظة (اختياري)</label>
            <input
              className="input"
              placeholder="مثال: سلفة نقدية طارئة…"
              value={note}
              onChange={(e) => setNote(e.target.value)}
            />
          </div>
          <button
            className="btn-primary"
            onClick={createAdvance}
            disabled={addBusy}
          >
            {addBusy ? "جارٍ الحفظ…" : "💾 إضافة السلفة"}
          </button>
        </div>
      </div>

      {/* السلف المسجلة (الفعالة والسابقة) */}
      <div className="flex items-center justify-between mb-2">
        <h3 className="font-extrabold text-sm">
          📋 السلف المسجلة للموظف ({advances.length})
        </h3>
        {busy && <span className="text-xs text-slate-400">جارٍ التحميل…</span>}
      </div>

      {advances.length === 0 ? (
        <div className="p-6 text-center text-sm text-slate-400 border border-dashed border-slate-200 rounded-lg">
          لا توجد سلف مسجلة لهذا الموظف بعد.
        </div>
      ) : (
        <div className="space-y-3 max-h-96 overflow-y-auto pr-1">
          {advances.map((adv) => (
            <div
              key={adv.id}
              className={`rounded-xl border p-3.5 space-y-2.5 ${
                adv.active
                  ? "border-amber-300 bg-amber-50/40"
                  : "border-slate-200 bg-white"
              }`}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span
                    className={`badge font-bold ${
                      adv.active
                        ? "bg-amber-100 text-amber-800"
                        : "bg-emerald-100 text-emerald-700"
                    }`}
                  >
                    {adv.active ? "⏳ سلفة فعالة (لم تُخصم بالكامل)" : "✓ مخصومة بالكامل"}
                  </span>
                  <span className="font-extrabold text-base">
                    {fmtMoney(adv.amount)} {curSym}
                  </span>
                  <span className="badge bg-slate-100 text-slate-700">
                    {adv.mode === "installments"
                      ? `تقسيط (${adv.installments_count} دفعات)`
                      : "دفعة واحدة على الراتب القادم"}
                  </span>
                  <span className="text-xs text-slate-500 font-mono">
                    تاريخ الإنشاء: {adv.created_at.slice(0, 10)}
                  </span>
                </div>
                <button
                  className="text-xs text-rose-600 hover:underline font-bold"
                  onClick={() => removeAdvance(adv)}
                  title="حذف السلفة"
                >
                  🗑 حذف
                </button>
              </div>

              {adv.note && (
                <div className="text-xs text-slate-600">
                  📝 ملاحظة: {adv.note}
                </div>
              )}

              <div className="text-xs text-slate-600 flex gap-4">
                <span>
                  المخصوم:{" "}
                  <b className="text-emerald-700">
                    {fmtMoney(adv.deducted_amount)} {curSym}
                  </b>
                </span>
                <span>
                  المتبقي:{" "}
                  <b className="text-rose-600">
                    {fmtMoney(adv.remaining_amount)} {curSym}
                  </b>
                </span>
              </div>

              {/* جدول أقساط السلفة */}
              <div className="border border-slate-200 rounded-lg overflow-hidden bg-white">
                <table className="w-full text-xs">
                  <thead>
                    <tr className="bg-slate-100">
                      <th className="py-1.5 px-2 text-right">#</th>
                      <th className="py-1.5 px-2 text-right">شهر الراتب</th>
                      <th className="py-1.5 px-2 text-right">المبلغ ({curSym})</th>
                      <th className="py-1.5 px-2 text-right">الحالة</th>
                      <th className="py-1.5 px-2 text-center">إجراء</th>
                    </tr>
                  </thead>
                  <tbody>
                    {adv.installments.map((inst, idx) => (
                      <tr
                        key={inst.id}
                        className="border-t border-slate-100 hover:bg-slate-50"
                      >
                        <td className="py-1.5 px-2 text-slate-400">{idx + 1}</td>
                        <td className="py-1.5 px-2 font-mono font-bold">
                          {inst.period}
                        </td>
                        <td className="py-1.5 px-2 font-bold">
                          {fmtMoney(inst.amount)} {curSym}
                        </td>
                        <td className="py-1.5 px-2">
                          {inst.deducted ? (
                            <span className="badge bg-emerald-100 text-emerald-700">
                              ✓ تم الخصم
                              {inst.deducted_at
                                ? ` (${inst.deducted_at.slice(0, 10)})`
                                : ""}
                            </span>
                          ) : (
                            <span className="badge bg-amber-100 text-amber-800">
                              فعالة — لم تُخصم بعد
                            </span>
                          )}
                        </td>
                        <td className="py-1.5 px-2 text-center">
                          <button
                            className={`px-2.5 py-1 rounded text-xs font-bold border transition ${
                              inst.deducted
                                ? "bg-white text-slate-600 border-slate-300 hover:bg-slate-100"
                                : "bg-emerald-600 text-white border-emerald-600 hover:bg-emerald-700"
                            }`}
                            onClick={() =>
                              toggleInst(inst.id, inst.deducted, inst.period)
                            }
                          >
                            {inst.deducted
                              ? "↩ إلغاء الخصم"
                              : "✓ تعليم أنها خُصمت"}
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          ))}
        </div>
      )}
    </Modal>
  );
}

/** عناوين أنواع الإجازات بالعربية. */
const LEAVE_LABELS: Record<string, string> = {
  annual: "إجازة سنوية",
  sick: "إجازة مرضية",
  comp: "عطلة تعويضية (يوم بدل)",
};

/** عدد أيام المدى [من، إلى] بما فيه اليومان (0 إن كان التاريخ غير صالح). */
const dayCount = (a: string, b: string): number => {
  const t1 = Date.parse(a);
  const t2 = Date.parse(b);
  if (Number.isNaN(t1) || Number.isNaN(t2)) return 0;
  return Math.round((t2 - t1) / 86400000) + 1;
};

/** نافذة عرض كل حركات موظف (دخول/خروج/استراحة/إضافي) + إجازاته، مع بحث. */
export function PunchesModal({
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
  // تأكيد «نعم/لا» داخلي (حذف الإجازات) — لا يستجيب للنقرة العابرة
  const confirm = useConfirm();
  // نطاق تلقائي: من أول الشهر الحالي إلى آخره (قابل للتغيير يدوياً)
  const [from, setFrom] = useState(() => curMonth()[0]);
  const [to, setTo] = useState(() => curMonth()[1]);
  const [qtime, setQtime] = useState(""); // بحث بالوقت HH:MM أو جزء التاريخ
  // العرض المدمج (الافتراضي): كل دخول مع خروجه في صف واحد + المدة بينهما
  const [grouped, setGrouped] = useState(true);
  const [punches, setPunches] = useState<PunchRow[]>([]);
  const [leaves, setLeaves] = useState<Leave[]>([]);
  const [busy, setBusy] = useState(false);
  // لترويسة الطباعة: نص الشركة وشعارها
  const [cfg, setCfg] = useState<Settings>({});
  const [logo, setLogo] = useState("");

  // رصيد الإجازات السنوية (يتحدث محلياً فورياً بعد تسجيل/حذف إجازة سنوية)
  const [balance, setBalance] = useState(emp.annual_leave_balance);
  // نموذج إجازة جديدة
  const [lvType, setLvType] = useState("annual");
  const [lvPaid, setLvPaid] = useState(true);
  const [lvFrom, setLvFrom] = useState("");
  const [lvTo, setLvTo] = useState("");
  const [lvNote, setLvNote] = useState("");
  const [lvBusy, setLvBusy] = useState(false);

  // «يوم بدل»: عطلة رسمية دوّم فيها الموظف → عطلة تعويضية في تاريخ آخر
  const [holidays, setHolidays] = useState<Holiday[]>([]);
  const [compOpen, setCompOpen] = useState(false);
  const [compHoliday, setCompHoliday] = useState("");
  const [compDate, setCompDate] = useState("");
  const [compBusy, setCompBusy] = useState(false);

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
    holidaysList().then(setHolidays).catch(() => {});
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** إسناد «يوم بدل» عن دوام الموظف في عطلة رسمية. */
  const assignComp = async () => {
    if (!compHoliday) {
      toast("اختر يوم العطلة الرسمية التي دوّم فيها", "err");
      return;
    }
    if (!compDate) {
      toast("اختر تاريخ يوم البدل", "err");
      return;
    }
    setCompBusy(true);
    try {
      await assignCompOff(emp.id, compHoliday, compDate);
      toast(`أُسند يوم بدل بتاريخ ${compDate} ✅`);
      setCompOpen(false);
      setCompHoliday("");
      setCompDate("");
      reloadLeaves();
      onChanged?.();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setCompBusy(false);
    }
  };

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
    const ok = await confirm(
      `حذف ${label} (${l.from_date} ← ${l.to_date})؟${
        restore ? " تُستعاد أيامها إلى الرصيد." : ""
      }`,
      { title: "حذف إجازة", yes: "نعم، احذف" },
    );
    if (!ok) return;
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

  // دمج الحركات المعروضة: كل دخول مع خروجه في صف واحد + المدة بينهما (لكل يوم)
  const pairs = pairPunches(shown);
  const totalMin = pairs.reduce((s, r) => s + (r.minutes ?? 0), 0);

  /** وصف صف المدمج: ينبه للحالات الشاذة وللأنواع غير القياسية (استراحة/إضافي). */
  const pairNote = (r: PunchPairRow) => {
    if (!r.inTime) return "⚠️ خروج بلا دخول";
    if (!r.outTime) return "⚠️ دخول بلا خروج";
    if (r.inType !== 0 || r.outType !== 1)
      return `${PUNCH_LABELS[r.inType ?? -1] ?? "؟"} ← ${
        PUNCH_LABELS[r.outType ?? -1] ?? "؟"
      }`;
    return "—";
  };

  /** طباعة الحركات المعروضة (ضمن الترشيح) بترويسة الشركة وتذييلها. */
  const printPunches = () => {
    if (shown.length === 0) {
      toast("لا حركات ضمن الترشيح الحالي — عدّل الفترة أو امسح الترشيح", "err");
      return;
    }
    document.body.classList.add("print-punches");
    try {
      window.print();
    } finally {
      document.body.classList.remove("print-punches");
    }
  };

  /** استخراج الحركات المعروضة إلى ملف Excel (CSV بترميز UTF-8 يفتح مباشرة). */
  const exportExcel = () => {
    if (shown.length === 0) {
      toast("لا حركات ضمن الترشيح الحالي — عدّل الفترة أو امسح الترشيح", "err");
      return;
    }
    const rows: string[][] = grouped
      ? [
          ["#", "التاريخ", "الدخول", "الخروج", "المدة", "ملاحظة"],
          ...pairs.map((r, i) => [
            String(i + 1),
            r.date,
            r.inTime ?? "",
            r.outTime ?? "",
            fmtDur(r.minutes),
            pairNote(r),
          ]),
          ["", "الإجمالي", "", "", fmtDur(totalMin), ""],
        ]
      : [
          ["#", "التاريخ", "الوقت", "النوع", "رمز الجهاز"],
          ...shown.map((p, i) => {
            const [d, t] = p.device_time.split("T");
            return [
              String(i + 1),
              d,
              (t ?? "").slice(0, 8),
              PUNCH_LABELS[p.punch_type] ?? `نوع ${p.punch_type}`,
              `${p.raw_status}/${p.work_code}`,
            ];
          }),
        ];
    const csv =
      "\uFEFF" + rows.map((r) => r.map(csvCell).join(",")).join("\r\n");
    const url = URL.createObjectURL(
      new Blob([csv], { type: "text/csv;charset=utf-8;" }),
    );
    const a = document.createElement("a");
    a.href = url;
    a.download = `حركات_${emp.name}_${emp.pin || emp.id}.csv`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    toast("جُهّز ملف Excel — يُفتح مباشرة في Excel ✅");
  };

  return (
    <>
    <style>{printCss}</style>
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
          {emp.device_privilege > 0 && (
            <span className="badge bg-amber-100 text-amber-700 mr-2 align-middle">
              👑 {privLabel(emp.device_privilege)} على جهاز البصمة
            </span>
          )}
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
          <button className="btn-ghost" onClick={printPunches}>
            🖨️ طباعة الحركات
          </button>
          <button
            className="btn-ghost"
            onClick={exportExcel}
            title="تنزيل الحركات المعروضة (ضمن الترشيح) كملف يفتح مباشرة في Excel"
          >
            ⬇️ استخراج Excel
          </button>
          <button
            className={`btn-ghost ${
              grouped ? "!bg-indigo-100 !text-indigo-700 !border !border-indigo-300" : ""
            }`}
            onClick={() => setGrouped((g) => !g)}
            title={
              grouped
                ? "إلغاء الدمج: عرض كل حركة على حدة كما سجّلها الجهاز"
                : "دمج كل دخول مع خروجه في صف واحد ويظهر عدد الساعات بينهما"
            }
          >
            {grouped ? "🔗 دخول/خروج مدموج" : "🔗 دمج دخول/خروج"}
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
          المعروض: {shown.length} حركة من {punches.length}
          {grouped && (
            <>
              {" "}
              — {pairs.length} صف دخول/خروج · إجمالي المدة:{" "}
              <b>{fmtDur(totalMin)}</b>
            </>
          )}
        </div>

        <div className="overflow-auto max-h-80 border border-slate-200 rounded-lg">
          <table className="w-full">
            <thead className="sticky top-0">
              {grouped ? (
                <tr>
                  <th className="th">#</th>
                  <th className="th">التاريخ</th>
                  <th className="th">الدخول</th>
                  <th className="th">الخروج</th>
                  <th className="th">المدة</th>
                  <th className="th">ملاحظة</th>
                </tr>
              ) : (
                <tr>
                  <th className="th">التاريخ</th>
                  <th className="th">الوقت</th>
                  <th className="th">النوع</th>
                  <th className="th">رمز الجهاز</th>
                </tr>
              )}
            </thead>
            <tbody>
              {shown.length === 0 && (
                <tr>
                  <td className="td text-center text-slate-400 py-6" colSpan={6}>
                    لا حركات ضمن هذا الترشيح.
                  </td>
                </tr>
              )}
              {grouped &&
                pairs.map((r, i) => (
                  <tr key={i} className="hover:bg-slate-50">
                    <td className="td text-slate-400">{i + 1}</td>
                    <td className="td font-mono">{r.date}</td>
                    <td className="td font-mono">{r.inTime ?? "—"}</td>
                    <td className="td font-mono">{r.outTime ?? "—"}</td>
                    <td className="td font-bold">{fmtDur(r.minutes)}</td>
                    <td
                      className={`td text-xs ${
                        r.inTime && r.outTime
                          ? "text-slate-500"
                          : "text-amber-600 font-bold"
                      }`}
                    >
                      {pairNote(r)}
                    </td>
                  </tr>
                ))}
              {!grouped &&
                shown.map((p, i) => {
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

        {/* يوم بدل: عطلة تعويضية عن دوام في عطلة رسمية */}
        <div className="mt-5 pt-4 border-t border-slate-200">
          <div className="flex items-center justify-between">
            <h3 className="font-bold">🎁 يوم بدل (عطلة تعويضية)</h3>
            <button
              className="btn-ghost !py-1 !px-2 text-xs"
              onClick={() => setCompOpen((o) => !o)}
            >
              {compOpen ? "إخفاء" : "➕ إسناد يوم بدل"}
            </button>
          </div>
          <p className="text-xs text-slate-500 mt-1 leading-relaxed">
            إن دوّم الموظف في <b>عطلة رسمية</b>، يمكن منحه يوماً بديلاً إجازةً مدفوعة
            بأجر يوم العطلة (62) — ويمكن أن يكون في الشهر التالي. يُحتسب ذلك في الراتب
            كـ«عطلة تعويضية».
          </p>
          {compOpen && (
            <div className="bg-slate-50 p-3 rounded-lg border border-slate-200 space-y-2 text-sm mt-2">
              <div className="grid sm:grid-cols-2 gap-2">
                <div>
                  <label className="label">العطلة الرسمية التي دوّم فيها</label>
                  <select
                    className="input"
                    value={compHoliday}
                    onChange={(e) => setCompHoliday(e.target.value)}
                  >
                    <option value="">— اختر —</option>
                    {holidays.map((h) => (
                      <option key={h.id} value={h.date}>
                        {h.date} — {h.name}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="label">تاريخ يوم البدل</label>
                  <input
                    type="date"
                    className="input"
                    value={compDate}
                    onChange={(e) => setCompDate(e.target.value)}
                  />
                </div>
              </div>
              {holidays.length === 0 && (
                <p className="text-xs text-amber-600">
                  لا توجد إجازات رسمية مسجّلة — أضفها من الإعدادات أولاً.
                </p>
              )}
              <div className="flex justify-end">
                <button
                  className="btn-primary"
                  onClick={assignComp}
                  disabled={compBusy || !compHoliday || !compDate}
                >
                  {compBusy ? "جارٍ الإسناد…" : "🎁 إسناد يوم البدل"}
                </button>
              </div>
            </div>
          )}
        </div>
    </Modal>

    {/* ===== ورقة طباعة الحركات (تُطبع بزر «طباعة الحركات» — مخفية على الشاشة) ===== */}
    {createPortal(
      <div className="punches-sheet report-sheet p-8">
        {(
          grouped ? chunk(pairs, ROWS_PER_PAGE) : chunk(shown, ROWS_PER_PAGE)
        ).map((rows, pi, pages) => (
          <div className="print-page" key={pi}>
            <PrintHeader
          cfg={cfg}
          logo={logo}
          title={`حركات موظف — ${emp.name}`}
          meta={
            <>
              <div className="text-slate-600">
                رقم البصمة: {emp.pin || "—"}
                {emp.job_title ? ` · ${emp.job_title}` : ""}
              </div>
              <div className="text-slate-600">
                الفترة: {from || "الكل"} ← {to || "الكل"} ·{" "}
                {grouped
                  ? `صفوف دخول/خروج: ${pairs.length} · إجمالي المدة: ${fmtDur(
                      totalMin,
                    )}`
                  : `عدد الحركات: ${shown.length}`}
                {pages.length > 1
                  ? ` · صفحة ${pi + 1} من ${pages.length}`
                  : ""}
              </div>
              <div className="text-slate-500 text-xs">
                تاريخ الطباعة: {todayISO()}
              </div>
            </>
          }
        />
        <table className="report-table">
          <thead>
            {grouped ? (
              <tr>
                <th>#</th>
                <th>التاريخ</th>
                <th>الدخول</th>
                <th>الخروج</th>
                <th>المدة</th>
                <th>ملاحظة</th>
              </tr>
            ) : (
              <tr>
                <th>#</th>
                <th>التاريخ</th>
                <th>الوقت</th>
                <th>النوع</th>
                <th>رمز الجهاز</th>
              </tr>
            )}
          </thead>
          <tbody>
            {grouped
              ? (rows as PunchPairRow[]).map((r, i) => (
                  <tr key={i}>
                    <td>{pi * ROWS_PER_PAGE + i + 1}</td>
                    <td className="font-mono">{r.date}</td>
                    <td className="font-mono">{r.inTime ?? "—"}</td>
                    <td className="font-mono">{r.outTime ?? "—"}</td>
                    <td className="font-bold">{fmtDur(r.minutes)}</td>
                    {/* الملاحظة تُطبع فارغة ليُملأ يدوياً على الورق */}
                    <td className="text-xs"></td>
                  </tr>
                ))
              : (rows as PunchRow[]).map((p, i) => {
                  const [d, t] = p.device_time.split("T");
                  return (
                    <tr key={i}>
                      <td>{pi * ROWS_PER_PAGE + i + 1}</td>
                      <td className="font-mono">{d}</td>
                      <td className="font-mono">{(t ?? "").slice(0, 8)}</td>
                      <td>
                        {PUNCH_LABELS[p.punch_type] ?? `نوع ${p.punch_type}`}
                      </td>
                      <td className="text-xs text-slate-500 font-mono">
                        {p.raw_status}/{p.work_code}
                      </td>
                    </tr>
                  );
                })}
          </tbody>
        </table>
        {pi === pages.length - 1 && (
          <div className="grid grid-cols-2 gap-8 mt-10 text-sm text-center">
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">توقيع الموظف</div>
            </div>
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">مسؤول الرواتب</div>
            </div>
          </div>
        )}
        <PrintFooter cfg={cfg} />
          </div>
        ))}
      </div>,
      document.body,
    )}
    </>
  );
}
