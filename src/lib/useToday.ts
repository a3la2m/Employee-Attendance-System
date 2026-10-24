// تاريخ اليوم المحلي — يتجدّد تلقائياً حتى لو بقي التطبيق مفتوحاً ليلة كاملة،
// فلا تتجمّد الواجهة على تاريخ الأمس (كانت الصفحات تحسب `now` مرة واحدة عند
// أوّل تحميل في بقيت ثابتة حتى إعادة التشغيل).
import { useEffect, useRef, useState } from "react";

/** تاريخ محلي بصيغة YYYY-MM-DD (بلا إزاحة UTC — عكس toISOString). */
export const isoLocal = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(
    d.getDate(),
  ).padStart(2, "0")}`;

/** الشهر المحلي بصيغة YYYY-MM. */
export const monthLocal = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;

/** فحص كل كم ثانية؟ (20 ثانية كافية ليتبدّل التاريخ خلال ثوانٍ من منتصف الليل) */
const TICK_MS = 20_000;

export interface TodayInfo {
  /** تاريخ اليوم المحلي (YYYY-MM-DD) — يتحدّث تلقائياً عند تبدّل اليوم. */
  today: string;
  /** الشهر المحلي (YYYY-MM). */
  month: string;
  /** اليوم الذي كان معروضاً قبل التبدّل — null إن لم يتبدّل بعد فتح الصفحة.
   * تستخدمه الصفحات لترحيل تاريخها/شهرها المُثبَّت إلى اليوم الجديد. */
  rolledFrom: string | null;
}

/**
 * يعيد تاريخ اليوم والشهر المحليين، ويحدّثهما تلقائياً عند منتصف الليل
 * (بلا حاجة لإعادة تشغيل التطبيق). `rolledFrom` يحمل التاريخ السابق لحظة
 * التبدّل حتى تنقل الصفحة تاريخها المُثبَّت إلى اليوم الجديد.
 */
export function useToday(): TodayInfo {
  const [now, setNow] = useState(() => new Date());
  const [rolledFrom, setRolledFrom] = useState<string | null>(null);
  const lastRef = useRef(isoLocal(now));

  useEffect(() => {
    const check = () => {
      const d = new Date();
      if (isoLocal(d) !== lastRef.current) {
        setRolledFrom(lastRef.current);
        lastRef.current = isoLocal(d);
        setNow(d);
      }
    };
    // فحص دوري + فحص عند العودة إلى النافذة (بعد السكون ليلة كاملة)
    const iv = setInterval(check, TICK_MS);
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    return () => {
      clearInterval(iv);
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, []);

  return { today: isoLocal(now), month: monthLocal(now), rolledFrom };
}
