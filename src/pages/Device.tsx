import { useEffect, useState } from "react";
import {
  deviceInfo,
  syncDevice,
  checkTimeSync,
  fixDeviceTime,
  getSettings,
  saveSettingsBulk,
  clockNote,
} from "../lib/api";
import type { DeviceInfo, TimeCheck, Settings } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

export default function Device({ toast }: { toast: Toast }) {
  const [info, setInfo] = useState<DeviceInfo | null>(null);
  const [tc, setTc] = useState<TimeCheck | null>(null);
  const [cfg, setCfg] = useState<Settings>({});
  const [busy, setBusy] = useState(false);

  const probe = async () => {
    setBusy(true);
    try {
      setInfo(await deviceInfo());
      setTc(await checkTimeSync());
      toast("تم الاتصال بالجهاز");
    } catch (e) {
      setInfo(null);
      setTc(null);
      toast(`تعذّر الاتصال: ${e}`, "err");
    } finally {
      setBusy(false);
    }
  };

  const sync = async () => {
    setBusy(true);
    try {
      const r = await syncDevice();
      toast(
        `سحب ناجح: ${r.device.records} سجل، ${r.new_punches} جديد — أُعيد حساب ${r.days_recomputed} يوم${clockNote(r.clock)}`,
      );
      setInfo(r.device);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const fixTime = async () => {
    setBusy(true);
    try {
      const r = await fixDevice_timeSafe();
      toast(`ضُبطت ساعة الجهاز على ${r.set_to}`);
      setTc(await checkTimeSync());
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const fixDevice_timeSafe = async () => fixDeviceTime();

  const loadCfg = () =>
    getSettings()
      .then((s) =>
        setCfg({
          device_ip: s.device_ip || "",
          device_port: s.device_port || "4370",
          device_comm_key: s.device_comm_key || "0",
          auto_sync_min: s.auto_sync_min || "5",
          timezone: s.timezone || "Asia/Jerusalem",
        }),
      )
      .catch(() => {});

  useEffect(() => {
    loadCfg();
  }, []);

  const saveNet = async () => {
    try {
      await saveSettingsBulk(cfg);
      toast("حُفظت إعدادات الشبكة");
      probe();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">جهاز البصمة</h1>
        <div className="flex gap-2">
          <button className="btn-ghost" onClick={probe} disabled={busy}>
            🔎 فحص الاتصال
          </button>
          <button className="btn-primary" onClick={sync} disabled={busy}>
            {busy ? "جارٍ…" : "⬇️ سحب السجلات"}
          </button>
        </div>
      </div>

      {/* إعدادات الشبكة */}
      <div className="card">
        <h2 className="font-bold mb-4">إعدادات الاتصال (LAN)</h2>
        <div className="grid sm:grid-cols-4 gap-4">
          <div>
            <label className="label">عنوان IP</label>
            <input
              className="input font-mono"
              value={cfg.device_ip || ""}
              onChange={(e) => setCfg({ ...cfg, device_ip: e.target.value })}
              placeholder="192.168.1.205"
            />
          </div>
          <div>
            <label className="label">المنفذ</label>
            <input
              className="input font-mono"
              value={cfg.device_port || ""}
              onChange={(e) => setCfg({ ...cfg, device_port: e.target.value })}
            />
          </div>
          <div>
            <label className="label">مفتاح الاتصال (CommKey)</label>
            <input
              className="input font-mono"
              value={cfg.device_comm_key || ""}
              onChange={(e) => setCfg({ ...cfg, device_comm_key: e.target.value })}
            />
          </div>
          <div>
            <label className="label">مزامنة كل (دقيقة)</label>
            <input
              type="number"
              className="input"
              value={cfg.auto_sync_min || ""}
              onChange={(e) => setCfg({ ...cfg, auto_sync_min: e.target.value })}
            />
          </div>
        </div>
        <div className="mt-4">
          <button className="btn-primary" onClick={saveNet}>
            💾 حفظ الإعدادات
          </button>
        </div>
      </div>

      {/* معلومات الجهاز */}
      <div className="card">
        <h2 className="font-bold mb-3">معلومات الجهاز</h2>
        {!info ? (
          <p className="text-sm text-slate-500">
            لم يُفحص بعد — اضغط «فحص الاتصال». تأكّد أن الجهاز مضاء وعلى نفس الشبكة.
          </p>
        ) : (
          <div className="grid sm:grid-cols-3 gap-4 text-sm">
            <Field label="الحالة" value={info.connected ? "✅ متصل" : "❌ منقطع"} />
            <Field label="الطراز" value={info.device_name || "—"} />
            <Field label="المنصة" value={info.platform || "—"} />
            <Field label="البرمجية" value={info.firmware || "—"} />
            <Field label="الرقم التسلسلي" value={info.serial || "—"} />
            <Field label="الموظفون" value={String(info.users)} />
            <Field label="البصمات" value={String(info.fingerprints)} />
            <Field label="سجلات الحضور" value={String(info.records)} />
          </div>
        )}
      </div>

      {/* ساعة الجهاز */}
      <div className="card">
        <div className="flex items-center justify-between mb-3">
          <h2 className="font-bold">🕐 ساعة الجهاز مقابل {cfg.timezone}</h2>
          <button className="btn-ghost" onClick={fixTime} disabled={busy || !tc}>
            ⏱️ ضبط الساعة الآن
          </button>
        </div>
        {!tc ? (
          <p className="text-sm text-slate-500">تعذّر قراءة ساعة الجهاز.</p>
        ) : (
          <div className="grid sm:grid-cols-3 gap-4 text-sm">
            <Field label="وقت الجهاز" value={tc.device_time} mono />
            <Field label="الوقت المحلي" value={tc.server_time} mono />
            <Field
              label="الفرق"
              value={tc.in_sync ? "✅ متزامن" : `${tc.diff_minutes} دقيقة ⚠️`}
            />
          </div>
        )}
        <p className="text-xs text-slate-500 mt-3">
          النظام يتحقق من مطابقة ساعة جهاز البصمة للمنطقة الزمنية المحددة (قابلة للتغيير من
          الإعدادات) ويتيح الضبط المباشر.
        </p>
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div>
      <div className="text-xs text-slate-500">{label}</div>
      <div className={`font-bold ${mono ? "font-mono" : ""}`}>{value}</div>
    </div>
  );
}
