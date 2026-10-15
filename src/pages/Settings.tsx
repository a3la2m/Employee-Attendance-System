import { useEffect, useState } from "react";
import {
  getSettings,
  saveSettingsBulk,
  setCompanyLogo,
  listEmployees,
} from "../lib/api";
import type { Employee, Settings } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

// كل مفاتيح الرواتب القابلة للتعديل من الواجهة
const PAYROLL_KEYS = [
  { k: "daily_hours", label: "ساعات الدوام الأساسية", step: "0.25" },
  { k: "break_min", label: "فترة الخصم (استراحة) بالدقائق", step: "1" },
  { k: "hourly_rate", label: "سعر الساعة (شيكل)", step: "0.5" },
  { k: "meal_allow", label: "بدل الفطور اليومي", step: "0.5" },
  { k: "transport_allow", label: "بدل المواصلات اليومي", step: "0.5" },
  { k: "overtime_after_hours", label: "العمل الإضافي بعد (ساعة)", step: "0.25" },
  { k: "overtime_rate_mult", label: "مضاعف العمل الإضافي", step: "0.1" },
  { k: "grace_late_min", label: "مهلة التأخير (دقيقة)", step: "1" },
];

const TIME_KEYS = [
  { k: "work_start", label: "بداية الدوام (HH:MM)", type: "time" },
  { k: "work_end", label: "نهاية الدوام (HH:MM)", type: "time" },
  { k: "timezone", label: "المنطقة الزمنية", type: "text" },
  { k: "weekend_days", label: "عطلة الأسبوع (0=اثنين … 4=جمعة 5=سبت)", type: "text" },
  { k: "auto_sync_min", label: "مزامنة تلقائية كل (دقيقة)", type: "number" },
];

export default function SettingsPage({ toast }: { toast: Toast }) {
  const [cfg, setCfg] = useState<Settings>({});
  const [employees, setEmployees] = useState<Employee[]>([]);
  const [newUser, setNewUser] = useState({ pin: "", name: "", job_title: "" });

  const load = async () => {
    try {
      setCfg(await getSettings());
    } catch (e) {
      toast(String(e), "err");
    }
    listEmployees().then(setEmployees).catch(() => {});
  };

  useEffect(() => {
    load();
  }, []);

  const set = (k: string, v: string) => setCfg((c) => ({ ...c, [k]: v }));

  const saveAll = async () => {
    try {
      await saveSettingsBulk(cfg);
      toast("تم حفظ جميع الإعدادات ✅");
    } catch (e) {
      toast(String(e), "err");
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

  const saveNewUser = async () => {
    if (!newUser.name.trim() || !newUser.pin.trim())
      return toast("الاسم ورقم البصمة مطلوبان", "err");
    const { upsertEmployee } = await import("../lib/api");
    try {
      await upsertEmployee({
        id: 0,
        pin: newUser.pin,
        name: newUser.name,
        job_title: newUser.job_title,
        phone: "",
        daily_hours: Number(cfg.daily_hours || 8.5),
        break_min: Number(cfg.break_min || 30),
        hourly_rate: Number(cfg.hourly_rate || 10),
        meal_allow: Number(cfg.meal_allow || 7),
        transport_allow: Number(cfg.transport_allow || 5),
        active: true,
      });
      toast("أُضيف الموظف ورُبط برقم البصمة");
      setNewUser({ pin: "", name: "", job_title: "" });
      listEmployees().then(setEmployees).catch(() => {});
    } catch (e) {
      toast(String(e), "err");
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

      {/* إعدادات الرواتب */}
      <div className="card">
        <h2 className="font-bold mb-1">💰 إعدادات الرواتب</h2>
        <p className="text-xs text-slate-500 mb-4">
          كل هذه القيم افتراضية وقابلة للتعديل، وتُطبَّق فوراً على حساب الكشف.
        </p>
        <div className="grid sm:grid-cols-4 gap-4">
          {PAYROLL_KEYS.map((f) => (
            <div key={f.k}>
              <label className="label">{f.label}</label>
              <input
                type="number"
                step={f.step}
                className="input"
                value={cfg[f.k] ?? ""}
                onChange={(e) => set(f.k, e.target.value)}
              />
            </div>
          ))}
        </div>
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
                className="input"
                value={cfg[f.k] ?? ""}
                onChange={(e) => set(f.k, e.target.value)}
              />
            </div>
          ))}
        </div>
      </div>

      {/* المستخدم الجديد في ويندوز */}
      <div className="card border-amber-200 bg-amber-50/50">
        <h2 className="font-bold mb-1">🪟 ربط مستخدم ويندوز جديد</h2>
        <p className="text-xs text-slate-500 mb-3">
          عند رصد مستخدم جديد في ويندوز يظهر إشعار — أدخل بياناته الأساسية هنا لربطه برقم
          البصمة.
        </p>
        <div className="grid sm:grid-cols-3 gap-3">
          <input
            className="input"
            placeholder="رقم البصمة (PIN)"
            value={newUser.pin}
            onChange={(e) => setNewUser({ ...newUser, pin: e.target.value })}
          />
          <input
            className="input"
            placeholder="اسم الموظف"
            value={newUser.name}
            onChange={(e) => setNewUser({ ...newUser, name: e.target.value })}
          />
          <input
            className="input"
            placeholder="الوظيفة"
            value={newUser.job_title}
            onChange={(e) => setNewUser({ ...newUser, job_title: e.target.value })}
          />
        </div>
        <div className="mt-3">
          <button className="btn-primary" onClick={saveNewUser}>
            + إضافة وربط بالبصمة
          </button>
        </div>
        {employees.length > 0 && (
          <div className="mt-3 text-xs text-slate-600">
            المسجّلون الآن ({employees.length}):{" "}
            {employees.map((e) => `${e.name}#${e.pin || "?"}`).join(" · ")}
          </div>
        )}
      </div>
    </div>
  );
}
