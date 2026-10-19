import { useEffect, useState } from "react";
import {
  dashboard,
  dashboardCharts,
  syncDevice,
  checkTimeSync,
  clockNote,
  presentNow,
} from "../lib/api";
import type {
  Dashboard as Dash,
  DashboardCharts,
  PresentNowItem,
  TimeCheck,
} from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

export default function DashboardPage({ toast }: { toast: Toast }) {
  const [d, setD] = useState<Dash | null>(null);
  const [charts, setCharts] = useState<DashboardCharts | null>(null);
  // undefined = لم ينتهِ الفحص بعد، null = تعذّر الوصول للجهاز
  const [tc, setTc] = useState<TimeCheck | null | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  // الموظفون الموجودون حالياً: سجّلوا دخولاً ولم يسجّلوا خروجاً بعد
  const [present, setPresent] = useState<PresentNowItem[]>([]);

  const loadPresent = () =>
    presentNow().then(setPresent).catch(() => {});

  const load = async () => {
    // تحميل متوازٍ: البطاقات والرسوم تظهران بأسرع ما يمكن بدل التتابع
    const [rd, rc] = await Promise.allSettled([
      dashboard(),
      dashboardCharts(),
    ]);
    if (rd.status === "fulfilled") setD(rd.value);
    else toast(String(rd.reason), "err");
    setCharts(rc.status === "fulfilled" ? rc.value : null);
    loadPresent();
  };

  useEffect(() => {
    load();
    // فحص ساعة الجهاز مؤجَّل قليلاً حتى لا يتحمّل اتصال الجهاز البطيء
    // (مهلة 4 ثوانٍ) مع عرض البيانات — يظهر «جارٍ الفحص» في بطاحته.
    const t = setTimeout(() => {
      checkTimeSync()
        .then(setTc)
        .catch(() => setTc(null));
    }, 700);
    const iv = setInterval(loadPresent, 30000); // تحديث الحاضرين تلقائياً
    return () => {
      clearTimeout(t);
      clearInterval(iv);
    };
  }, []);

  const sync = async () => {
    setBusy(true);
    try {
      const r = await syncDevice();
      toast(
        `مزامنة ناجحة: ${r.new_punches} بصمة + ${r.new_users_added} موظف — أُعيد حساب ${r.days_recomputed} يوم${clockNote(r.clock)}`,
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
          <div
            key={c.label}
            className={`card flex items-center gap-4 ${!d ? "animate-pulse" : ""}`}
          >
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

      {/* الموجودون حالياً: دُخل ولم يخرج بعد */}
      <div className="card">
        <h2 className="font-bold mb-3">
          🏢 الموجودون حالياً
          <span className="text-sm font-normal text-slate-500 mr-2">
            (سجّلوا دخولاً اليوم ولم يسجّلوا خروجاً بعد)
          </span>
        </h2>
        {present.length === 0 ? (
          <p className="text-sm text-slate-400">
            لا يوجد أحد حاضراً الآن — أول خروج يظهر الموظفون فوراً بعد تسجيل
            الدخول من الجهاز.
          </p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {present.map((p) => (
              <span
                key={p.employee_id}
                className="inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-emerald-50 border border-emerald-200 text-sm"
              >
                <span className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
                <span className="font-bold">{p.name}</span>
                {p.pin && (
                  <span className="text-xs text-slate-400 font-mono">
                    #{p.pin}
                  </span>
                )}
                <span className="text-xs text-emerald-700 font-mono">
                  دخول {p.last_in.split("T").pop()?.slice(0, 5) || "—"}
                </span>
              </span>
            ))}
          </div>
        )}
      </div>

      {/* الرسوم البيانية */}
      <Charts data={charts} />

      {/* حالة ساعة الجهاز */}
      <div className="card">
        <h2 className="font-bold mb-3">🕐 حالة ساعة الجهاز</h2>
        {tc === undefined ? (
          <p className="text-sm text-slate-500 animate-pulse">
            ⏳ جارٍ فحص الساعة عبر الشبكة…
          </p>
        ) : !tc ? (
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

// ------------------------------------------------------------- الرسوم البيانية

const BAR_COLORS = {
  normal: "#10b981", // أخضر — حاضر
  late: "#f59e0b", // كهرماني — متأخر
  leave: "#0ea5e9", // أزرق فاتح — إجازة
  holiday: "#8b5cf6", // بنفسجي — عطلة رسمية
  weekend: "#94a3b8", // رمادي — عطلة أسبوعية
  absent: "#f43f5e", // وردي — غائب
};

function LegendDot({ c, t }: { c: string; t: string }) {
  return (
    <span className="flex items-center gap-1.5">
      <span className="w-3 h-3 rounded-full" style={{ background: c }} />
      {t}
    </span>
  );
}

/** مجموع الرسوم: أعمدة آخر 7 أيام + ملخص الشهر (دائري) + الأعلى بالساعات. */
function Charts({ data }: { data: DashboardCharts | null }) {
  if (!data) return null;
  const { last7, month, top } = data;
  const empty =
    last7.every((d) => d.present + d.absent + d.leave + (d.holiday || 0) === 0) &&
    top.length === 0;
  if (empty)
    return (
      <div className="card text-center text-sm text-slate-500 py-8">
        📊 لا توجد بيانات حضور بعد — اسحب من الجهاز لعرض الرسوم.
      </div>
    );

  // ---- أعمدة آخر 7 أيام (الأقدم يميناً، اليوم أقصى اليسار — اتجاه عربي)
  const W = 560;
  const H = 230;
  const padT = 18;
  const padB = 36;
  const padX = 26;
  const n = last7.length;
  const step = (W - padX * 2) / n;
  const bw = Math.min(40, step * 0.52);
  const maxTot = Math.max(
    1,
    ...last7.map(
      (d) => Math.max(0, d.present - d.late) + d.late + d.absent + d.leave + (d.holiday || 0),
    ),
  );
  const chartH = H - padT - padB;
  const yOf = (v: number) => padT + chartH * (1 - v / maxTot);
  const xOf = (i: number) => padX + (n - 1 - i) * step + (step - bw) / 2;

  // ---- دائرة ملخص الشهر
  const donut = [
    { label: "حاضر", v: Math.max(0, month.present - month.late), c: BAR_COLORS.normal },
    { label: "متأخر", v: month.late, c: BAR_COLORS.late },
    { label: "إجازة", v: month.leave, c: BAR_COLORS.leave },
    { label: "عطلة رسمية", v: month.holiday || 0, c: BAR_COLORS.holiday },
    { label: "عطلة أسبوعية", v: month.weekend || 0, c: BAR_COLORS.weekend },
    { label: "غائب", v: month.absent, c: BAR_COLORS.absent },
  ];
  const donutTotal = Math.max(1, donut.reduce((a, b) => a + b.v, 0));
  const R = 52;
  const CIRC = 2 * Math.PI * R;
  let acc = 0;
  const donutSegs = donut.map((s) => {
    const frac = s.v / donutTotal;
    const seg = { ...s, frac, offset: acc };
    acc += frac;
    return seg;
  });

  const maxTop = Math.max(1, ...top.map((t) => t.hours));

  return (
    <div className="grid lg:grid-cols-3 gap-4">
      {/* آخر 7 أيام */}
      <div className="card lg:col-span-2">
        <h2 className="font-bold mb-1">📊 الحضور خلال آخر 7 أيام</h2>
        <p className="text-xs text-slate-500 mb-2">
          من اليمين (الأقدم) إلى اليسار (اليوم) — مرِّر الفأرة فوق أي عمود للتفاصيل
        </p>
        <svg viewBox={`0 0 ${W} ${H}`} className="w-full h-auto">
          {/* خطوط الشبكة */}
          {[0, 0.5, 1].map((f) => (
            <g key={f}>
              <line
                x1={padX}
                x2={W - padX}
                y1={yOf(maxTot * f)}
                y2={yOf(maxTot * f)}
                stroke="#e2e8f0"
                strokeDasharray="4 4"
              />
              <text
                x={W - padX + 4}
                y={yOf(maxTot * f) + 3}
                fontSize="8"
                className="fill-slate-300"
              >
                {Math.round(maxTot * f)}
              </text>
            </g>
          ))}
          {last7.map((d, i) => {
            const normal = Math.max(0, d.present - d.late);
            const segs = [
              { v: normal, c: BAR_COLORS.normal, t: "حاضر" },
              { v: d.late, c: BAR_COLORS.late, t: "متأخر" },
              { v: d.leave, c: BAR_COLORS.leave, t: "إجازة" },
              { v: d.holiday || 0, c: BAR_COLORS.holiday, t: "عطلة رسمية" },
              { v: d.absent, c: BAR_COLORS.absent, t: "غائب" },
            ];
            const total = segs.reduce((a, s) => a + s.v, 0);
            let y = H - padB;
            const x = xOf(i);
            const isToday = i === n - 1;
            return (
              <g key={d.date}>
                {d.weekend && (
                  <rect
                    x={x - 3}
                    y={padT - 4}
                    width={bw + 6}
                    height={H - padB - padT + 34}
                    rx={6}
                    fill="#f8fafc"
                  />
                )}
                {segs.map((s, si) => {
                  if (s.v <= 0) return null;
                  const h = chartH * (s.v / maxTot);
                  y -= h;
                  return (
                    <rect
                      key={si}
                      x={x}
                      y={y}
                      width={bw}
                      height={Math.max(3, h - 1.5)}
                      rx={4}
                      fill={s.c}
                      opacity={isToday ? 1 : 0.82}
                    >
                      <title>{`${d.label} ${d.date} — ${s.t}: ${s.v}`}</title>
                    </rect>
                  );
                })}
                {total > 0 && (
                  <text
                    x={x + bw / 2}
                    y={Math.max(10, yOf(total) - 5)}
                    textAnchor="middle"
                    fontSize="10"
                    fontWeight="700"
                    className="fill-slate-500"
                  >
                    {total}
                  </text>
                )}
                <text
                  x={x + bw / 2}
                  y={H - padB + 15}
                  textAnchor="middle"
                  fontSize="10.5"
                  fontWeight={isToday ? 800 : 500}
                  className={isToday ? "fill-indigo-600" : "fill-slate-500"}
                >
                  {d.label}
                </text>
                <text
                  x={x + bw / 2}
                  y={H - padB + 27}
                  textAnchor="middle"
                  fontSize="8.5"
                  className={isToday ? "fill-indigo-400" : "fill-slate-300"}
                >
                  {d.date.slice(8)}
                </text>
              </g>
            );
          })}
        </svg>
        <div className="flex flex-wrap gap-4 justify-center text-xs mt-2 text-slate-600">
          <LegendDot c={BAR_COLORS.normal} t="حاضر" />
          <LegendDot c={BAR_COLORS.late} t="متأخر" />
          <LegendDot c={BAR_COLORS.leave} t="إجازة" />
          <LegendDot c={BAR_COLORS.holiday} t="عطلة رسمية" />
          <LegendDot c={BAR_COLORS.weekend} t="عطلة أسبوعية" />
          <LegendDot c={BAR_COLORS.absent} t="غائب" />
        </div>
      </div>

      {/* ملخص الشهر */}
      <div className="card">
        <h2 className="font-bold mb-3">🗓️ ملخص الشهر الحالي</h2>
        <div className="flex items-center gap-4">
          <svg viewBox="0 0 140 140" className="w-32 h-32 shrink-0">
            <circle cx="70" cy="70" r={R} fill="none" stroke="#f1f5f9" strokeWidth="17" />
            {donutSegs.map(
              (s) =>
                s.v > 0 && (
                  <circle
                    key={s.label}
                    cx="70"
                    cy="70"
                    r={R}
                    fill="none"
                    stroke={s.c}
                    strokeWidth="17"
                    strokeDasharray={`${s.frac * CIRC} ${CIRC}`}
                    strokeDashoffset={-s.offset * CIRC}
                    transform="rotate(-90 70 70)"
                  >
                    <title>{`${s.label}: ${s.v} يوم`}</title>
                  </circle>
                ),
            )}
            <text
              x="70"
              y="67"
              textAnchor="middle"
              fontSize="24"
              fontWeight="800"
              className="fill-slate-800"
            >
              {month.hours.toFixed(0)}
            </text>
            <text x="70" y="84" textAnchor="middle" fontSize="10" className="fill-slate-400">
              ساعة عمل
            </text>
          </svg>
          <div className="flex-1 space-y-2 text-sm">
            {donutSegs.map((s) => (
              <div key={s.label} className="flex items-center gap-2">
                <span
                  className="w-2.5 h-2.5 rounded-full shrink-0"
                  style={{ background: s.c }}
                />
                <span className="flex-1 text-slate-600">{s.label}</span>
                <span className="font-bold">{s.v}</span>
              </div>
            ))}
            <div className="pt-2 border-t border-slate-100 flex items-center gap-2 text-xs text-slate-500">
              <span>أيام العمل حتى اليوم</span>
              <span className="font-bold text-slate-700">{month.workdays}</span>
            </div>
          </div>
        </div>
      </div>

      {/* الأعلى اجتهاداً */}
      <div className="card lg:col-span-3">
        <h2 className="font-bold mb-1">🏆 الأعلى اجتهاداً خلال 30 يوماً</h2>
        <p className="text-xs text-slate-500 mb-3">حسب إجمالي ساعات العمل المحسوبة</p>
        {top.length === 0 ? (
          <p className="text-sm text-slate-500">لا توجد ساعات مسجلة بعد.</p>
        ) : (
          <div className="space-y-2.5">
            {top.map((t, i) => (
              <div key={t.name} className="flex items-center gap-3">
                <span
                  className={`w-7 h-7 shrink-0 rounded-full grid place-content-center text-xs font-extrabold ${
                    i === 0
                      ? "bg-amber-100 text-amber-700"
                      : i === 1
                        ? "bg-slate-200 text-slate-600"
                        : i === 2
                          ? "bg-orange-100 text-orange-700"
                          : "bg-slate-100 text-slate-500"
                  }`}
                >
                  {i + 1}
                </span>
                <span className="w-40 truncate text-sm font-semibold">{t.name}</span>
                <div
                  className="flex-1 h-6 bg-slate-100 rounded-full overflow-hidden"
                  dir="ltr"
                >
                  <div
                    className="h-full rounded-full bg-gradient-to-l from-indigo-600 to-sky-400 flex items-center justify-end pr-2.5 text-[11px] text-white font-bold shadow-sm"
                    style={{
                      width: `${Math.max(10, (t.hours / maxTop) * 100)}%`,
                      transition: "width .6s ease",
                    }}
                    title={`${t.hours.toFixed(1)} ساعة`}
                  >
                    {t.hours.toFixed(1)} س
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
