import { useEffect, useMemo, useState } from "react";
import {
  getSettings,
  saveSetting,
  saveSettingsBulk,
  telegramTest,
  telegramSupervisors,
  telegramAddSupervisor,
  telegramDeleteSupervisor,
  telegramUnlinkSupervisor,
  telegramLinks,
  telegramUnlinkEmployee,
  telegramSetBotAllowed,
  telegramBroadcast,
  absenceNotesList,
  telegramChatMessages,
} from "../lib/api";
import type {
  AbsenceNote,
  TelegramChatMessage,
  TelegramLink,
  TelegramSupervisor,
} from "../lib/types";
import { useConfirm } from "../components/Confirm";
import Modal from "../components/Modal";

type Toast = (m: string, t?: "ok" | "err") => void;

/** مقدمة جاهزة لنص التعميم (قابلة للتعديل أو الحذف). */
const BC_PREFIX = "📣 تعميم\n\n";

/** خيارات الموظف التي يمكن السماح بها أو منعها من البوت. */
const PERMS: { key: string; label: string; hint: string }[] = [
  {
    key: "tg_allow_today",
    label: "حالة اليوم",
    hint: "الأمر /اليوم — هل سجّل دخوله وخروجه اليوم",
  },
  {
    key: "tg_allow_sheet",
    label: "كشف دوام الشهر الحالي",
    hint: "الأمر /كشف — تفاصيل أيام الشهر الحالي",
  },
  {
    key: "tg_allow_month",
    label: "كشف شهر محدد",
    hint: "الأمر /كشف 2026-09 — أي شهر سابق",
  },
  {
    key: "tg_allow_salary",
    label: "آخر راتب",
    hint: "الأمر /راتب — عرض آخر راتب مدفوع",
  },
  {
    key: "tg_allow_leave",
    label: "طلبات الإجازات",
    hint: "الطلب من البوت — سنوية / مرضية بمرفق / بدون راتب / طارئة",
  },
  {
    key: "tg_allow_advance",
    label: "طلبات السلف",
    hint: "طلب سلفة مالية من البوت بمبلغ يختاره الموظف",
  },
  {
    key: "tg_allow_reason",
    label: "تسجيل سبب الغياب",
    hint: "الأمر /سبب ... — يصل سببه للمشرفين",
  },
];

const dt = (iso: string | null) =>
  iso ? `${iso.slice(0, 10)} ${iso.slice(11, 16)}` : "—";

export default function Telegram({ toast }: { toast: Toast }) {
  const confirm = useConfirm();

  // إعدادات البوت (نُعدّلها محلياً ثم نحفظ)
  const [token, setToken] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [alertStart, setAlertStart] = useState(true);
  const [alertEnd, setAlertEnd] = useState(true);
  const [perms, setPerms] = useState<Record<string, boolean>>({});
  const [showToken, setShowToken] = useState(false);

  const [savingBot, setSavingBot] = useState(false);
  const [testing, setTesting] = useState(false);
  const [botName, setBotName] = useState<string | null>(null);

  const [supervisors, setSupervisors] = useState<TelegramSupervisor[]>([]);
  const [supName, setSupName] = useState("");
  const [supPhone, setSupPhone] = useState("");
  const [addingSup, setAddingSup] = useState(false);

  const [links, setLinks] = useState<TelegramLink[]>([]);
  const [search, setSearch] = useState("");

  const [notes, setNotes] = useState<AbsenceNote[]>([]);
  const [busy, setBusy] = useState(false);
  const [guideOpen, setGuideOpen] = useState(false);

  const [bcOpen, setBcOpen] = useState(false);
  const [bcTarget, setBcTarget] = useState("all");
  const [bcText, setBcText] = useState(BC_PREFIX);
  const [broadcasting, setBroadcasting] = useState(false);

  // نافذة سجل المحادثات مع الموظفين
  const [logOpen, setLogOpen] = useState(false);
  const [chatMsgs, setChatMsgs] = useState<TelegramChatMessage[]>([]);
  const [logEmpId, setLogEmpId] = useState<number | "all">("all");
  const [logSearch, setLogSearch] = useState("");
  const [logLoading, setLogLoading] = useState(false);
  const [logPreviewImg, setLogPreviewImg] = useState<string | null>(null);

  const loadChatLog = async () => {
    setLogLoading(true);
    try {
      const rows = await telegramChatMessages(null, null, 500);
      setChatMsgs(rows);
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setLogLoading(false);
    }
  };

  const openChatLog = () => {
    setLogOpen(true);
    loadChatLog();
  };

  const load = async () => {
    try {
      const s = await getSettings();
      setToken(s.tg_token ?? "");
      setEnabled((s.tg_enabled ?? "0") !== "0");
      setAlertStart((s.tg_alert_start ?? "1") !== "0");
      setAlertEnd((s.tg_alert_end ?? "1") !== "0");
      const p: Record<string, boolean> = {};
      for (const it of PERMS) p[it.key] = (s[it.key] ?? "1") !== "0";
      setPerms(p);
      const [sup, ln, nt] = await Promise.all([
        telegramSupervisors(),
        telegramLinks(),
        absenceNotesList(50),
      ]);
      setSupervisors(sup);
      setLinks(ln);
      setNotes(nt);
    } catch (e) {
      toast(String(e), "err");
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const linkedCount = useMemo(
    () => links.filter((l) => l.chat_id).length,
    [links],
  );
  const supLinked = useMemo(
    () => supervisors.filter((s) => s.chat_id).length,
    [supervisors],
  );
  const filtered = useMemo(() => {
    const q = search.trim();
    if (!q) return links;
    return links.filter(
      (l) => l.name.includes(q) || l.phone.includes(q),
    );
  }, [links, search]);

  const toggleBotRunning = async () => {
    const next = !enabled;
    if (next && !token.trim()) {
      toast("أدخل توكن البوت واحفظه أولاً", "err");
      return;
    }
    setEnabled(next);
    try {
      await saveSetting("tg_enabled", next ? "1" : "0");
      toast(
        next
          ? "تم تشغيل البوت واستئناف الرد على الرسائل ✅"
          : "تم إيقاف البوت — لن يرد على الرسائل ولن يرسل تنبيهات",
        "ok",
      );
    } catch (e) {
      setEnabled(!next);
      toast(String(e), "err");
    }
  };

  const saveBot = async () => {
    setSavingBot(true);
    try {
      const items: Record<string, string> = {
        tg_token: token.trim(),
        tg_enabled: enabled ? "1" : "0",
        tg_alert_start: alertStart ? "1" : "0",
        tg_alert_end: alertEnd ? "1" : "0",
      };
      await saveSettingsBulk(items);
      toast("تم حفظ إعدادات البوت ✅", "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setSavingBot(false);
    }
  };

  const test = async () => {
    setTesting(true);
    setBotName(null);
    try {
      // نحفظ التوكن أولاً حتى يقرأه الاختبار من قاعدة البيانات
      await saveSetting("tg_token", token.trim());
      const name = await telegramTest();
      setBotName(name);
      toast(`الاتصال ناجح — ${name}`, "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setTesting(false);
    }
  };

  const togglePerm = async (key: string, on: boolean) => {
    setPerms((p) => ({ ...p, [key]: on }));
    try {
      await saveSetting(key, on ? "1" : "0");
    } catch (e) {
      toast(String(e), "err");
      setPerms((p) => ({ ...p, [key]: !on }));
    }
  };

  const addSup = async () => {
    if (!supName.trim() || !supPhone.trim()) {
      toast("أدخل اسم المشرف ورقم هاتفه", "err");
      return;
    }
    setAddingSup(true);
    try {
      await telegramAddSupervisor(supName.trim(), supPhone.trim());
      setSupName("");
      setSupPhone("");
      setSupervisors(await telegramSupervisors());
      toast("أُضيف المشرف — عليه الآن مشاركة رقمه مع البوت ✅", "ok");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setAddingSup(false);
    }
  };

  const delSup = async (s: TelegramSupervisor) => {
    const ok = await confirm(
      `حذف «${s.name}» من قائمة المشرفين؟ لن تصله تنبيهات بعد الآن.`,
      { title: "حذف مشرف", yes: "حذف" },
    );
    if (!ok) return;
    try {
      await telegramDeleteSupervisor(s.id);
      setSupervisors(await telegramSupervisors());
      toast("حُذف المشرف");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const unlinkSup = async (s: TelegramSupervisor) => {
    try {
      await telegramUnlinkSupervisor(s.id);
      setSupervisors(await telegramSupervisors());
      toast("أُلغي ربط المشرف — عليه مشاركة رقمه مجدداً");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const unlinkEmp = async (l: TelegramLink) => {
    try {
      await telegramUnlinkEmployee(l.employee_id);
      setLinks(await telegramLinks());
      toast("أُلغي ربط الموظف");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const toggleBotAllowed = async (l: TelegramLink) => {
    const next = !l.bot_allowed;
    const ok = await confirm(
      next
        ? `السماح لـ ${l.name} باستخدام البوت؟`
        : `منع ${l.name} من استخدام البوت؟\n\nلن يستطيع استخدام الأزرار أو الأوامر، وستظهر له رسالة أن الحساب ممنوع. يمكنك إعادة السماح في أي وقت.`,
      {
        title: next ? "✅ السماح بالبوت" : "🚫 منع من استخدام البوت",
        yes: next ? "سماح" : "منع",
      },
    );
    if (!ok) return;
    try {
      await telegramSetBotAllowed(l.employee_id, next);
      setLinks(await telegramLinks());
      toast(next ? "تم السماح للموظف باستخدام البوت ✅" : "تم منع الموظف من استخدام البوت");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** نص التعميم بعد إزالة المقدمة — لمنع إرسال «تعميم» وحدها بلا مضمون. */
  const bcBody = bcText
    .trim()
    .replace(/^📣?\s*تعميم\s*/u, "")
    .trim();

  const openBroadcast = () => {
    if (!bcText.trim() || bcText.trim() === "📣 تعميم") setBcText(BC_PREFIX);
    setBcOpen(true);
  };

  const sendBroadcast = async () => {
    const txt = bcText.trim();
    if (!bcBody) {
      toast("اكتب نص التعميم بعد المقدمة", "err");
      return;
    }
    const who =
      bcTarget === "all"
        ? `كل المستخدمين المرتبطين (${linkedCount + supLinked})`
        : bcTarget.startsWith("emp:")
          ? `الموظف: ${links.find((l) => `emp:${l.employee_id}` === bcTarget)?.name ?? ""}`
          : `المشرف: ${supervisors.find((s) => `sup:${s.id}` === bcTarget)?.name ?? ""}`;
    const ok = await confirm(`إرسال هذا التعميم إلى ${who}؟\n\n${txt}`, {
      title: "📣 تأكيد إرسال تعميم",
      yes: "إرسال",
    });
    if (!ok) return;
    setBroadcasting(true);
    try {
      const [sent, failed] = await telegramBroadcast(txt, bcTarget);
      if (failed === 0) {
        toast(`أُرسل التعميم إلى ${sent} مستخدماً ✅`);
        setBcText(BC_PREFIX);
        setBcOpen(false);
      } else {
        toast(`أُرسل إلى ${sent}، وتعذّر الوصول إلى ${failed}`, "err");
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBroadcasting(false);
    }
  };

  const refreshNotes = async () => {
    setBusy(true);
    try {
      setNotes(await absenceNotesList(50));
      toast("حُدّثت أسباب الغياب");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div>
          <h1 className="text-2xl font-extrabold">بوت تلجرام 🤖</h1>
          <p className="text-sm text-slate-500 mt-1">
            الموظف يسأل عن دوامه وراتبه من تلجرام، والمشرف يصله كل التنبيهات —
            من دون أي خدمة خارجية أو اشتراك.
          </p>
        </div>
        <div className="flex items-center gap-2 text-sm flex-wrap">
          <button
            type="button"
            onClick={toggleBotRunning}
            className={`px-3.5 py-2 rounded-lg font-bold text-sm transition ${
              enabled && token
                ? "bg-rose-600 hover:bg-rose-700 text-white"
                : "bg-emerald-600 hover:bg-emerald-700 text-white"
            }`}
            title={
              enabled
                ? "إيقاف البوت فوراً وعدم الرد على أي رسائل"
                : "تفعيل البوت واستقبال الرسائل"
            }
          >
            {enabled && token ? "⏸️ إيقاف البوت" : "▶️ تشغيل البوت"}
          </button>
          <button
            type="button"
            className="btn-ghost border border-slate-200 bg-white hover:bg-slate-50 font-bold"
            onClick={openChatLog}
          >
            💬 سجل المحادثات
          </button>
          <button className="btn-primary" onClick={openBroadcast}>
            📣 إرسال تعميم
          </button>
          <span
            className={`badge ${
              enabled && token
                ? "bg-emerald-100 text-emerald-700"
                : "bg-slate-100 text-slate-500"
            }`}
          >
            {enabled && token ? "● البوت مفعّل" : "○ البوت متوقف"}
          </span>
        </div>
      </div>

      {/* ===== 1) بيانات البوت ===== */}
      <section className="card space-y-4">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <h2 className="font-extrabold text-lg">1) ربط البوت بالبرنامج</h2>
          {botName && (
            <span className="badge bg-indigo-100 text-indigo-700">
              متصل: {botName}
            </span>
          )}
        </div>

        <div>
          <label className="label">توكن البوت (Bot Token)</label>
          <div className="flex gap-2">
            <input
              className="input font-mono text-left"
              dir="ltr"
              type={showToken ? "text" : "password"}
              placeholder="123456789:AAH... (من @BotFather)"
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
            <button
              className="btn-ghost shrink-0"
              onClick={() => setShowToken((v) => !v)}
              type="button"
            >
              {showToken ? "🙈 إخفاء" : "👁️ إظهار"}
            </button>
          </div>
          <div className="text-xs text-slate-500 mt-1">
            تُنشئ البوت مرة واحدة من @BotFather ← /newbot ← انسخ التوكن والصقه
            هنا. التوكن سرّي — لا تشاركه مع أحد.
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-5">
          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="accent-indigo-600 w-4 h-4"
            />
            تفعيل البوت (استقبال الرسائل وإرسال التنبيهات)
          </label>
          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={alertStart}
              onChange={(e) => setAlertStart(e.target.checked)}
              className="accent-indigo-600 w-4 h-4"
            />
            تنبيه عدم تسجيل الدخول (بعد بداية الشفت بربع ساعة)
          </label>
          <label className="flex items-center gap-2 text-sm font-medium">
            <input
              type="checkbox"
              checked={alertEnd}
              onChange={(e) => setAlertEnd(e.target.checked)}
              className="accent-indigo-600 w-4 h-4"
            />
            تنبيه نهاية الشفت — الخروج المفقود (بعد نهاية الدوام بثلاث ساعات)
          </label>
        </div>

        <div className="text-xs text-slate-500 bg-slate-50 border border-slate-200 rounded-lg p-3">
          ⏰ يُحسب الوقت من شفت كل موظف في «الإعدادات ← الشفتات»: تنبيه الغياب
          بعد بداية شفته بربع ساعة، وتنبيه الخروج بعد نهاية ورديته بثلاث ساعات
          (وكذلك ملخص نهاية اليوم للمشرفين عند آخر وردية + 3 ساعات — بلا وقت
          ثابت). إن كان الجهاز مطفأً أو البرنامج غير عامل عند الموعد، يُرسل
          التنبيه فور عودة البرنامج في نفس اليوم.
        </div>

        <div className="flex flex-wrap gap-2">
          <button className="btn-primary" onClick={saveBot} disabled={savingBot}>
            {savingBot ? "… يحفظ" : "💾 حفظ إعدادات البوت"}
          </button>
          <button className="btn-ghost" onClick={test} disabled={testing || !token}>
            {testing ? "… يختبر" : "🔌 اختبار الاتصال"}
          </button>
        </div>
        <div className="text-xs text-slate-500 bg-slate-50 border border-slate-200 rounded-lg p-3">
          💡 بعد الحفظ والاختبار: أضف المشرفين في القسم التالي، ثم اطلب من كل
          مشرف وموظف فتح البوت والضغط على زر «📱 مشاركة رقم هاتفي» — لا نطلب
          كتابة الرقم حتى لا يُنتحل حساب أحد.
        </div>
      </section>

      {/* ===== 2) الصلاحيات ===== */}
      <section className="card space-y-3">
        <div>
          <h2 className="font-extrabold text-lg">2) ما المسموح للموظف؟</h2>
          <p className="text-sm text-slate-500">
            أطفئ أي خيار تريد منعه — سيظهر للموظف «هذا الخيار غير مسموح من
            الإدارة».
          </p>
        </div>
        <div className="divide-y divide-slate-100">
          {PERMS.map((p) => (
            <label
              key={p.key}
              className="flex items-start gap-3 py-3 cursor-pointer"
            >
              <input
                type="checkbox"
                checked={!!perms[p.key]}
                onChange={(e) => togglePerm(p.key, e.target.checked)}
                className="accent-indigo-600 w-4 h-4 mt-1"
              />
              <span className="flex-1">
                <span className="block text-sm font-bold">{p.label}</span>
                <span className="block text-xs text-slate-500 mt-0.5">
                  {p.hint}
                </span>
              </span>
            </label>
          ))}
        </div>
      </section>

      {/* ===== 3) المشرفون ===== */}
      <section className="card space-y-4">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div>
            <h2 className="font-extrabold text-lg">3) المشرفون (مستلمو التنبيهات)</h2>
            <p className="text-sm text-slate-500">
              يصلهم: التأخير، الخروج المفقود، أسباب الغياب، وملخص نهاية اليوم.
            </p>
          </div>
          <span className="badge bg-slate-100 text-slate-600">
            مرتبط: {supLinked} من {supervisors.length}
          </span>
        </div>

        <div className="grid md:grid-cols-[1fr_1fr_auto] gap-2 items-end">
          <div>
            <label className="label">اسم المشرف</label>
            <input
              className="input"
              placeholder="مثال: المدير العام"
              value={supName}
              onChange={(e) => setSupName(e.target.value)}
            />
          </div>
          <div>
            <label className="label">رقم الهاتف</label>
            <input
              className="input"
              dir="ltr"
              placeholder="0599123456"
              value={supPhone}
              onChange={(e) => setSupPhone(e.target.value)}
            />
          </div>
          <button className="btn-primary" onClick={addSup} disabled={addingSup}>
            {addingSup ? "…" : "➕ إضافة"}
          </button>
        </div>
        <p className="text-xs text-slate-500">
          لا يهم شكل الرقم: <bdi dir="ltr">0599123456</bdi> و
          <bdi dir="ltr">+970 599 123 456</bdi> و
          <bdi dir="ltr">+972 599 123 456</bdi> كلها تُطابق بعضها (آخر 9 أرقام).
        </p>

        {supervisors.length === 0 ? (
          <div className="text-sm text-slate-500 bg-slate-50 rounded-lg p-3">
            لا يوجد مشرفون بعد. أضف مشرفاً واحداً على الأقل لتصلك التنبيهات.
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr>
                  <th className="th">الاسم</th>
                  <th className="th">الهاتف</th>
                  <th className="th">الحالة</th>
                  <th className="th">ربط</th>
                  <th className="th"></th>
                </tr>
              </thead>
              <tbody>
                {supervisors.map((s) => (
                  <tr key={s.id}>
                    <td className="td font-medium">{s.name}</td>
                    <td className="td" dir="ltr">
                      {s.phone || "—"}
                    </td>
                    <td className="td">
                      {s.chat_id ? (
                        <span className="badge bg-emerald-100 text-emerald-700">
                          مرتبط
                        </span>
                      ) : (
                        <span className="badge bg-amber-100 text-amber-700">
                          بانتظار مشاركة الرقم
                        </span>
                      )}
                    </td>
                    <td className="td text-xs text-slate-500">
                      {dt(s.linked_at)}
                    </td>
                    <td className="td">
                      <div className="flex gap-2 justify-end">
                        {s.chat_id && (
                          <button
                            className="btn-ghost !px-2 !py-1 text-xs"
                            onClick={() => unlinkSup(s)}
                          >
                            إلغاء الربط
                          </button>
                        )}
                        <button
                          className="btn-danger !px-2 !py-1 text-xs"
                          onClick={() => delSup(s)}
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
        )}
      </section>

      {/* ===== 4) الموظفون المرتبطون ===== */}
      <section className="card space-y-4">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div>
            <h2 className="font-extrabold text-lg">4) الموظفون المرتبطون</h2>
            <p className="text-sm text-slate-500">
              يُربط الموظف تلقائياً حين يشارك رقم هاتفه مع البوت، بشرط أن يكون
              الرقم نفسه مسجّلاً في بطاقة الموظف. زر «منع» يوقفه عن استخدام
              البوت فقط، والقيمة نفسها تظهر في شاشة «الموظفون».
            </p>
          </div>
          <span className="badge bg-slate-100 text-slate-600">
            مرتبط: {linkedCount} من {links.length}
          </span>
        </div>

        <input
          className="input max-w-sm"
          placeholder="🔍 بحث بالاسم أو الهاتف"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />

        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr>
                <th className="th">الموظف</th>
                <th className="th">الهاتف</th>
                <th className="th">الحالة</th>
                <th className="th">البوت</th>
                <th className="th">تاريخ الربط</th>
                <th className="th"></th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((l) => (
                <tr key={l.employee_id}>
                  <td className="td font-medium">
                    {l.name}
                    {!l.active && (
                      <span className="badge bg-slate-100 text-slate-500 mr-2">
                        غير فعّال
                      </span>
                    )}
                  </td>
                  <td className="td" dir="ltr">
                    {l.phone || (
                      <span className="text-rose-600 text-xs">
                        لا يوجد رقم في البطاقة
                      </span>
                    )}
                  </td>
                  <td className="td">
                    {l.chat_id ? (
                      <span className="badge bg-emerald-100 text-emerald-700">
                        مرتبط
                      </span>
                    ) : (
                      <span className="badge bg-slate-100 text-slate-500">
                        غير مرتبط
                      </span>
                    )}
                  </td>
                  <td className="td">
                    <div className="flex items-center gap-2">
                      {l.bot_allowed ? (
                        <span className="badge bg-emerald-100 text-emerald-700">
                          مسموح
                        </span>
                      ) : (
                        <span className="badge bg-rose-100 text-rose-700">
                          ممنوع
                        </span>
                      )}
                      <button
                        className={`!px-2 !py-1 text-xs rounded-lg border font-bold ${
                          l.bot_allowed
                            ? "text-rose-700 border-rose-200 hover:bg-rose-50"
                            : "text-emerald-700 border-emerald-200 hover:bg-emerald-50"
                        }`}
                        onClick={() => toggleBotAllowed(l)}
                        title={
                          l.bot_allowed
                            ? "منع هذا الموظف من استخدام البوت"
                            : "السماح له باستخدام البوت"
                        }
                      >
                        {l.bot_allowed ? "🚫 منع" : "✅ سماح"}
                      </button>
                    </div>
                  </td>
                  <td className="td text-xs text-slate-500">
                    {dt(l.linked_at)}
                  </td>
                  <td className="td text-left">
                    {l.chat_id && (
                      <button
                        className="btn-ghost !px-2 !py-1 text-xs"
                        onClick={() => unlinkEmp(l)}
                      >
                        إلغاء الربط
                      </button>
                    )}
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr>
                  <td className="td text-slate-500" colSpan={6}>
                    لا نتائج.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>

      {/* ===== 5) أسباب الغياب ===== */}
      <section className="card space-y-3">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div>
            <h2 className="font-extrabold text-lg">5) أسباب الغياب المرسلة</h2>
            <p className="text-sm text-slate-500">
              تُسجَّل من البوت بالأمر /سبب، وتصل نسخة منها للمشرفين فوراً.
            </p>
          </div>
          <button className="btn-ghost" onClick={refreshNotes} disabled={busy}>
            {busy ? "… يحدّث" : "🔄 تحديث"}
          </button>
        </div>
        {notes.length === 0 ? (
          <div className="text-sm text-slate-500 bg-slate-50 rounded-lg p-3">
            لم يرسل أحد سبباً بعد.
          </div>
        ) : (
          <ul className="divide-y divide-slate-100 text-sm">
            {notes.map((n) => (
              <li key={n.id} className="py-2 flex items-start gap-3">
                <span className="text-slate-400 text-xs w-24 shrink-0" dir="ltr">
                  {n.date}
                </span>
                <span className="font-bold shrink-0">{n.employee_name}</span>
                <span className="flex-1 text-slate-600">{n.reason}</span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* ===== 6) الدليل ===== */}
      <section className="card space-y-4">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div>
            <h2 className="font-extrabold text-lg">6) 📘 دليل الاستخدام الكامل</h2>
            <p className="text-sm text-slate-500">
              خطوات مرقّمة من الصفر حتى أول تنبيه على تلجرام.
            </p>
          </div>
          <button
            className="btn-ghost"
            onClick={() => setGuideOpen((v) => !v)}
            aria-expanded={guideOpen}
          >
            {guideOpen ? "▲ إخفاء الدليل" : "▼ عرض الدليل"}
          </button>
        </div>

        {guideOpen && (
        <>
        <ol className="space-y-3 text-sm leading-7 list-none">
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>1) أنشئ البوت:</b> افتح تلجرام ← ابحث عن{" "}
            <bdi dir="ltr">@BotFather</bdi> ← أرسل <bdi dir="ltr">/newbot</bdi> ←
            اختر اسماً ثم اسم مستخدم ينتهي بـ <bdi dir="ltr">bot</bdi> ← سيصلك
            «Token» انسخه.
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>2) اربط البرنامج:</b> الصق التوكن في القسم 1 ← اضغط «حفظ إعدادات
            البوت» ← اضغط «اختبار الاتصال». يجب أن يظهر اسم البوت بجانب «متصل».
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>3) أضف المشرفين:</b> في القسم 3 اكتب اسم المشرف ورقمه (آخر 9
            أرقام هي المهمة — لا يهم إن كتبته بـ <bdi dir="ltr">970</bdi> أو{" "}
            <bdi dir="ltr">972</bdi> أو محلياً) ← «إضافة». إذا كان رقمه مسجّلاً
            كموظف أيضاً، تُضاف صلاحيات المشرف تلقائياً ويصله تحديث الأزرار
            برسالة فورية (بلا حاجة لإعادة مشاركة الرقم).
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>4) كل مشرف يربط حسابه:</b> يفتح رابط البوت الذي أنشأته ← يضغط{" "}
            <bdi dir="ltr">/start</bdi> ← يضغط زر «📱 مشاركة رقم هاتفي». تتغيّر
            حالته إلى «مرتبط»، ويبدأ بوصول التنبيهات فوراً، وتظهر له أزرار
            جاهزة أسفل المحادثة (/ملخص و/مساعدة).
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>5) أرقام الموظفين:</b> تأكد من شاشة «الموظفون» أن رقم هاتف كل
            موظف مسجّل في بطاقته، وأن الموظف «فعّال». ثم يفتح الموظف البوت ←
            <bdi dir="ltr"> /start</bdi> ← «📱 مشاركة رقم هاتفي» ← تظهر له أزرار
            جاهزة: دوام اليوم، كشف الشهر، شهر آخر، آخر راتب، رصيد الإجازات، طلب إجازة، طلب سلفة، سبب الغياب —
            يضغط بدل الكتابة.
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>6) اضبط الصلاحيات:</b> من القسم 2 اسمح للموظف بما تريد فقط. أي
            خيار مغلق يرد عليه البوت «غير مسموح من الإدارة».
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>7) منع موظف من البوت:</b> من القسم 4 (جدول الموظفين المرتبطين)
            اضغط «🚫 منع» أمام الموظف. لن يستطيع استخدام البوت، وتظهر له رسالة
            واضحة، ويبقى مرتبطاً ليستأنف فور «✅ سماح». نفس الخيار موجود في
            بطاقة الموظف بشاشة «الموظفون».
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>8) التنبيهات حسب الشفت:</b> لكل موظف حسب ورديته في «الإعدادات ←
            الشفتات» — تنبيه غياب بعد بداية دوامه بربع ساعة إن لم يسجّل حضوره،
            وتنبيه خروج مفقود بعد نهاية ورديته بثلاث ساعات إن لم يسجّل انصرافه،
            وملخص نهاية اليوم للمشرفين بعد آخر وردية بثلاث ساعات — مرة واحدة
            يومياً. إن كان الجهاز مطفأً أو البرنامج غير عامل عند الموعد، يُرسل
            التنبيه فور عودة البرنامج في نفس اليوم.
          </li>
          <li className="bg-slate-50 border border-slate-200 rounded-lg p-3">
            <b>9) التعميمات:</b> اضغط زر <b>«📣 إرسال تعميم»</b> أعلى الصفحة،
            اكتب الرسالة بعد المقدمة، واختر «الجميع» أو موظفاً/مشرفاً بعينه،
            ثم «إرسال». تصل فوراً لمن هو مرتبط ونشط فقط.
          </li>
        </ol>

        <div className="text-sm bg-indigo-50 border border-indigo-200 text-indigo-800 rounded-lg p-3">
          💡 لا حاجة لحفظ الأوامر: بعد ربط الحساب تظهر <b>أزرار جاهزة</b> أسفل
          المحادثة (دوام اليوم · كشف الشهر · شهر آخر · آخر راتب · رصيد الإجازات · طلب إجازة · طلب سلفة · سبب الغياب).
          زر «شهر آخر» يعرض الأشهر بالأرقام (مثل <bdi dir="ltr">10/2026</bdi>)
          للاختيار، وزر «سبب الغياب» يعرض أسباباً جاهزة. الكتابة اليدوية بالأوامر أدناه تبقى متاحة لمن يفضّلها.
        </div>

        <div className="grid md:grid-cols-2 gap-3 text-sm">
          <div className="border border-slate-200 rounded-lg overflow-hidden">
            <div className="bg-indigo-50 text-indigo-800 font-bold px-3 py-2">
              أوامر الموظف
            </div>
            <ul className="divide-y divide-slate-100">
              {[
                ["/اليوم", "هل ختمت دخولك وخروجك اليوم؟"],
                ["/كشف", "كشف دوام الشهر الحالي"],
                ["/كشف \u20662026-09\u2069", "كشف شهر محدد"],
                ["/راتب", "آخر راتب"],
                ["/رصيد", "رصيد الإجازات السنوية"],
                ["/اجازة", "طلب إجازة (سنوية / مرضية بمرفق / بدون راتب / طارئة)"],
                ["/سلفة", "طلب سلفة مالية"],
                ["/سبب ظرف عائلي", "تسجيل سبب الغياب"],
                ["/الغاء", "إلغاء ربط المحادثة"],
                ["/مساعدة", "قائمة الأوامر"],
              ].map(([c, d]) => (
                <li key={c} className="px-3 py-2 flex justify-between gap-2">
                  <span dir="rtl" className="font-mono text-xs text-indigo-700">
                    {c}
                  </span>
                  <span className="text-slate-600">{d}</span>
                </li>
              ))}
            </ul>
          </div>
          <div className="border border-slate-200 rounded-lg overflow-hidden">
            <div className="bg-emerald-50 text-emerald-800 font-bold px-3 py-2">
              أوامر المشرف
            </div>
            <ul className="divide-y divide-slate-100">
              {[
                ["/ملخص", "من حضر، من لم يسجّل خروجاً، ومن غاب"],
                ["/مساعدة", "قائمة الأوامر"],
                ["/الغاء", "إلغاء ربط المحادثة"],
              ].map(([c, d]) => (
                <li key={c} className="px-3 py-2 flex justify-between gap-2">
                  <span dir="rtl" className="font-mono text-xs text-emerald-700">
                    {c}
                  </span>
                  <span className="text-slate-600">{d}</span>
                </li>
              ))}
            </ul>
          </div>
        </div>

        <div className="text-xs text-slate-500 bg-amber-50 border border-amber-200 rounded-lg p-3 leading-6">
          ⚠️ ملاحظات مهمة:
          <br />• البوت يعمل فقط عندما يكون البرنامج مفتوحاً على جهاز الشركة.
          <br />• لا يعمل البوت من دون إنترنت، ولا يُرسل أي بيانات عند إيقافه من
          المفتاح أعلاه.
          <br />• إلغاء ربط الموظف لا يحذف بياناته — فقط يمنعه من استخدام
          البوت حتى يشارك رقمه مجدداً.
          <br />• إذا كان المشرف موظفاً أيضاً بنفس رقم الهاتف، تُربط له
          الصفتان معاً: يرى أزرار الموظف وزر «ملخص اليوم»، وتصله تنبيهات
          المشرفين. وإضافة المشرف بعد ربط الموظف تربط محادثته تلقائياً.
          <br />• «منع من استخدام البوت» يوقف الموظف عن البوت فقط ولا يؤثر
          على راتبه أو حضوره، ويُعكس فوراً في شاشة «الموظفون».
          <br />• إذا أوقفت موظفاً (جعله غير فعّال) من شاشة «الموظفون» يتوقف
          البوت عن الرد عليه فوراً برسالة «حسابك موقوف»، ويعود تلقائياً عند
          إعادة تفعيله — بلا حاجة لإعادة الربط.
          <br />• الرقم يُطابق بآخر 9 أرقام، لذا{" "}
          <bdi dir="ltr">+970 599 123 456</bdi> و
          <bdi dir="ltr">0599123456</bdi> يُعدّان نفس الرقم.
          <br />• يمكن استخدام نفس البوت لمؤسسة واحدة فقط (هذا البرنامج)، وكل
          إعداداته محفوظة محلياً في قاعدة بياناتك.
        </div>
        </>
        )}
      </section>

      {bcOpen && (
        <Modal
          onClose={() => !broadcasting && setBcOpen(false)}
          title="📣 إرسال تعميم"
          panelClass="max-w-lg"
          backdropClass="z-40"
        >
          <div className="space-y-3">
            <div>
              <label className="label">المستلم</label>
              <select
                className="input"
                value={bcTarget}
                onChange={(e) => setBcTarget(e.target.value)}
              >
                <option value="all">
                  👥 الجميع — {linkedCount} موظف + {supLinked} مشرف مرتبط
                </option>
                <optgroup label="موظف محدد">
                  {links
                    .filter((l) => l.chat_id && l.active && l.bot_allowed)
                    .map((l) => (
                      <option
                        key={`emp:${l.employee_id}`}
                        value={`emp:${l.employee_id}`}
                      >
                        {l.name}
                      </option>
                    ))}
                </optgroup>
                <optgroup label="مشرف محدد">
                  {supervisors
                    .filter((s) => s.chat_id && s.active)
                    .map((s) => (
                      <option key={`sup:${s.id}`} value={`sup:${s.id}`}>
                        {s.name}
                      </option>
                    ))}
                </optgroup>
              </select>
            </div>

            <div>
              <label className="label">نص التعميم</label>
              <textarea
                className="input min-h-36"
                placeholder="اكتب التعميم هنا…"
                value={bcText}
                maxLength={3500}
                onChange={(e) => setBcText(e.target.value)}
                autoFocus
              />
              <div className="flex justify-between text-xs text-slate-400 mt-1">
                <span>لا يُرسل لمن أوقف حسابه أو ألغى الربط.</span>
                <span dir="ltr">{bcText.length} / 3500</span>
              </div>
            </div>

            <div className="flex items-center gap-2 justify-end pt-1">
              <button
                className="btn-ghost"
                onClick={() => setBcOpen(false)}
                disabled={broadcasting}
              >
                إلغاء
              </button>
              <button
                className="btn-primary"
                onClick={sendBroadcast}
                disabled={broadcasting || !bcBody}
              >
                {broadcasting ? "… جارٍ الإرسال" : "إرسال التعميم"}
              </button>
            </div>
          </div>
        </Modal>
      )}

      {logOpen && (
        <ChatLogModal
          messages={chatMsgs}
          links={links}
          loading={logLoading}
          empId={logEmpId}
          setEmpId={setLogEmpId}
          search={logSearch}
          setSearch={setLogSearch}
          onRefresh={loadChatLog}
          onClose={() => setLogOpen(false)}
          previewImg={logPreviewImg}
          setPreviewImg={setLogPreviewImg}
        />
      )}
    </div>
  );
}

function ChatLogModal({
  messages,
  links,
  loading,
  empId,
  setEmpId,
  search,
  setSearch,
  onRefresh,
  onClose,
  previewImg,
  setPreviewImg,
}: {
  messages: TelegramChatMessage[];
  links: TelegramLink[];
  loading: boolean;
  empId: number | "all";
  setEmpId: (v: number | "all") => void;
  search: string;
  setSearch: (v: string) => void;
  onRefresh: () => void;
  onClose: () => void;
  previewImg: string | null;
  setPreviewImg: (v: string | null) => void;
}) {
  const q = search.trim().toLowerCase();

  // قائمة الموظفين الذين لديهم محادثات أو مرتبطين
  const contacts = useMemo(() => {
    const map = new Map<
      number,
      { id: number; name: string; phone: string; count: number; lastAt: string }
    >();
    for (const l of links) {
      if (l.chat_id) {
        map.set(l.employee_id, {
          id: l.employee_id,
          name: l.name,
          phone: l.phone,
          count: 0,
          lastAt: l.linked_at || "",
        });
      }
    }
    for (const m of messages) {
      if (m.employee_id != null) {
        const cur = map.get(m.employee_id);
        if (cur) {
          cur.count += 1;
          if (m.created_at > cur.lastAt) cur.lastAt = m.created_at;
        } else {
          map.set(m.employee_id, {
            id: m.employee_id,
            name: m.employee_name || `#${m.employee_id}`,
            phone: m.employee_phone || "",
            count: 1,
            lastAt: m.created_at,
          });
        }
      }
    }
    return Array.from(map.values())
      .filter(
        (c) =>
          !q ||
          c.name.toLowerCase().includes(q) ||
          c.phone.toLowerCase().includes(q) ||
          messages.some(
            (m) =>
              m.employee_id === c.id && m.body.toLowerCase().includes(q),
          ),
      )
      .sort((a, b) => b.lastAt.localeCompare(a.lastAt));
  }, [links, messages, q]);

  // الرسائل المعروضة (مرتبة زمنياً من الأقدم للأحدث داخل المحادثة، أو الأحدث أولاً عند عرض الكل)
  const filteredMsgs = useMemo(() => {
    const list = messages.filter((m) => {
      if (empId !== "all" && m.employee_id !== empId) return false;
      if (!q) return true;
      return (
        m.body.toLowerCase().includes(q) ||
        m.employee_name.toLowerCase().includes(q) ||
        m.employee_phone.toLowerCase().includes(q)
      );
    });
    return empId === "all" ? list : [...list].reverse();
  }, [messages, empId, q]);

  return (
    <Modal
      onClose={onClose}
      title="💬 سجل محادثات بوت تلجرام مع الموظفين"
      panelClass="max-w-5xl"
      backdropClass="z-40"
    >
      <div className="space-y-3">
        {/* شريط البحث والتحديث */}
        <div className="flex items-center gap-2 flex-wrap">
          <input
            className="input flex-1 min-w-[240px]"
            placeholder="🔍 ابحث باسم الموظف أو رقم الهاتف أو نص الرسالة…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            autoFocus
          />
          {search && (
            <button
              type="button"
              className="btn-ghost !py-1.5 text-xs"
              onClick={() => setSearch("")}
            >
              مسح البحث
            </button>
          )}
          <button
            type="button"
            className="btn-ghost !py-1.5 text-xs"
            onClick={onRefresh}
            disabled={loading}
          >
            {loading ? "… يحدّث" : "🔄 تحديث السجل"}
          </button>
        </div>

        <div className="grid md:grid-cols-[240px_1fr] gap-3 h-[62vh]">
          {/* قائمة الموظفين على الجانب */}
          <div className="border border-slate-200 rounded-xl overflow-y-auto bg-slate-50 p-2 space-y-1">
            <button
              type="button"
              onClick={() => setEmpId("all")}
              className={`w-full text-right px-3 py-2 rounded-lg text-xs font-bold flex items-center justify-between transition ${
                empId === "all"
                  ? "bg-indigo-600 text-white"
                  : "hover:bg-slate-200/70 text-slate-700"
              }`}
            >
              <span>👥 كل المحادثات</span>
              <span
                className={`px-1.5 py-0.5 rounded-full text-[10px] ${
                  empId === "all"
                    ? "bg-white/20 text-white"
                    : "bg-slate-200 text-slate-600"
                }`}
              >
                {messages.length}
              </span>
            </button>

            {contacts.map((c) => {
              const active = empId === c.id;
              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => setEmpId(c.id)}
                  className={`w-full text-right px-3 py-2 rounded-lg text-xs transition ${
                    active
                      ? "bg-indigo-600 text-white"
                      : "hover:bg-slate-200/70 text-slate-800"
                  }`}
                >
                  <div className="flex items-center justify-between gap-1">
                    <span className="font-bold truncate">{c.name}</span>
                    <span
                      className={`px-1.5 py-0.5 rounded-full text-[10px] font-bold ${
                        active
                          ? "bg-white/20 text-white"
                          : "bg-slate-200 text-slate-600"
                      }`}
                    >
                      {c.count}
                    </span>
                  </div>
                  {c.phone && (
                    <div
                      className={`text-[10px] font-mono mt-0.5 ${
                        active ? "text-indigo-100" : "text-slate-400"
                      }`}
                      dir="ltr"
                    >
                      {c.phone}
                    </div>
                  )}
                </button>
              );
            })}
          </div>

          {/* نافذة عرض الرسائل */}
          <div className="border border-slate-200 rounded-xl bg-white flex flex-col overflow-hidden">
            <div className="px-4 py-2.5 bg-slate-50 border-b border-slate-200 flex items-center justify-between text-xs">
              <span className="font-extrabold text-slate-700">
                {empId === "all"
                  ? "📋 أحدث الرسائل من جميع الموظفين"
                  : `💬 محادثة: ${
                      contacts.find((c) => c.id === empId)?.name ?? "الموظف"
                    }`}
              </span>
              <span className="text-slate-400">
                {filteredMsgs.length} رسالة
              </span>
            </div>

            <div className="flex-1 overflow-y-auto p-4 space-y-3 bg-slate-50/40">
              {filteredMsgs.length === 0 ? (
                <div className="h-full grid place-content-center text-center text-sm text-slate-400">
                  لا توجد رسائل مطابقة للعرض حالياً.
                </div>
              ) : (
                filteredMsgs.map((m) => {
                  const isIn = m.direction === "in";
                  return (
                    <div
                      key={m.id}
                      className={`flex flex-col ${
                        isIn ? "items-start" : "items-end"
                      }`}
                    >
                      <div
                        className={`max-w-[82%] rounded-2xl px-3.5 py-2.5 text-xs space-y-1.5 shadow-sm border ${
                          isIn
                            ? "bg-white border-slate-200 text-slate-800 rounded-tr-sm"
                            : "bg-indigo-600 border-indigo-600 text-white rounded-tl-sm"
                        }`}
                      >
                        <div className="flex items-center justify-between gap-3 text-[10px] opacity-80">
                          <span className="font-bold">
                            {isIn
                              ? `👤 ${m.employee_name}`
                              : `🤖 البوت ← ${m.employee_name}`}
                          </span>
                          <span className="font-mono" dir="ltr">
                            {m.created_at.slice(0, 16).replace("T", " ")}
                          </span>
                        </div>
                        <div className="whitespace-pre-wrap leading-relaxed">
                          {m.body}
                        </div>
                        {m.attachment_data && (
                          <img
                            src={m.attachment_data}
                            alt="مرفق"
                            className="max-h-40 rounded-lg border border-slate-200 cursor-pointer mt-1"
                            onClick={() => setPreviewImg(m.attachment_data)}
                          />
                        )}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>
      </div>

      {previewImg && (
        <Modal
          onClose={() => setPreviewImg(null)}
          title="📎 معاينة الصورة المرفقة"
          panelClass="max-w-3xl"
          backdropClass="z-50"
        >
          <div className="flex flex-col items-center gap-3">
            <img
              src={previewImg}
              alt="مرفق"
              className="max-h-[75vh] w-auto rounded-lg border border-slate-200"
            />
            <button
              type="button"
              className="btn-ghost self-end"
              onClick={() => setPreviewImg(null)}
            >
              إغلاق
            </button>
          </div>
        </Modal>
      )}
    </Modal>
  );
}
