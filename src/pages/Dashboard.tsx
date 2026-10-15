import { useEffect, useState } from "react";
import { dashboard, syncDevice, checkTimeSync } from "../lib/api";
import type { Dashboard as Dash, TimeCheck } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

export default function DashboardPage({ toast }: { toast: Toast }) {
  const [d, setD] = useState<Dash | null>(null);
  const [tc, setTc] = useState<TimeCheck | null>(null);
  const [busy, setBusy] = useState(false);

  const load = async () => {
    try {
      setD(await dashboard());
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    load();
    checkTimeSync()
      .then(setTc)
      .catch(() => setTc(null));
  }, []);

  const sync = async () => {
    setBusy(true);
    try {
      const r = await syncDevice();
      toast(
        `مزامنة ناجحة: ${r.new_punches} بصمة + ${r.new_users_added} موظف — أُعيد حساب ${r.days_recomputed} يوم`,
      );
      await load();
    } catch (e) {
      toast(`تعذّرت المزامنة: ${e}`, "err");
    } finally {
      setBusy(false);
    }
  };

  const cards = [
    { label: "الموظفون النشطون", value: d?.employees ?? "—", icon: "👥", tone: "bg-indigo-50 text-indigo-600" },
    { label: "الحاضرون اليوم", value: d?.present ?? "—", icon: "✅", tone: "bg-emerald-50 text-emerald-600" },
    { label: "الغائبون اليوم", value: d?.absent ?? "—", icon: "❌", tone: "bg-rose-50 text-rose-600" },
    { label: "إجمالي البصمات", value: d?.total_punches ?? "—", icon: "🗓️", tone: "bg-slate-100 text-slate-600" },
  ];

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-extrabold">لوحة التحكم</h1>
          <p className="text-sm text-slate-500 mt-1">
            {d ? `اليوم: ${d.date}` : "جارٍ التحميل…"} · آخر مزامنة: {d?.last_sync || "—" }
          </p>
        </div>
        <button className="btn-primary" onClick={sync} disabled={busy}>
          {busy ? "جارٍ السحب…" : "🔄 سحب من الجهاز الآن"}
        </button>
      </div>

      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        {cards.map((c) => (
          <div key={c.label} className="card flex items-center gap-4">
            <div className={`w-12 h-12 rounded-xl grid place-content-center text-xl ${c.tone}`}>
              {c.icon}
            </div>
            <div>
              <div className="text-2xl font-extrabold">{c.value}</div>
              <div className="text-xs text-slate-500">{c.label}</div>
            </div>
          </div>
        ))}
      </div>

      {/* حالة ساعة الجهاز */}
      <div className="card">
        <h2 className="font-bold mb-3">🕐 حالة ساعة الجهاز</h2>
        {!tc ? (
          <p className="text-sm text-slate-500">
            تعذّر الوصول للجهاز (أوقفه أو تحقّق من الشبكة).
          </p>
        ) : (
          <div className="grid sm:grid-cols-3 gap-4 text-sm">
            <div>
              <div className="text-slate-500 text-xs">وقت الجهاز</div>
              <div className="font-mono font-bold">{tc.device_time}</div>
            </div>
            <div>
              <div className="text-slate-500 text-xs">الوقت المحلي ({tc.timezone})</div>
              <div className="font-mono font-bold">{tc.server_time}</div>
            </div>
            <div>
              <div className="text-slate-500 text-xs">الفرق</div>
              <div
                className={`font-bold ${
                  tc.in_sync ? "text-emerald-600" : "text-rose-600"
                }`}
              >
                {tc.in_sync ? "✅ متزامن" : `⚠️ ${tc.diff_minutes} دقيقة`}
              </div>
            </div>
          </div>
        )}
      </div>

      <div className="card text-sm text-slate-600 leading-relaxed">
        <b>ملاحظة:</b> النظام يسحب سجلات البصمة تلقائياً عند الإقلاع ثم دوريّاً حسب فترة
        المزامنة المضبوطة في الإعدادات، ويعيد احتساب أيام الحضور تلقائياً.
      </div>
    </div>
  );
}
