import { useEffect, useRef, useState } from "react";
import type { ComponentType } from "react";
import Dashboard from "./pages/Dashboard";
import Employees from "./pages/Employees";
import Attendance from "./pages/Attendance";
import Payroll from "./pages/Payroll";
import Reports from "./pages/Reports";
import Device from "./pages/Device";
import Settings from "./pages/Settings";
import Telegram from "./pages/Telegram";
import Toast from "./components/Toast";
import { useConfirm } from "./components/Confirm";
import { listen } from "@tauri-apps/api/event";
import {
  listNotifications,
  markNotificationsSeen,
  clearNotifications,
  leaveRequestsList,
  advanceRequestsList,
} from "./lib/api";
import type { AdvanceRequest, Notification } from "./lib/types";
import BotRequestsModal from "./components/BotRequestsModal";

type TabKey =
  | "dashboard"
  | "employees"
  | "attendance"
  | "payroll"
  | "reports"
  | "device"
  | "telegram"
  | "settings";

/** مفاتيح الشاشات المجمّدة مؤقتاً (تظهر في القائمة برمز 🔒 ولا تُفتح). */
const FROZEN: TabKey[] = [];

const NAV: { key: TabKey; label: string; icon: string }[] = [
  { key: "dashboard", label: "لوحة التحكم", icon: "📊" },
  { key: "employees", label: "الموظفون", icon: "👥" },
  { key: "attendance", label: "الحضور", icon: "🗓️" },
  { key: "payroll", label: "الرواتب", icon: "💰" },
  { key: "reports", label: "تقارير", icon: "📋" },
  { key: "device", label: "جهاز البصمة", icon: "🔐" },
  { key: "telegram", label: "بوت تلجرام", icon: "🤖" },
  { key: "settings", label: "الإعدادات", icon: "⚙️" },
];

const PAGES: Record<TabKey, ComponentType<{ toast: (m: string, t?: "ok" | "err") => void }>> = {
  dashboard: Dashboard,
  employees: Employees,
  attendance: Attendance,
  payroll: Payroll,
  reports: Reports,
  device: Device,
  telegram: Telegram,
  settings: Settings,
};

/** طلب فتح شاشة الحضور على تاريخ معيّن (يُستخدم من تنبيه «يوم بدل»). */
export interface AttendanceFocus {
  date: string;
  employeeId?: number;
  /** فتح نافذة «يوم بدل» تلقائياً لهذا الموظف. */
  openComp?: boolean;
}

const KIND_ICON: Record<string, string> = {
  late: "⏰",
  missing_out: "🚪",
  orphan_out: "↩️",
  comp_off: "🎁",
  leave_work: "🔄",
  absent: "📌",
  early_leave: "⏳",
  no_punches: "📅",
  no_checkout: "🚪",
  absent_eod: "📌",
};

export default function App() {
  const [tab, setTab] = useState<TabKey>("dashboard");
  const [toastMsg, setToastMsg] = useState<{ m: string; t: "ok" | "err" } | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  // طلب فتح الحضور على تاريخ معيّن (من تنبيه «يوم بدل»)
  const [attFocus, setAttFocus] = useState<AttendanceFocus | null>(null);
  // سجل التنبيهات + عدّاد الجرس (كل التنبيهات محفوظة ويمكن العودة إليها)
  const [notifs, setNotifs] = useState<Notification[]>([]);
  const [bellOpen, setBellOpen] = useState(false);
  const bellRef = useRef<HTMLDivElement>(null);
  const unseen = notifs.filter((n) => !n.seen).length;

  // طلبات الإجازات والسلف الواردة من البوت + عدّاد الطلبات المعلقة
  const [botReqOpen, setBotReqOpen] = useState(false);
  const [pendingBotCount, setPendingBotCount] = useState(0);
  const [advFocus, setAdvFocus] = useState<AdvanceRequest | null>(null);

  const toast = (m: string, t: "ok" | "err" = "ok") => setToastMsg({ m, t });
  // تأكيد «نعم/لا» داخلي (بديل window.confirm — لا يستجيب للنقرة العابرة)
  const confirm = useConfirm();

  /** نقل المستخدم إلى سجل الحضور على اليوم ذي الصلة بالتنبيه. */
  const gotoDay = (n: Notification) => {
    setBellOpen(false);
    if (FROZEN.includes("attendance")) {
      toast("شاشة الحضور مجمّدة مؤقتاً 🔒 — لن يُفتح التنبيه الآن", "err");
      return;
    }
    setAttFocus({
      date: n.day,
      employeeId: n.employee_id ?? undefined,
      openComp: n.kind === "comp_off",
    });
    setTab("attendance");
  };

  const loadBotRequestsCount = () => {
    Promise.all([leaveRequestsList(), advanceRequestsList()])
      .then(([lv, adv]) => {
        const cnt =
          lv.filter((x) => x.status === "pending").length +
          adv.filter((x) => x.status === "pending").length;
        setPendingBotCount(cnt);
      })
      .catch(() => {});
  };

  const loadNotifs = () => {
    listNotifications(200).then(setNotifs).catch(() => {});
    loadBotRequestsCount();
  };

  useEffect(() => {
    loadNotifs();
    const iv = setInterval(loadNotifs, 15000); // تحديث العدّاد تلقائياً
    return () => clearInterval(iv);
  }, []);

  // الضغط خارج لوحة الإشعارات يُغلقها
  useEffect(() => {
    if (!bellOpen) return;
    const onDocDown = (e: MouseEvent) => {
      if (!bellRef.current?.contains(e.target as Node)) setBellOpen(false);
    };
    document.addEventListener("mousedown", onDocDown);
    return () => document.removeEventListener("mousedown", onDocDown);
  }, [bellOpen]);

  // فتح الجرس: يعرض السجل ويصفّر العدّاد (تعليم الكل كمقروء)
  const openBell = () => {
    const next = !bellOpen;
    setBellOpen(next);
    if (next && unseen > 0) {
      setNotifs((ns) => ns.map((n) => ({ ...n, seen: true })));
      markNotificationsSeen().catch(() => {});
    }
  };

  const wipeNotifs = async () => {
    const ok = await confirm("مسح كل التنبيهات المحفوظة نهائياً؟", {
      title: "مسح التنبيهات",
      yes: "نعم، امسح الكل",
    });
    if (!ok) return;
    setNotifs([]);
    setBellOpen(false);
    clearNotifications().catch(() => {});
  };

  useEffect(() => {
    // مزامنة تلقائية تُبلّغ من الخلفية
    const un1 = listen<{
      new_punches?: number;
      ok: boolean;
      error?: string;
      clock_diff_min?: number;
      device_time?: string;
      clock_fixed?: { set_to?: string; diff_min?: number };
    }>("sync", (e) => {
      const fixed = e.payload.clock_fixed;
      const skew = e.payload.clock_diff_min;
      if (fixed) {
        const hhmm = fixed.set_to ? fixed.set_to.slice(11, 16) : "—";
        const m = Math.round(Math.abs(fixed.diff_min ?? 0));
        toast(
          `✅ صُحّحت ساعة الجهاز تلقائياً إلى ${hhmm} (كانت تختلف بحوالي ${m} دقيقة)${
            m >= 50 && m <= 70 ? " — تغيّر التوقيت الصيفي/الشتوي 🕐" : ""
          }`,
          "ok",
        );
      } else if (typeof skew === "number" && Math.abs(skew) > 5) {
        const m = Math.round(Math.abs(skew));
        toast(
          m >= 50 && m <= 70
            ? `⚠️ ساعة الجهاز تختلف بمقدار ${m} دقيقة (ساعة كاملة) — غالباً بعد تغيير التوقيت الصيفي/الشتوي. افتح «الجهاز» واضغط «ضبط الساعة الآن»، أو فعّل «تصحيح ساعة الجهاز تلقائياً» من الإعدادات.`
            : Math.abs(skew) > 120
              ? `⚠️ ساعة الجهاز خاطئة (تُظهر ${e.payload.device_time ?? "؟؟"}) — بصمات اليوم ستُسجل بتاريخ خاطئ. افتح «الجهاز» واضغط «تصحيح الوقت».`
              : `⚠️ ساعة الجهاز تختلف بحوالي ${m} دقيقة — صحّحها من صفحة «الجهاز».`,
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
    // طلب إجازة أو سلفة جديد من بوت تلجرام
    const un3 = listen<{ kind?: string }>("bot-request", (e) => {
      loadBotRequestsCount();
      toast(
        e.payload?.kind === "advance"
          ? "💵 وصل طلب سلفة جديد من البوت — افتح «طلبات الإجازات والسلف» للمراجعة"
          : "🏖️ وصل طلب إجازة جديد من البوت — افتح «طلبات الإجازات والسلف» للمراجعة",
        "ok",
      );
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
      un3.then((f) => f());
    };
  }, []);

  const Page = PAGES[tab];

  return (
    <div className="flex h-full">
      {/* الشريط الجانبي */}
      <aside className="w-60 shrink-0 bg-slate-900 text-slate-100 flex flex-col">
        <div ref={bellRef} className="px-5 py-5 border-b border-slate-800 relative">
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="text-lg font-extrabold">نظام الحضور</div>
              <div className="text-xs text-slate-400 mt-0.5">والرواتب — بصمة K50</div>
            </div>
            {/* زر الإشعارات: سجل التنبيهات + عدّاد غير المقروء */}
            <button
                onClick={openBell}
                title="التنبيهات (كل التنبيهات محفوظة)"
                aria-label={`التنبيهات${unseen > 0 ? ` — ${unseen} غير مقروء` : ""}`}
                aria-expanded={bellOpen}
                className="relative shrink-0 w-9 h-9 rounded-full bg-slate-800 hover:bg-slate-700 grid place-content-center text-base transition"
              >
                🔔
                {unseen > 0 && (
                  <span className="absolute -top-1 -left-1 min-w-[18px] h-[18px] px-1 rounded-full bg-rose-500 text-white text-[10px] font-extrabold grid place-content-center">
                    {unseen > 99 ? "99+" : unseen}
                  </span>
                )}
              </button>
          </div>

          {/* زر بارز لطلبات الإجازات والسلف مع العدّاد */}
          <button
            type="button"
            onClick={() => {
              setBellOpen(false);
              setBotReqOpen(true);
            }}
            className={`mt-3 w-full flex items-center justify-between px-3 py-2 rounded-lg text-xs font-bold transition ${
              pendingBotCount > 0
                ? "bg-amber-500/20 text-amber-300 border border-amber-500/40 hover:bg-amber-500/30"
                : "bg-slate-800/80 text-slate-300 hover:bg-slate-800"
            }`}
          >
            <span>📥 طلبات الإجازات والسلف</span>
            <span
              className={`px-2 py-0.5 rounded-full text-[11px] font-extrabold ${
                pendingBotCount > 0
                  ? "bg-amber-400 text-slate-950"
                  : "bg-slate-700 text-slate-300"
              }`}
            >
              {pendingBotCount}
            </span>
          </button>

          {/* لوحة التنبيهات المنسدلة */}
          {bellOpen && (
            <div className="absolute top-full right-0 mt-2 w-[22rem] max-h-[28rem] flex flex-col bg-white text-slate-800 rounded-xl shadow-2xl border border-slate-200 z-50">
              <div className="flex items-center justify-between px-4 py-2.5 border-b border-slate-100">
                <div className="font-extrabold text-sm">
                  🔔 التنبيهات
                  <span className="text-slate-400 font-normal mr-1">
                    ({notifs.length})
                  </span>
                </div>
                <button
                  className="text-slate-400 hover:text-slate-700 text-sm"
                  onClick={() => setBellOpen(false)}
                >
                  ✕
                </button>
              </div>
              <div className="overflow-auto flex-1">
                {notifs.length === 0 && (
                  <div className="p-6 text-center text-sm text-slate-400">
                    لا توجد تنبيهات بعد ✓
                  </div>
                )}
                {notifs.map((n) => {
                  const actionable =
                    n.kind === "comp_off" ||
                    n.kind === "missing_out" ||
                    n.kind === "orphan_out" ||
                    n.kind === "absent" ||
                    n.kind === "early_leave" ||
                    n.kind === "no_punches" ||
                    n.kind === "no_checkout" ||
                    n.kind === "absent_eod";
                  return (
                  <div
                    key={n.id}
                    onClick={actionable ? () => gotoDay(n) : undefined}
                    className={`px-4 py-2.5 border-b border-slate-50 flex gap-3 ${
                      n.seen ? "" : "bg-indigo-50"
                    } ${actionable ? "cursor-pointer hover:bg-sky-50" : ""}`}
                    title={
                      n.kind === "comp_off"
                        ? "اضغط للانتقال لصف يوم العطلة وإسناد يوم بدل"
                        : actionable
                          ? "اضغط للانتقال ليوم الحضور لعرض التفاصيل وإدخال تعديل يدوياً"
                          : undefined
                    }
                  >
                    <span className="text-base mt-0.5">
                      {KIND_ICON[n.kind] ?? "🔔"}
                    </span>
                    <div className="min-w-0">
                      <div className="text-xs font-extrabold leading-tight">
                        {n.title}
                      </div>
                      <div className="text-[11px] text-slate-600 leading-snug mt-0.5">
                        {n.body}
                      </div>
                      <div className="text-[10px] text-slate-400 font-mono mt-0.5">
                        {n.day} · {n.created_at.slice(11, 16)}
                        {actionable && (
                          <span className="text-sky-600 font-bold mr-1">
                            {" "}
                            · فتح اليوم لتعديل يدوياً ←
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                  );
                })}
              </div>
              <div className="flex items-center justify-between px-3 py-2 border-t border-slate-100 text-xs">
                <button
                  className="text-emerald-600 hover:underline font-bold"
                  onClick={() => {
                    setNotifs((ns) => ns.map((n) => ({ ...n, seen: true })));
                    markNotificationsSeen().catch(() => {});
                  }}
                >
                  ✓ تحديد الكل كمقروء
                </button>
                <button
                  className="text-rose-500 hover:underline font-bold"
                  onClick={wipeNotifs}
                >
                  🗑 مسح الكل
                </button>
              </div>
            </div>
          )}
        </div>
        <nav className="flex-1 p-3 space-y-1">
          {NAV.map((n) => {
            const active = tab === n.key;
            const locked = FROZEN.includes(n.key);
            return (
              <button
                key={n.key}
                onClick={() => {
                  if (locked) {
                    toast(`شاشة «${n.label}» مجمّدة مؤقتاً 🔒`, "err");
                    return;
                  }
                  setTab(n.key);
                }}
                title={
                  locked
                    ? "شاشة مجمّدة مؤقتاً — ستُفعَّل في تحديث لاحق"
                    : undefined
                }
                className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition ${
                  active
                    ? "bg-indigo-600 text-white font-bold"
                    : locked
                      ? "text-slate-500 hover:bg-slate-800/60"
                      : "text-slate-300 hover:bg-slate-800"
                }`}
              >
                <span className="text-base">{n.icon}</span>
                <span className="flex-1 text-right">{n.label}</span>
                {locked && <span className="text-xs">🔒</span>}
              </button>
            );
          })}
        </nav>
        <div className="p-4 text-[11px] text-slate-500 border-t border-slate-800 space-y-1">
          <div>يعمل في الخلفية كأيقونة في شريط إشعارات النظام</div>
          <div className="text-slate-600 select-none">
            © 2026{" "}
            <a
              href="https://ordoo.shop"
              target="_blank"
              rel="noreferrer"
              className="text-sky-400 hover:underline"
            >
              ordoo.shop
            </a>{" "}
            — جميع الحقوق محفوظة
          </div>
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
        {tab === "attendance" ? (
          <Attendance
            toast={toast}
            focus={attFocus}
            onFocusHandled={() => setAttFocus(null)}
          />
        ) : tab === "employees" ? (
          <Employees
            toast={toast}
            focusAdvance={advFocus}
            onAdvanceHandled={() => {
              setAdvFocus(null);
              loadBotRequestsCount();
            }}
          />
        ) : (
          <Page toast={toast} />
        )}
      </main>
      {botReqOpen && (
        <BotRequestsModal
          onClose={() => {
            setBotReqOpen(false);
            loadBotRequestsCount();
          }}
          toast={toast}
          onChanged={loadBotRequestsCount}
          onOpenEmployeeAdvance={(req) => {
            setBotReqOpen(false);
            setAdvFocus(req);
            setTab("employees");
          }}
        />
      )}
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
