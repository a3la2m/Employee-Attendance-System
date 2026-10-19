import { useEffect, useState } from "react";
import {
  getSettings,
  saveSettingsBulk,
  setCompanyLogo,
  getStorageInfo,
  setStorageDir,
  backupNow,
  restoreSettings,
  holidaysList,
  holidayAdd,
  holidayDelete,
  accrueLeaveNow,
} from "../lib/api";
import type { Holiday, Settings, StorageInfo } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

// كل مفاتيح الرواتب القابلة للتعديل من الواجهة
const PAYROLL_KEYS = [
  {
    k: "daily_hours",
    label: "ساعات الدوام الأساسية (منها تُخصم الاستراحة — ما زاد عنها إضافي)",
    step: "0.25",
  },
  { k: "break_min", label: "فترة الخصم (استراحة) بالدقائق", step: "1" },
  { k: "hourly_rate", label: "سعر الساعة", step: "0.5" },
  { k: "meal_allow", label: "بدل الفطور اليومي", step: "0.5" },
  { k: "transport_allow", label: "بدل المواصلات اليومي", step: "0.5" },
  { k: "overtime_rate_mult", label: "مضاعف العمل الإضافي", step: "0.1" },
  { k: "grace_late_min", label: "مهلة التأخير (دقيقة) — للتنبيهات فقط", step: "1" },
  { k: "weekend_allow", label: "بدل العطلة (62 — مبلغ الأسبوع في الطريقة الأسبوعية، أو مبلغ كل يوم عطلة في الطريقة القديمة)", step: "1" },
];

const TIME_KEYS = [
  { k: "work_start", label: "بداية الدوام (للتنبيهات فقط)", type: "text", ph: "HH:MM" },
  { k: "work_end", label: "نهاية الدوام (للتنبيهات فقط)", type: "text", ph: "HH:MM" },
  { k: "timezone", label: "المنطقة الزمنية", type: "text", ph: "Asia/Jerusalem" },
  { k: "weekend_days", label: "عطلة الأسبوع (4=جمعة — الأسبوع من السبت إلى الجمعة)", type: "text", ph: "4" },
  { k: "auto_sync_min", label: "مزامنة تلقائية كل (دقيقة)", type: "number", ph: "5" },
];

export default function SettingsPage({ toast }: { toast: Toast }) {
  const [cfg, setCfg] = useState<Settings>({});
  const [autostart, setAutostart] = useState(false);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [busyStorage, setBusyStorage] = useState(false);
  // الإجازات الرسمية (الأعياد) — مدفوعة لكل الموظفين
  const [holidays, setHolidays] = useState<Holiday[]>([]);
  const [hDate, setHDate] = useState("");
  const [hName, setHName] = useState("");
  const [hBusy, setHBusy] = useState(false);
  // الاحتساب الشهري لأرصدة الإجازات السنوية
  const [accrualBusy, setAccrualBusy] = useState(false);

  const load = async () => {
    try {
      setCfg(await getSettings());
    } catch (e) {
      toast(String(e), "err");
    }
    getStorageInfo().then(setStorage).catch(() => {});
    holidaysList().then(setHolidays).catch(() => {});
  };

  useEffect(() => {
    load();
    import("@tauri-apps/plugin-autostart")
      .then((m) => m.isEnabled())
      .then(setAutostart)
      .catch(() => {});
  }, []);

  const toggleAutostart = async () => {
    setAutostartBusy(true);
    try {
      const m = await import("@tauri-apps/plugin-autostart");
      if (autostart) {
        await m.disable();
        setAutostart(false);
        toast("أُلغي التشغيل التلقائي عند بدء النظام");
      } else {
        await m.enable();
        setAutostart(true);
        toast("سيعمل التطبيق تلقائياً عند بدء النظام (في الخلفية) ✅", "ok");
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setAutostartBusy(false);
    }
  };

  const set = (k: string, v: string) => setCfg((c) => ({ ...c, [k]: v }));
  const curSym =
    cfg.currency === "USD" ? "$" : cfg.currency === "JOD" ? "د.أ" : "₪";

  const saveAll = async () => {
    try {
      await saveSettingsBulk(cfg);
      toast("تم حفظ جميع الإعدادات ✅");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // اختيار مجلد التخزين (مثلاً مجلد Google Drive المتزامن)
  const chooseStorageDir = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({ directory: true, multiple: false, title: "اختر مجلد التخزين" });
      if (typeof dir === "string") {
        setBusyStorage(true);
        const info = await setStorageDir(dir);
        setStorage(info);
        toast(`نُقل التخزين إلى: ${info.storage_dir}`);
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyStorage(false);
    }
  };

  // نسخة احتياطية متكاملة (قاعدة + إعدادات) في مجلد التخزين
  const doBackup = async () => {
    setBusyStorage(true);
    try {
      const r = await backupNow();
      toast(`تم النسخ الاحتياطي ✅\n${r.db_backup}`, "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyStorage(false);
    }
  };

  const doRestore = async () => {
    if (!confirm("استعادة الإعدادات من attendance-settings.json فوق الإعدادات الحالية؟"))
      return;
    setBusyStorage(true);
    try {
      const n = await restoreSettings();
      toast(`تمت استعادة ${n} إعداداً — أعد تحميل الصفحة`);
      load();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyStorage(false);
    }
  };

  /** إضافة إجازة رسمية (عيد): يُعاد احتساب ذلك اليوم لكل الموظفين فوراً. */
  const addHoliday = async () => {
    if (!hDate) return toast("حدد تاريخ الإجازة الرسمية (YYYY-MM-DD)", "err");
    setHBusy(true);
    try {
      await holidayAdd(hDate, hName);
      toast("أُضيفت الإجازة الرسمية وأُعيد احتساب هذا اليوم لكل الموظفين");
      setHDate("");
      setHName("");
      setHolidays(await holidaysList());
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setHBusy(false);
    }
  };

  const removeHoliday = async (h: Holiday) => {
    if (
      !window.confirm(
        `حذف الإجازة الرسمية ${h.date}${h.name ? " — " + h.name : ""}؟${
          " يُعاد احتساب اليوم لكل الموظفين."
        }`,
      )
    )
      return;
    try {
      await holidayDelete(h.id);
      toast("حُذفت الإجازة الرسمية وأُعيد احتساب اليوم");
      setHolidays(await holidaysList());
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // الاحتساب الفوري لأرصدة الإجازات السنوية (زر يدوي — يعمل يومياً تلقائياً أيضاً)
  const runAccrual = async () => {
    setAccrualBusy(true);
    try {
      const r = await accrueLeaveNow();
      if (!r.enabled) {
        toast("الإضافة التلقائية لأرصدة الإجازات مُعطّلة من الإعدادات", "err");
      } else if (r.accrued === 0) {
        toast(`لا يوجد شيء يُضاف — آخر شهر محتسب: ${r.month ?? "—"}`);
      } else {
        const names = (r.details ?? [])
          .map((d) => `${d.name} (+${d.days.toFixed(2)})`)
          .join("، ");
        toast(
          `أُضيف ${r.days.toFixed(2)} يوماً لـ${r.accrued} موظفاً عن شهر ${r.month} ✅\n${names}`,
          "ok",
        );
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setAccrualBusy(false);
    }
  };

  const pickLogo = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const file = await open({ multiple: false, filters: [{ name: "صورة", extensions: ["png", "jpg", "jpeg", "svg"] }] });
      if (typeof file === "string") {
        await setCompanyLogo(file);
        toast("تم اختيار الشعار — سيظهر في التقارير المطبوعة");
        setCfg((c) => ({ ...c, company_logo: file }));
      }
    } catch {
      toast("تعذّر فتح نافذة اختيار الملف (أضف تبعية plugin-dialog)", "err");
    }
  };

  return (
    <div className="p-6 space-y-5 max-w-4xl">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">الإعدادات</h1>
        <button className="btn-primary" onClick={saveAll}>
          💾 حفظ الكل
        </button>
      </div>

      {/* بيانات الشركة والشعار */}
      <div className="card">
        <h2 className="font-bold mb-3">🏢 بيانات الشركة (لتقارير الطباعة)</h2>
        <div className="grid sm:grid-cols-2 gap-4">
          <div>
            <label className="label">اسم الشركة</label>
            <input
              className="input"
              value={cfg.company_name || ""}
              onChange={(e) => set("company_name", e.target.value)}
            />
          </div>
          <div>
            <label className="label">شعار الشركة (يظهر في الترويسة)</label>
            <div className="flex gap-2">
              <input
                className="input text-xs"
                value={cfg.company_logo || ""}
                readOnly
                placeholder="لم يُختار"
              />
              <button className="btn-ghost" onClick={pickLogo}>
                اختيار…
              </button>
            </div>
          </div>
        </div>
      </div>

      {/* إعدادات الطباعة العامة */}
      <div className="card border-indigo-200 bg-indigo-50/40">
        <h2 className="font-bold mb-1">🖨️ إعدادات الطباعة (لكل التقارير)</h2>
        <p className="text-xs text-slate-500 mb-4 leading-relaxed">
          نصّا الرأس والتذييل يظهران في <b>كل</b> المطبوعات: التقارير الشهرية واليومية،
          كشف الرواتب، وقسيمة الراتب. يُسمح بكتابة أكثر من سطر (اسم المؤسسة، الفرع،
          الهاتف…).
        </p>
        <div className="grid sm:grid-cols-2 gap-4">
          <div>
            <label className="label">رأس المستند</label>
            <textarea
              className="input"
              rows={3}
              placeholder={`${cfg.company_name || "اسم الشركة"} — فرع رام الله\nهاتف: 02-xxx`}
              value={cfg.print_header ?? ""}
              onChange={(e) => set("print_header", e.target.value)}
            />
            <div className="text-[11px] text-slate-400 mt-1">
              الفارغ = اسم الشركة
            </div>
          </div>
          <div>
            <label className="label">تذييل المستند</label>
            <textarea
              className="input"
              rows={3}
              placeholder={`${cfg.company_name || "اسم الشركة"} — جميع الحقوق محفوظة`}
              value={cfg.print_footer ?? ""}
              onChange={(e) => set("print_footer", e.target.value)}
            />
            <div className="text-[11px] text-slate-400 mt-1">
              الفارغ = اسم الشركة · يُطبع أسفل كل ورقة مع تاريخ الطباعة
            </div>
          </div>
        </div>
        <label className="flex items-center gap-2 text-sm font-medium mt-4">
          <input
            type="checkbox"
            checked={(cfg.print_show_logo ?? "1") !== "0"}
            onChange={(e) => set("print_show_logo", e.target.checked ? "1" : "0")}
            className="accent-indigo-600 w-4 h-4"
          />
          إظهار شعار الشركة في رأس المطبوعات
        </label>
      </div>

      {/* التخزين والنسخ الاحتياطي */}
      <div className="card border-sky-200 bg-sky-50/40">
        <h2 className="font-bold mb-1">💾 التخزين والنسخ الاحتياطي</h2>
        <p className="text-xs text-slate-500 mb-4 leading-relaxed">
          كل الملفات في مجلد واحد بأسماء إنجليزية منظّمة — اختر مجلداً متزامناً مع{" "}
          <b>Google Drive / Dropbox</b> ليتم النسخ التلقائي. أو اضغط «نسخ احتياطي الآن»
          لتوليد اللقطة بنفسك.
        </p>

        <div className="space-y-2 text-sm bg-white rounded-lg border border-slate-200 p-3 font-mono">
          <Row label="المجلد النشط" value={storage?.storage_dir || "…"} />
          <Row label="قاعدة البيانات" value={storage?.db_path || "attendance.db"} />
          <Row label="ملف الإعدادات" value={storage?.settings_json || "attendance-settings.json"} />
          <Row label="ملف التهيئة" value={storage?.config_file || "storage-config.json"} />
        </div>

        <div className="flex flex-wrap gap-2 mt-4">
          <button
            className="btn-primary"
            onClick={doBackup}
            disabled={busyStorage}
          >
            🛟 نسخ احتياطي الآن
          </button>
          <button
            className="btn-ghost"
            onClick={chooseStorageDir}
            disabled={busyStorage}
          >
            📁 اختيار مجلد التخزين…
          </button>
          <button
            className="btn-ghost"
            onClick={doRestore}
            disabled={busyStorage}
          >
            ♻️ استعادة الإعدادات
          </button>
        </div>

        <div className="mt-3 text-[11px] text-slate-500 leading-relaxed">
          ملفات النسخ الاحتياطي:{" "}
          <code className="bg-white px-1 rounded">attendance-backup-YYYYMMDD-HHMMSS.db</code>{" "}
          (لقطة القاعدة بـ VACUUM INTO، ثابتة وآمنة) +{" "}
          <code className="bg-white px-1 rounded">attendance-settings.json</code>{" "}
          (كل الإعدادات بصيغة JSON). المجلد الأساسي:{" "}
          <span className="text-amber-700">{storage?.is_custom ? "مُخصَّص" : "افتراضي في مجلد التطبيق"}</span>.
        </div>
      </div>

      {/* إعدادات الرواتب */}
      <div className="card">
        <h2 className="font-bold mb-1">💰 إعدادات الرواتب</h2>
        <p className="text-xs text-slate-500 mb-4">
          كل هذه القيم افتراضية وقابلة للتعديل، وتُطبَّق فوراً على حساب الكشف.
        </p>
        <div className="grid sm:grid-cols-4 gap-4">
          <div>
            <label className="label">العملة</label>
            <select
              className="input"
              value={cfg.currency || "ILS"}
              onChange={(e) => set("currency", e.target.value)}
            >
              <option value="ILS">₪ شيكل (ILS)</option>
              <option value="USD">$ دولار (USD)</option>
              <option value="JOD">د.أ دينار أردني (JOD)</option>
            </select>
          </div>
          {PAYROLL_KEYS.map((f) => (
            <div key={f.k}>
              <label className="label">
                {f.k === "hourly_rate" ? `سعر الساعة (${curSym})` : f.label}
              </label>
              <input
                type="number"
                step={f.step}
                className="input"
                value={cfg[f.k] ?? ""}
                onChange={(e) => set(f.k, e.target.value)}
              />
            </div>
          ))}
          <div>
            <label className="label">طريقة احتساب بدل العطلة الأسبوعية</label>
            <select
              className="input"
              value={cfg.weekend_pay_mode ?? "weekly"}
              onChange={(e) => set("weekend_pay_mode", e.target.value)}
            >
              <option value="weekly">
                الأسبوعي — 62 عن الأسبوع ÷ 6 أيام (تُخصم نصيب الغياب)
              </option>
              <option value="per_day">لكل يوم عطلة — 62 عن كل يوم عطلة أسبوعية</option>
            </select>
          </div>
        </div>
        <p className="text-xs text-slate-500 mt-3 leading-relaxed">
          الأسبوع يبدأ <b>السبت</b> وينتهي <b>الجمعة</b>: الجمعة عطلة أسبوعية مدفوعة
          تُسجَّل تلقائياً في الحضور بلا بصمة. في الطريقة الأسبوعية يُقسَّم مبلغ الأسبوع
          (62) على ستة أيام دوام (السبت→الخميس) ويُدفع بنسبة الدوام الفعلي — كل يوم غياب
          يُخصم منه 62÷6 — ودوام الجمعة يُدفع عادياً ويبقى بدل الأسبوع حسب نسبة دوام أيامه
          الأُخرى. في الطريقة الثانية يُدفع 62 كامل عن كل يوم عطلة مستحق. تُطبَّق فور الحفظ
          على الكشف (لا تحتاج إعادة احتساب الحضور).
        </p>
      </div>

      {/* التوقيت */}
      <div className="card">
        <h2 className="font-bold mb-3">🕐 التوقيت والحضور</h2>
        <div className="grid sm:grid-cols-3 gap-4">
          {TIME_KEYS.map((f) => (
            <div key={f.k}>
              <label className="label">{f.label}</label>
              <input
                type={f.type}
                placeholder={f.ph}
                className="input"
                dir="ltr"
                value={cfg[f.k] ?? ""}
                onChange={(e) => set(f.k, e.target.value)}
              />
            </div>
          ))}
        </div>
        <label className="flex items-center gap-2 text-sm font-medium mt-4">
          <input
            type="checkbox"
            checked={(cfg.auto_fix_clock ?? "1") !== "0"}
            onChange={(e) => set("auto_fix_clock", e.target.checked ? "1" : "0")}
            className="accent-indigo-600 w-4 h-4"
          />
          تصحيح ساعة الجهاز تلقائياً عند اختلافها عن ساعة النظام
        </label>
        {/* زر الاستراحة: خصم 30 دقيقة دائماً أو إلغاؤه */}
        <label className="flex items-center gap-2 text-sm font-medium mt-3">
          <input
            type="checkbox"
            checked={(cfg.deduct_break ?? "1") !== "0"}
            onChange={(e) => set("deduct_break", e.target.checked ? "1" : "0")}
            className="accent-indigo-600 w-4 h-4"
          />
          خصم الاستراحة من الدوام (30 دقيقة تُخصم دائماً حتى لو دُوّم أقل ساعات)
        </label>
        <p className="text-xs text-slate-500 mt-1 leading-relaxed">
          عند تفعيله يُخصم {cfg.break_min || "30"} دقيقة من كل يوم دوام بلا بصمات
          استراحة — ولو كان اليوم أقصر من ساعات الدوام. عند إلغائه تُحتسب كل
          الدقائق المسجّلة بلا خصم. الاستراحة المسجّلة فعلياً (بصمات خروج/عودة)
          تُحسب دائماً كما هي مهما كان الخيار. <b>يتطلب إعادة احتساب الحضور</b>{" "}
          ليُطبَّق على الأيام السابقة.
        </p>
        <p className="text-xs text-slate-500 mt-1 leading-relaxed">
          مفيد إذا ضعفت بطارية الجهاز فتعود ساعته خاطئة (مثل السنة 2000): يُصحَّح
          عند كل مزامنة — كل {cfg.auto_sync_min || "5"} دقائق — فتُختم البصمات
          باليوم الصحيح وتظهر حضور اليوم كما هو.
        </p>

        {/* تنبيه التأخير */}
        <div className="mt-5 pt-4 border-t border-slate-200">
          <h3 className="font-bold text-sm mb-1">⏰ تنبيه تأخير الموظفين</h3>
          <p className="text-xs text-slate-500 mb-3 leading-relaxed">
            عند حلول الوقت المحدد يظهر إشعار على سطح المكتب بأسماء المتأخرين عن
            الدوام ودقائق تأخيرهم — مرة واحدة كل يوم. <b>معلومة فقط:</b> التأخير
            لا يُخصم من الرواتب ولا يؤثر على القسيمة (العمل قد يكون في أي ساعة).
          </p>
          <div className="flex flex-wrap items-end gap-4">
            <label className="flex items-center gap-2 text-sm font-medium">
              <input
                type="checkbox"
                checked={(cfg.late_notify_enabled ?? "1") !== "0"}
                onChange={(e) =>
                  set("late_notify_enabled", e.target.checked ? "1" : "0")
                }
                className="accent-indigo-600 w-4 h-4"
              />
              تفعيل تنبيه التأخير
            </label>
            <div>
              <label className="label">وقت التنبيه</label>
              <input
                type="text"
                placeholder="HH:MM"
                dir="ltr"
                className="input w-28"
                value={cfg.late_notify_time ?? "09:00"}
                onChange={(e) => set("late_notify_time", e.target.value)}
              />
            </div>
          </div>
        </div>
      </div>

      {/* الإجازات الرسمية (الأعياد) */}
      <div className="card">
        <h2 className="font-bold mb-1">🎆 الإجازات الرسمية (الأعياد)</h2>
        <p className="text-xs text-slate-500 mb-3 leading-relaxed">
          كل تاريخ هنا يُعامَل عطلة رسمية لجميع الموظفين: بلا ساعات، ويستحق كل
          موظف جزء بدل العطلة اليومي عنه (مثل يوم الإجازة المدفوعة). تُعاد
          حسابات اليوم تلقائياً عند الإضافة أو الحذف.
        </p>
        <div className="flex flex-wrap items-end gap-3 mb-3">
          <div>
            <label className="label">التاريخ</label>
            <input
              type="date"
              dir="ltr"
              className="input w-44"
              value={hDate}
              onChange={(e) => setHDate(e.target.value)}
            />
          </div>
          <div className="flex-1 min-w-48">
            <label className="label">الاسم (مثال: عيد الاستقلال)</label>
            <input
              className="input"
              placeholder="اختياري"
              value={hName}
              onChange={(e) => setHName(e.target.value)}
            />
          </div>
          <button className="btn-primary" onClick={addHoliday} disabled={hBusy}>
            ➕ إضافة
          </button>
        </div>
        {holidays.length === 0 ? (
          <div className="text-sm text-slate-400">
            لا توجد إجازات رسمية مسجلة.
          </div>
        ) : (
          <ul className="space-y-1 text-sm">
            {holidays.map((h) => (
              <li
                key={h.id}
                className="flex gap-3 items-center p-2 bg-violet-50 rounded border border-violet-200"
              >
                <span className="font-mono">{h.date}</span>
                <span className="font-medium">{h.name || "عطلة رسمية"}</span>
                <button
                  className="mr-auto text-rose-500 hover:text-rose-700"
                  title="حذف (يعود اليوم غياباً/حضراً حسب البصمات)"
                  onClick={() => removeHoliday(h)}
                >
                  🗑️
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* الإجازات السنوية — الإضافة التلقائية الشهرية للرصيد */}
      <div className="card">
        <h2 className="font-bold mb-1">🌴 الإجازات السنوية — إضافة تلقائية شهرية</h2>
        <p className="text-xs text-slate-500 mb-3 leading-relaxed">
          يُضاف لكل موظف نشِط جزء شهري من استحقاقه السنوي{" "}
          <b>(الاستحقاق ÷ 12)</b> إلى رصيده تلقائياً عند انتهاء كل شهر مكتمل —
          مرة واحدة يومياً أثناء عمل التطبيق. الاحتساب <b>يُستأنف من حيث توقف</b>{" "}
          آخر مرة فلا يُحتسب شهر مرتين ولا يفوّت شهر إن أُغلق التطبيق. أول احتساب
          لشهر التعيين يُحسب <b>بنسبة الأيام الباقية</b> من تاريخ التعيين فقط
          (عادل على الطرفين)، والشهر الجاري لا يُحتسب قبل اكتماله. الاستحقاق
          لكل موظف قابل للتعديل من بطاقة الموظف (الافتراضي 14 يوماً في السنة).
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={(cfg.leave_accrual_enabled ?? "1") !== "0"}
              onChange={(e) =>
                set("leave_accrual_enabled", e.target.checked ? "1" : "0")
              }
              className="accent-indigo-600 w-4 h-4"
            />
            تفعيل الإضافة التلقائية
          </label>
          <button
            className="btn-primary"
            onClick={runAccrual}
            disabled={accrualBusy}
            title="احتساب فوري لجميع الموظفين النشطين — آمن للتكرار"
          >
            {accrualBusy ? "جارٍ الاحتساب…" : "🧮 احتساب الأرصدة الآن"}
          </button>
        </div>
      </div>

      {/* التشغيل التلقائي */}
      <div className="card">
        <h2 className="font-bold mb-1">🚀 التشغيل التلقائي</h2>
        <p className="text-xs text-slate-500 mb-3">
          عند تفعيله يعمل التطبيق في الخلفية فور تسجيل الدخول (أيقونة في شريط الإشعارات بلا
          نافذة). زر الإغلاق يخفي النافذة فقط — يبقى يعمل، و«خروج» من قائمة الأيقونة ينهيه.
        </p>
        <label className="flex items-center gap-2 text-sm font-medium">
          <input
            type="checkbox"
            checked={autostart}
            disabled={autostartBusy}
            onChange={toggleAutostart}
            className="accent-indigo-600 w-4 h-4"
          />
          بدء تشغيل التطبيق تلقائياً مع تشغيل النظام
        </label>
      </div>

      {/* حقوق النشر */}
      <div className="pt-4 pb-2 text-center">
        <div className="text-xs text-slate-400 select-none">
          © 2026{" "}
          <a
            href="https://ordoo.shop"
            target="_blank"
            rel="noreferrer"
            className="text-sky-600 hover:underline"
          >
            ordoo.shop
          </a>{" "}
          — جميع الحقوق محفوظة
        </div>
      </div>
    </div>
  );
}

/** سطر عرض مسار في قسم التخزين. */
function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-2 items-baseline">
      <span className="font-sans text-xs text-slate-500 shrink-0 w-28">{label}</span>
      <span className="text-xs text-slate-800 break-all">{value}</span>
    </div>
  );
}
