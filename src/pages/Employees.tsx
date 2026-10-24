import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import Modal from "../components/Modal";
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
} from "../lib/api";
import type { PunchRow } from "../lib/api";
import { translitArabic } from "../lib/translit";
import { printCss, todayISO } from "../lib/print";
import { pairPunches, fmtDur } from "../lib/punchPairs";
import type { PunchPairRow } from "../lib/punchPairs";
import type { Employee, Leave, Holiday, Shift, Settings } from "../lib/types";

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
  const [rows, setRows] = useState<Employee[]>([]);
  const [form, setForm] = useState<Employee | null>(null);
  const [q, setQ] = useState("");
  const [status, setStatus] = useState<"all" | "active" | "inactive">("all");
  const [cur, setCur] = useState("ILS");
  const [viewEmp, setViewEmp] = useState<Employee | null>(null);
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
      enTouched.current = true;
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

  // توليد تلقائي عند الخروج من حقل الاسم العربي (مع تأخير خفيف لضمان تحديث القيمة)
  const autoGenEnOnBlur = () => {
    setForm((f) => {
      if (!f || enTouched.current) return f;
      const auto = translitArabic(f.name);
      if (!auto.trim()) return f;
      if (f.name_en && f.name_en !== lastAutoEn.current) return f;
      lastAutoEn.current = auto;
      return { ...f, name_en: auto };
    });
  };

  // توليد يدوي صريح من زر 🔄 (يعمل للإضافة والتعديل معاً)
  const genEn = () => {
    if (!form) return;
    enTouched.current = false;
    const auto = translitArabic(form.name);
    lastAutoEn.current = auto;
    setForm({ ...form, name_en: auto });
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
      const ok = window.confirm(
        `إيقاف الموظف «${form.name}» سيحذف بصمه من جهاز البصمة فلا يعود يُبصم.\n` +
          `سجل حضوره السابق يبقى كاملاً في البرنامج.\n` +
          `وعند إعادة تفعيله لاحقاً يجب إعادة تعريفه على الجهاز وتسجيل بصماته من جديد.\n\n` +
          `هل تريد المتابعة؟`,
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
                  onBlur={autoGenEnOnBlur}
                  onKeyDown={(e) => {
                    if (e.key === "Tab") {
                      setTimeout(autoGenEnOnBlur, 50);
                    }
                  }}
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
    </div>
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
              ? pairs.map((r, i) => (
                  <tr key={i}>
                    <td>{i + 1}</td>
                    <td className="font-mono">{r.date}</td>
                    <td className="font-mono">{r.inTime ?? "—"}</td>
                    <td className="font-mono">{r.outTime ?? "—"}</td>
                    <td className="font-bold">{fmtDur(r.minutes)}</td>
                    <td
                      className={
                        r.inTime && r.outTime
                          ? "text-xs text-slate-500"
                          : "text-xs font-bold text-amber-700"
                      }
                    >
                      {pairNote(r)}
                    </td>
                  </tr>
                ))
              : shown.map((p, i) => {
                  const [d, t] = p.device_time.split("T");
                  return (
                    <tr key={i}>
                      <td>{i + 1}</td>
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
        <PrintFooter cfg={cfg} />
      </div>,
      document.body,
    )}
    </>
  );
}
