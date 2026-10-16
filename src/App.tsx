import { useEffect, useState } from "react";
import type { ComponentType } from "react";
import Dashboard from "./pages/Dashboard";
import Employees from "./pages/Employees";
import Attendance from "./pages/Attendance";
import Payroll from "./pages/Payroll";
import Reports from "./pages/Reports";
import Device from "./pages/Device";
import Settings from "./pages/Settings";
import Toast from "./components/Toast";
import { listen } from "@tauri-apps/api/event";

type TabKey =
  | "dashboard"
  | "employees"
  | "attendance"
  | "payroll"
  | "reports"
  | "device"
  | "settings";

const NAV: { key: TabKey; label: string; icon: string }[] = [
  { key: "dashboard", label: "لوحة التحكم", icon: "📊" },
  { key: "employees", label: "الموظفون", icon: "👥" },
  { key: "attendance", label: "الحضور", icon: "🗓️" },
  { key: "payroll", label: "الرواتب", icon: "💰" },
  { key: "reports", label: "التقارير والطباعة", icon: "🖨️" },
  { key: "device", label: "جهاز البصمة", icon: "🔐" },
  { key: "settings", label: "الإعدادات", icon: "⚙️" },
];

const PAGES: Record<TabKey, ComponentType<{ toast: (m: string, t?: "ok" | "err") => void }>> = {
  dashboard: Dashboard,
  employees: Employees,
  attendance: Attendance,
  payroll: Payroll,
  reports: Reports,
  device: Device,
  settings: Settings,
};

export default function App() {
  const [tab, setTab] = useState<TabKey>("dashboard");
  const [toastMsg, setToastMsg] = useState<{ m: string; t: "ok" | "err" } | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  const toast = (m: string, t: "ok" | "err" = "ok") => setToastMsg({ m, t });

  useEffect(() => {
    // مزامنة تلقائية تُبلّغ من الخلفية
    const un1 = listen<{
      new_punches?: number;
      ok: boolean;
      error?: string;
      clock_diff_min?: number;
      device_time?: string;
    }>("sync", (e) => {
      const skew = e.payload.clock_diff_min;
      if (typeof skew === "number" && Math.abs(skew) > 5) {
        toast(
          Math.abs(skew) > 120
            ? `⚠️ ساعة الجهاز خاطئة (تُظهر ${e.payload.device_time ?? "؟؟"}) — بصمات اليوم ستُسجل بتاريخ خاطئ. افتح «الجهاز» واضغط «تصحيح الوقت».`
            : `⚠️ ساعة الجهاز تختلف بحوالي ${Math.round(Math.abs(skew))} دقيقة — صحّحها من صفحة «الجهاز».`,
          "err",
        );
      } else if (e.payload.ok && (e.payload.new_punches ?? 0) > 0) {
        toast(`تم سحب ${e.payload.new_punches} بصمة جديدة من الجهاز`, "ok");
      } else if (!e.payload.ok && e.payload.error) {
        // صامت عمداً: فقدان الاتصال شائع عند الإقلاع قبل الشبكة
        console.warn("sync error:", e.payload.error);
      }
    });
    // مستخدم ويندوز جديد
    const un2 = listen<{ profile: string }>("new-windows-user", (e) => {
      setPending(e.payload.profile);
      toast(`مستخدم ويندوز جديد: ${e.payload.profile} — أدخل بيانات الموظف`, "err");
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
    };
  }, []);

  const Page = PAGES[tab];

  return (
    <div className="flex h-full">
      {/* الشريط الجانبي */}
      <aside className="w-60 shrink-0 bg-slate-900 text-slate-100 flex flex-col">
        <div className="px-5 py-5 border-b border-slate-800">
          <div className="text-lg font-extrabold">نظام الحضور</div>
          <div className="text-xs text-slate-400 mt-0.5">والرواتب — بصمة K50</div>
        </div>
        <nav className="flex-1 p-3 space-y-1">
          {NAV.map((n) => {
            const active = tab === n.key;
            return (
              <button
                key={n.key}
                onClick={() => setTab(n.key)}
                className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition ${
                  active
                    ? "bg-indigo-600 text-white font-bold"
                    : "text-slate-300 hover:bg-slate-800"
                }`}
              >
                <span className="text-base">{n.icon}</span>
                <span>{n.label}</span>
              </button>
            );
          })}
        </nav>
        <div className="p-4 text-[11px] text-slate-500 border-t border-slate-800">
          يظل يعمل في الخلفية كأيقونة في شريط إشعارات النظام
        </div>
      </aside>

      {/* المحتوى */}
      <main className="flex-1 overflow-auto">
        {pending && (
          <div className="m-4 p-3 rounded-lg bg-amber-50 border border-amber-300 text-amber-800 text-sm flex items-center justify-between">
            <span>
              رُصد مستخدم ويندوز جديد: <b>{pending}</b> — افتح قسم «الموظفون» لإدخال بياناته
              الأساسية والربط بالبصمة.
            </span>
            <button className="btn-ghost !py-1" onClick={() => setPending(null)}>
              حسناً
            </button>
          </div>
        )}
        <Page toast={toast} />
      </main>

      {toastMsg && (
        <Toast
          message={toastMsg.m}
          type={toastMsg.t}
          onClose={() => setToastMsg(null)}
        />
      )}
    </div>
  );
}
