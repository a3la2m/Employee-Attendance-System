import { useEffect, useState } from "react";
import {
  getSettings,
  saveSettingsBulk,
  setCompanyLogo,
  getStorageInfo,
  setStorageDir,
  backupNow,
  exportStorageFiles,
  restoreSettings,
  holidaysList,
  holidayAdd,
  holidayDelete,
  accrueLeaveNow,
  listShifts,
  upsertShift,
  setDefaultShift,
  deleteShift,
  assignEmployeesToShift,
  listEmployees,
  reassignShiftRecompute,
  clearDemoData,
} from "../lib/api";
import Modal from "../components/Modal";
import { useConfirm } from "../components/Confirm";
import type { Employee, Holiday, Settings, Shift, StorageInfo } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

// كل مفاتيح الرواتب القابلة للتعديل من الواجهة
const PAYROLL_KEYS = [
  {
    k: "daily_hours",
    label: "ساعات الدوام الأساسية الافتراضية (تُضبط فعلياً لكل وردية — هذه للوردية الرئيسية عند إنشائها)",
    step: "0.25",
  },
  { k: "break_min", label: "فترة الخصم (استراحة) الافتراضية بالدقائق (تُضبط لكل وردية)", step: "1" },
  { k: "hourly_rate", label: "سعر الساعة", step: "0.5" },
  { k: "meal_allow", label: "بدل الفطور اليومي", step: "0.5" },
  { k: "transport_allow", label: "بدل المواصلات اليومي", step: "0.5" },
  { k: "overtime_rate_mult", label: "مضاعف العمل الإضافي", step: "0.1" },
  { k: "grace_late_min", label: "مهلة التأخير (دقيقة) — يُطرح من التأخير ويُنبَّه على ما زاد", step: "1" },
  { k: "weekend_allow", label: "مبلغ بدل الاجازه", step: "1" },
];

const TIME_KEYS = [
  { k: "weekend_days", label: "عطلة الأسبوع (4=جمعة — الأسبوع من السبت إلى الجمعة)", type: "text", ph: "4" },
  { k: "auto_sync_min", label: "مزامنة تلقائية كل (دقيقة)", type: "number", ph: "5" },
];

/** المناطق الزمنية المتاحة: [المعرّف IANA، الوصف العربي]. */
const TZ_ZONES: [string, string][] = [
  ["Asia/Jerusalem", "فلسطين — القدس"],
  ["Asia/Gaza", "فلسطين — غزة"],
  ["Asia/Amman", "الأردن — عمّان"],
  ["Asia/Riyadh", "السعودية — الرياض"],
  ["Asia/Baghdad", "العراق — بغداد"],
  ["Asia/Kuwait", "الكويت"],
  ["Asia/Qatar", "قطر"],
  ["Asia/Dubai", "الإمارات — دبي"],
  ["Asia/Manama", "البحرين"],
  ["Asia/Muscat", "عُمان"],
  ["Asia/Aden", "اليمن — عدن"],
  ["Asia/Damascus", "سوريا — دمشق"],
  ["Asia/Beirut", "لبنان — بيروت"],
  ["Asia/Tehran", "إيران — طهران"],
  ["Asia/Istanbul", "تركيا — إسطنبول"],
  ["Africa/Cairo", "مصر — القاهرة"],
  ["Africa/Tripoli", "ليبيا — طرابلس"],
  ["Africa/Khartoum", "السودان — الخرطوم"],
  ["Africa/Tunis", "تونس"],
  ["Africa/Algiers", "الجزائر"],
  ["Africa/Casablanca", "المغرب — الدار البيضاء"],
  ["Africa/Nairobi", "كينيا — نيروبي"],
  ["Europe/London", "بريطانيا — لندن"],
  ["UTC", "غرينتش (UTC)"],
];

/** خيارات القائمة: تُدرج قيمة حالية غير مدرجة (نسيان إدخال يدوي قديم). */
const tzOptions = (v: string): [string, string][] =>
  TZ_ZONES.some(([id]) => id === v) ? TZ_ZONES : [[v, v], ...TZ_ZONES];

export default function SettingsPage({ toast }: { toast: Toast }) {
  // تأكيد «نعم/لا» داخلي — لا يستجيب للنقرة العابرة كـ window.confirm
  const confirm = useConfirm();
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
  // الورديات
  const [shifts, setShifts] = useState<Shift[]>([]);
  const [shiftBusy, setShiftBusy] = useState(false);
  const EMPTY_SHIFT: Shift = {
    id: 0,
    name: "",
    work_start: "09:00",
    work_end: "17:30",
    daily_hours: 8.5,
    break_min: 30,
    is_default: false,
    active: true,
    employee_count: 0,
  };
  const [shiftForm, setShiftForm] = useState<Shift>(EMPTY_SHIFT);
  // الإسناد الجماعي: الوردية المفتوحة للنافذة + قائمة الموظفين + المختارون
  const [assignShift, setAssignShift] = useState<Shift | null>(null);
  const [allEmployees, setAllEmployees] = useState<Employee[]>([]);
  const [picked, setPicked] = useState<Set<number>>(new Set());

  const load = async () => {
    try {
      setCfg(await getSettings());
    } catch (e) {
      toast(String(e), "err");
    }
    import("@tauri-apps/plugin-autostart")
      .then((m) => m.isEnabled())
      .then(setAutostart)
      .catch(() => {});
    getStorageInfo().then(setStorage).catch(() => {});
    holidaysList().then(setHolidays).catch(() => {});
    listShifts().then(setShifts).catch(() => {});
  };

  useEffect(() => {
    load();
  }, []);

  const saveShift = async () => {
    setShiftBusy(true);
    try {
      await upsertShift(shiftForm);
      setShifts(await listShifts());
      setShiftForm(EMPTY_SHIFT);
      toast("حُفظت الوردية ✅ — اضغط «إعادة احتساب الحضور» لتطبيقها على الأيام السابقة", "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setShiftBusy(false);
    }
  };

  const makeDefault = async (id: number) => {
    setShiftBusy(true);
    try {
      await setDefaultShift(id);
      setShifts(await listShifts());
      toast("صارت الوردية الرئيسية ✅");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setShiftBusy(false);
    }
  };

  const removeShift = async (id: number) => {
    if (!(await confirm("حذف هذه الوردية؟ الموظفون المسنَدون إليها سيعودون للوردية الرئيسية.", { title: "حذف الوردية", yes: "نعم، احذف" })))
      return;
    setShiftBusy(true);
    try {
      await deleteShift(id);
      setShifts(await listShifts());
      toast("حُذفت الوردية ✅");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setShiftBusy(false);
    }
  };

  // فتح نافذة الإسناد الجماعي لوردية: نُحمّل الموظفين ونحدّد المسنَدين إليها مسبقاً
  const openAssign = async (s: Shift) => {
    setAssignShift(s);
    setPicked(new Set());
    try {
      const emps = await listEmployees();
      setAllEmployees(emps);
      setPicked(new Set(emps.filter((e) => e.shift_id === s.id).map((e) => e.id)));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const togglePicked = (id: number) => {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const saveAssign = async () => {
    if (!assignShift) return;
    setShiftBusy(true);
    try {
      // المسنَدون الآن = المختارون. ومن كان مسنَداً لهذه الوردية ولم يبقَ مختاراً
      // نُخرجه (shift_id=NULL → يرث الوردية الرئيسية).
      const keep = allEmployees.filter((e) => picked.has(e.id)).map((e) => e.id);
      const remove = allEmployees
        .filter((e) => e.shift_id === assignShift.id && !picked.has(e.id))
        .map((e) => e.id);
      let changed = 0;
      if (keep.length) {
        changed += await assignEmployeesToShift(keep, assignShift.id);
        await reassignShiftRecompute("1900-01-01", "2100-01-01");
      }
      if (remove.length) {
        changed += await assignEmployeesToShift(remove, null);
        await reassignShiftRecompute("1900-01-01", "2100-01-01");
      }
      setShifts(await listShifts());
      toast(`تم الإسناد ✅ (${changed} موظف) — وأُعيد احتساب الحضور`, "ok");
      setAssignShift(null);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setShiftBusy(false);
    }
  };

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
  // مبلغ بدل الإجازة (يُقرأ حياً من مربع النص في الإعدادات — الافتراض 62)
  const wAllow = Number(cfg.weekend_allow) > 0 ? Number(cfg.weekend_allow) : 62;

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

  // نقل ملفات قاعدة البيانات والإعدادات إلى مجلد يختاره المستخدم (مثال: Google Drive)
  const moveStorageFiles = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const dir = await open({ directory: true, multiple: false, title: "اختر مجلد نقل ملفات النسخ الاحتياطي" });
      if (typeof dir === "string") {
        setBusyStorage(true);
        const files = await exportStorageFiles(dir);
        if (files.length === 0) {
          toast("لم يُنسخ أي ملف", "err");
          return;
        }
        const msg = `تم نقل ${files.length} ملف إلى:\n${dir}\n\nيُفضل إعادة تشغيل التطبيق للتأكد من عمله بعد النقل. هل تريد إعادة التشغيل الآن؟`;
        if (await confirm(msg, { title: "إعادة التشغيل", yes: "نعم، أعد التشغيل" })) {
          try {
          const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
          await getCurrentWebviewWindow().close();
        } catch {
          try {
            const w = (window as any).__TAURI__?.window?.getCurrentWindow?.();
            await w?.close();
          } catch {
            window.close();
          }
        }
        } else {
          toast("تم النقل بنجاح — يُنصح بإعادة تشغيل البرنامج للتأكد", "ok");
        }
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyStorage(false);
    }
  };

  /** حذف الموظفين التجريبيين (9001..9005) وبصماتهم — يُنشأ تلقائياً لأول تشغيل. */
  const doClearDemo = async () => {
    const ok = await confirm(
      "حذف البيانات التجريبية (5 موظفين وبصماتهم)؟\nلا يُمسّ أي موظف حقيقي.",
      { title: "حذف البيانات التجريبية", yes: "نعم، احذفها" },
    );
    if (!ok) return;
    setBusyStorage(true);
    try {
      const r = await clearDemoData();
      toast(
        r.deleted > 0
          ? `حُذفت ${r.deleted} صفاً من البيانات التجريبية ✅`
          : "لا توجد بيانات تجريبية للحذف",
      );
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusyStorage(false);
    }
  };

  const doRestore = async () => {
    const ok = await confirm(
      "استعادة الإعدادات من attendance-settings.json فوق الإعدادات الحالية؟",
      { title: "استعادة الإعدادات", yes: "نعم، استعد" },
    );
    if (!ok) return;
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
    // تأكيد داخلي: لا حذف إلا بعد ضغط «نعم» فعلياً (window.confirm كان قد
    // يستقبل نقرة ثانية عابرة فيُنفَّذ الحذف قبل قراءة السؤال)
    const ok = await confirm(
      `حذف الإجازة الرسمية ${h.date}${h.name ? " — " + h.name : ""}؟\nيُعاد احتساب اليوم لكل الموظفين.`,
      { title: "حذف إجازة رسمية", yes: "نعم، احذف" },
    );
    if (!ok) return;
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
          <button
            className="btn-ghost"
            onClick={moveStorageFiles}
            disabled={busyStorage}
            title="نقل ملفات قاعدة البيانات والإعدادات إلى مجلد آخر (مثل جوجل درايف) مع إمكانية إعادة التشغيل"
          >
            📦 نقل الملفات إلى مجلد آخر…
          </button>
          <button
            className="btn-ghost"
            onClick={doClearDemo}
            disabled={busyStorage}
            title="حذف الموظفين التجريبيين (9001..9005) وبصماتهم — تُنشأ لأول تشغيل فقط"
          >
            🧹 حذف البيانات التجريبية
          </button>
        </div>

        <div className="mt-3 text-[11px] text-slate-500 leading-relaxed">
          ملفات النسخ الاحتياطي:{" "}
          <code className="bg-white px-1 rounded">attendance-backup-YYYYMMDD-HHMMSS.db</code>{" "}
          (لقطة القاعدة بـ VACUUM INTO، ثابتة وآمنة) +{" "}
          <code className="bg-white px-1 rounded">attendance-settings.json</code>{" "}
          (كل الإعدادات بصيغة JSON). «نقل الملفات إلى مجلد آخر» يُنسخ{" "}
          <code className="bg-white px-1 rounded">attendance.db</code> و
          <code className="bg-white px-1 rounded">attendance-settings.json</code> و
          <code className="bg-white px-1 rounded">storage-config.json</code> إلى المجلد
          المختار ويعرض خيار إعادة تشغيل التطبيق بعد النقل للتأكد من سلامة العملية. المجلد
          الأساسي:{" "}
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
                الأسبوعي — {wAllow} عن الأسبوع ÷ 6 أيام (تُخصم نصيب الغياب)
              </option>
              <option value="per_day">
                لكل يوم عطلة — {wAllow} عن كل يوم عطلة أسبوعية
              </option>
            </select>
          </div>
        </div>
        <p className="text-xs text-slate-500 mt-3 leading-relaxed">
          الأسبوع يبدأ <b>السبت</b> وينتهي <b>الجمعة</b>: الجمعة عطلة أسبوعية مدفوعة
          تُسجَّل تلقائياً في الحضور بلا بصمة. في الطريقة الأسبوعية يُقسَّم مبلغ
          «مبلغ بدل الاجازه» ({wAllow}) على ستة أيام دوام (السبت→الخميس) ويُدفع بنسبة
          الدوام الفعلي — كل يوم غياب يُخصم منه {wAllow}÷6 — ودوام الجمعة يُدفع عادياً
          ويبقى بدل الأسبوع حسب نسبة دوام أيامه الأُخرى. في الطريقة الثانية يُدفع{" "}
          {wAllow} كامل عن كل يوم عطلة مستحق. تُطبَّق فور الحفظ
          على الكشف (لا تحتاج إعادة احتساب الحضور).
        </p>
      </div>

      {/* الورديات */}
      <div className="card">
        <h2 className="font-bold mb-1">🗂️ الورديات (ساعات الدوام)</h2>
        <p className="text-xs text-slate-500 mb-3 leading-relaxed">
          كل وردية تحدّد <b>بداية ونهاية الدوام</b> و<b>الساعات الأساسية</b> و<b>الاستراحة</b>،
          ويُسنَد كل موظف إلى ورديته من صفحة «الموظفون». الوردية <b>الرئيسية</b> (⭐) يرثها كل
          موظف غير مسنَد لوردية. الوردية غير المفعّلة تصبح <b>مهجورة</b> (تبقى في السجل بلا
          إسناد). الوردية الليلية (نهايتها أصغر من بدايتها مثل 20:00→04:00) تُحتسب تلقائياً
          عبر منتصف الليل. <b>بعد تغيير وردية اضغط «إعادة احتساب الحضور»</b> في صفحة
          الحضور لتُطبَّق على الأيام السابقة.
        </p>

        {/* جدول الورديات */}
        <div className="overflow-x-auto">
          <table className="table text-sm">
            <thead>
              <tr>
                <th>الاسم</th>
                <th>البداية</th>
                <th>النهاية</th>
                <th>ساعات/يوم</th>
                <th>استراحة (د)</th>
                <th>الموظفون</th>
                <th>الحالة</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {shifts.map((s) => (
                <tr key={s.id}>
                  <td className="font-medium">
                    {s.name} {s.is_default && <span title="الوردية الرئيسية">⭐</span>}
                  </td>
                  <td dir="ltr">{s.work_start}</td>
                  <td dir="ltr">{s.work_end}</td>
                  <td>{s.daily_hours}</td>
                  <td>{s.break_min}</td>
                  <td>{s.employee_count}</td>
                  <td>
                    {s.active ? (
                      <span className="badge bg-emerald-100 text-emerald-700">مفعّلة</span>
                    ) : (
                      <span className="badge bg-slate-200 text-slate-600">مهجورة</span>
                    )}
                  </td>
                  <td>
                    <div className="flex gap-2 justify-end">
                      <button
                        className="btn btn-ghost text-xs"
                        onClick={() => setShiftForm({ ...s })}
                        disabled={shiftBusy}
                      >
                        تعديل
                      </button>
                      <button
                        className="btn btn-ghost text-xs"
                        onClick={() => openAssign(s)}
                        disabled={shiftBusy}
                        title="اختيار الموظفين المسنَدين لهذه الوردية"
                      >
                        👥 إسناد موظفين
                      </button>
                      {!s.is_default && (
                        <button
                          className="btn btn-ghost text-xs"
                          onClick={() => makeDefault(s.id)}
                          disabled={shiftBusy}
                        >
                          اجعلها الرئيسية
                        </button>
                      )}
                      {shifts.length > 1 && (
                        <button
                          className="btn btn-ghost text-xs text-red-600"
                          onClick={() => removeShift(s.id)}
                          disabled={shiftBusy}
                        >
                          حذف
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* نموذج وردية */}
        <div className="mt-4 pt-4 border-t border-slate-200">
          <h3 className="font-bold text-sm mb-2">
            {shiftForm.id > 0 ? `تعديل «${shiftForm.name}»` : "إضافة وردية جديدة"}
          </h3>
          <div className="grid sm:grid-cols-3 lg:grid-cols-6 gap-3">
            <div className="sm:col-span-2">
              <label className="label">اسم الوردية</label>
              <input
                className="input"
                value={shiftForm.name}
                onChange={(e) => setShiftForm({ ...shiftForm, name: e.target.value })}
                placeholder="مثال: وردية صباحية"
              />
            </div>
            <div>
              <label className="label">بداية الدوام</label>
              <input
                type="text"
                dir="ltr"
                className="input"
                placeholder="HH:MM"
                value={shiftForm.work_start}
                onChange={(e) => setShiftForm({ ...shiftForm, work_start: e.target.value })}
              />
            </div>
            <div>
              <label className="label">نهاية الدوام</label>
              <input
                type="text"
                dir="ltr"
                className="input"
                placeholder="HH:MM"
                value={shiftForm.work_end}
                onChange={(e) => setShiftForm({ ...shiftForm, work_end: e.target.value })}
              />
            </div>
            <div>
              <label className="label">ساعات الدوام الأساسية</label>
              <input
                type="number"
                step="0.25"
                className="input"
                value={shiftForm.daily_hours}
                onChange={(e) =>
                  setShiftForm({ ...shiftForm, daily_hours: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="label">الاستراحة (دقيقة)</label>
              <input
                type="number"
                step="1"
                className="input"
                value={shiftForm.break_min}
                onChange={(e) =>
                  setShiftForm({ ...shiftForm, break_min: Number(e.target.value) })
                }
              />
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-4 mt-3">
            <label className="flex items-center gap-2 text-sm font-medium">
              <input
                type="checkbox"
                checked={shiftForm.active}
                onChange={(e) => setShiftForm({ ...shiftForm, active: e.target.checked })}
                className="accent-indigo-600 w-4 h-4"
              />
              مفعّلة (إلغاؤها يجعلها مهجورة)
            </label>
            <label className="flex items-center gap-2 text-sm font-medium">
              <input
                type="checkbox"
                checked={shiftForm.is_default}
                onChange={(e) => setShiftForm({ ...shiftForm, is_default: e.target.checked })}
                className="accent-indigo-600 w-4 h-4"
              />
              اجعلها الوردية الرئيسية
            </label>
            <div className="flex gap-2 mr-auto">
              <button
                className="btn btn-primary"
                onClick={saveShift}
                disabled={shiftBusy || !shiftForm.name.trim()}
              >
                {shiftForm.id > 0 ? "حفظ التعديل" : "إضافة الوردية"}
              </button>
              {shiftForm.id > 0 && (
                <button
                  className="btn btn-ghost"
                  onClick={() => setShiftForm(EMPTY_SHIFT)}
                  disabled={shiftBusy}
                >
                  إلغاء
                </button>
              )}
            </div>
          </div>
        </div>

        {/* نافذة الإسناد الجماعي للموظفين */}
        {assignShift && (
          <Modal
            onClose={() => setAssignShift(null)}
            title={`👥 إسناد موظفين إلى «${assignShift.name}»`}
            panelClass="max-w-lg"
          >
            <p className="text-xs text-slate-500 mb-3">
              اختر الموظفين الذين يعملون على هذه الوردية. من تُزيله من القائمة يعود
              للوردية الرئيسية. يُعاد احتساب الحضور تلقائياً بعد الحفظ.
            </p>
            <div className="flex items-center justify-between mb-2">
              <button
                className="btn btn-ghost text-xs"
                onClick={() =>
                  setPicked(
                    picked.size === allEmployees.length
                      ? new Set()
                      : new Set(allEmployees.map((e) => e.id)),
                  )
                }
              >
                {picked.size === allEmployees.length ? "إلغاء تحديد الكل" : "تحديد الكل"}
              </button>
              <span className="text-xs text-slate-400">
                {picked.size} من {allEmployees.length}
              </span>
            </div>
            <div className="max-h-72 overflow-y-auto border border-slate-200 rounded-lg divide-y divide-slate-100">
              {allEmployees.length === 0 && (
                <p className="p-3 text-sm text-slate-400 text-center">لا يوجد موظفون.</p>
              )}
              {allEmployees.map((e) => (
                <label
                  key={e.id}
                  className="flex items-center gap-3 px-3 py-2 hover:bg-slate-50 cursor-pointer"
                >
                  <input
                    type="checkbox"
                    className="accent-indigo-600 w-4 h-4"
                    checked={picked.has(e.id)}
                    onChange={() => togglePicked(e.id)}
                  />
                  <span className="text-sm font-medium">{e.name}</span>
                  {!e.active && (
                    <span className="badge bg-slate-200 text-slate-500 text-[10px]">موقوف</span>
                  )}
                  {e.shift_id != null && e.shift_id !== assignShift.id && (
                    <span className="text-[10px] text-amber-600 mr-auto">
                      حالياً: {shifts.find((s) => s.id === e.shift_id)?.name ?? "وردية أخرى"}
                    </span>
                  )}
                  {e.shift_id === assignShift.id && (
                    <span className="text-[10px] text-emerald-600 mr-auto">مسنَد هنا</span>
                  )}
                </label>
              ))}
            </div>
            <div className="flex justify-end gap-2 mt-4">
              <button
                className="btn btn-ghost"
                onClick={() => setAssignShift(null)}
                disabled={shiftBusy}
              >
                إلغاء
              </button>
              <button className="btn btn-primary" onClick={saveAssign} disabled={shiftBusy}>
                {shiftBusy ? "جارٍ الحفظ…" : "حفظ الإسناد"}
              </button>
            </div>
          </Modal>
        )}
      </div>

      {/* التوقيت */}
      <div className="card">
        <h2 className="font-bold mb-3">🕐 التوقيت والحضور</h2>
        <div className="grid sm:grid-cols-3 gap-4">
          <div>
            <label className="label">
              المنطقة الزمنية (توقيت الجهاز وحساب الدوام)
            </label>
            <select
              className="input"
              dir="ltr"
              value={cfg.timezone || "Asia/Jerusalem"}
              onChange={(e) => set("timezone", e.target.value)}
            >
              {tzOptions(cfg.timezone || "Asia/Jerusalem").map(([id, ar]) => (
                <option key={id} value={id}>
                  {ar} — {id}
                </option>
              ))}
            </select>
          </div>
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
        <p className="text-xs text-slate-500 mt-2 leading-relaxed">
          🕐 كل ما يُحتسب من <b>الوقت المفروض</b> — بداية الدوام ونهايته
          والتأخير والانصراف المبكر والإضافي — يُحسب بتوقيت هذه المنطقة، ويُضبط
          جهاز البصمة على توقيتها نفسه عند كل مزامنة تلقائية (كل{" "}
          {cfg.auto_sync_min || "5"} دقائق). وعند تغيّر التوقيت الصيفي/الشتوي
          يُكتشف الفرق ويُصحَّح تلقائياً مع تنبيه على الشاشة.
        </p>
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
          <h3 className="font-bold text-sm mb-1">⏰ تنبيه تأخير وغياب الموظفين (حسب الشفت)</h3>
          <p className="text-xs text-slate-500 mb-3 leading-relaxed">
            يُرسل التنبيه تلقائياً على سطح المكتب وبوت تلجرام <b>بعد بداية شفت كل موظف بـ 15 دقيقة</b>{" "}
            — مرة واحدة لكل موظف يومياً. <b>معلومة فقط:</b> التأخير لا يُخصم من
            الرواتب ولا يؤثر على القسيمة.
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
              تفعيل تنبيهات التأخير والدوام (بعد بداية الشفت بـ 15 دقيقة)
            </label>
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
