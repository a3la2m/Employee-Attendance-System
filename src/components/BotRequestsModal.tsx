import { useEffect, useState } from "react";
import Modal from "./Modal";
import { useConfirm } from "./Confirm";
import {
  leaveRequestsList,
  decideLeaveRequest,
  advanceRequestsList,
  decideAdvanceRequest,
  getSettings,
} from "../lib/api";
import type { AdvanceRequest, LeaveRequest } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

export const LEAVE_TYPE_NAMES: Record<string, string> = {
  annual: "🏖️ إجازة سنوية مدفوعة",
  sick: "🏥 إجازة مرضية",
  unpaid: "📄 إجازة بدون راتب",
  emergency: "🚨 إجازة طارئة / ظرف عائلي",
  marriage: "💍 إجازة زواج",
  bereavement: "🕊️ إجازة وفاة (عزاء)",
  comp: "🎁 يوم بدل",
};

const curPeriod = (): string => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

export default function BotRequestsModal({
  onClose,
  toast,
  onChanged,
  onOpenEmployeeAdvance,
}: {
  onClose: () => void;
  toast: Toast;
  onChanged?: () => void;
  /** نقل المستخدم إلى صفحة الموظفين وفتح نافذة السلف للموظف مع تعبئة الطلب */
  onOpenEmployeeAdvance?: (req: AdvanceRequest) => void;
}) {
  const confirm = useConfirm();
  const [tab, setTab] = useState<"leaves" | "advances">("leaves");
  const [onlyPending, setOnlyPending] = useState(true);
  const [leaves, setLeaves] = useState<LeaveRequest[]>([]);
  const [advances, setAdvances] = useState<AdvanceRequest[]>([]);
  const [curSym, setCurSym] = useState("₪");
  const [busy, setBusy] = useState(false);

  // ملاحظات الإدارة وحالة الدفع لكل طلب إجازة قبل الاعتماد
  const [lvNotes, setLvNotes] = useState<Record<number, string>>({});
  const [lvPaid, setLvPaid] = useState<Record<number, boolean>>({});
  const [previewImg, setPreviewImg] = useState<{
    src: string;
    title: string;
  } | null>(null);

  // إعدادات اعتماد طلب السلفة (طريقة الخصم / التقسيط)
  const [advNotes, setAdvNotes] = useState<Record<number, string>>({});
  const [advMode, setAdvMode] = useState<Record<number, "single" | "installments">>({});
  const [advCount, setAdvCount] = useState<Record<number, number>>({});
  const [advStart, setAdvStart] = useState<Record<number, string>>({});

  const load = async () => {
    setBusy(true);
    try {
      const [lv, adv, s] = await Promise.all([
        leaveRequestsList(),
        advanceRequestsList(),
        getSettings(),
      ]);
      setLeaves(lv);
      setAdvances(adv);
      const c = s.currency || "ILS";
      setCurSym(c === "USD" ? "$" : c === "JOD" ? "د.أ" : "₪");
      const paidMap: Record<number, boolean> = {};
      for (const r of lv) paidMap[r.id] = r.paid;
      setLvPaid((prev) => ({ ...paidMap, ...prev }));
      // إذا لم تكن هناك إجازات معلقة وهناك سلف معلقة نفتح تبويب السلف تلقائياً
      if (
        lv.filter((x) => x.status === "pending").length === 0 &&
        adv.filter((x) => x.status === "pending").length > 0
      ) {
        setTab("advances");
      }
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pendingLeaves = leaves.filter((l) => l.status === "pending");
  const pendingAdvances = advances.filter((a) => a.status === "pending");
  const shownLeaves = onlyPending ? pendingLeaves : leaves;
  const shownAdvances = onlyPending ? pendingAdvances : advances;

  const handleLeaveDecision = async (r: LeaveRequest, approve: boolean) => {
    const paid = lvPaid[r.id] ?? r.paid;
    const note = (lvNotes[r.id] ?? "").trim();
    const label = LEAVE_TYPE_NAMES[r.leave_type] ?? r.leave_type;
    const ok = await confirm(
      approve
        ? `الموافقة على طلب ${label} للموظف «${r.employee_name}» (${r.from_date} ← ${r.to_date}، ${r.days_count} يوم — ${
            paid ? "مدفوعة" : "بدون راتب"
          })؟\nسيتم تسجيل الإجازة في النظام وإبلاغ الموظف على تلجرام.`
        : `رفض طلب ${label} للموظف «${r.employee_name}» وإبلاغه على تلجرام؟`,
      {
        title: approve ? "✅ تأكيد قبول الإجازة" : "❌ تأكيد رفض الإجازة",
        yes: approve ? "موافقة واعتماد" : "رفض الطلب",
      },
    );
    if (!ok) return;
    setBusy(true);
    try {
      await decideLeaveRequest(r.id, approve, paid, note);
      toast(
        approve
          ? "تمت الموافقة على الإجازة وتسجيلها في النظام وإشعار الموظف ✅"
          : "تم رفض طلب الإجازة وإشعار الموظف",
      );
      await load();
      onChanged?.();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const handleAdvanceDecision = async (r: AdvanceRequest, approve: boolean) => {
    const mode = advMode[r.id] ?? "single";
    const count = mode === "installments" ? Math.max(2, advCount[r.id] ?? 2) : 1;
    const start = advStart[r.id] || curPeriod();
    const note = (advNotes[r.id] ?? "").trim();
    const ok = await confirm(
      approve
        ? `الموافقة على سلفة للموظف «${r.employee_name}» بمبلغ ${r.amount} ${curSym} (${
            mode === "installments"
              ? `تقسيط على ${count} أشهر ابتداءً من ${start}`
              : `دفعة واحدة على راتب ${start}`
          })؟\nستُسجّل في سلف الموظف ويُبلغ عبر تلجرام.`
        : `رفض طلب السلفة للموظف «${r.employee_name}» بمبلغ ${r.amount} ${curSym}؟`,
      {
        title: approve ? "✅ تأكيد اعتماد السلفة" : "❌ تأكيد رفض السلفة",
        yes: approve ? "اعتماد السلفة" : "رفض الطلب",
      },
    );
    if (!ok) return;
    setBusy(true);
    try {
      await decideAdvanceRequest(
        r.id,
        approve,
        r.amount,
        mode,
        count,
        start,
        note,
      );
      toast(
        approve
          ? "تم اعتماد السلفة وتسجيل أقساطها وإشعار الموظف على تلجرام ✅"
          : "تم رفض طلب السلفة وإشعار الموظف",
      );
      await load();
      onChanged?.();
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      onClose={onClose}
      title="📥 طلبات الإجازات والسلف من البوت"
      panelClass="max-w-4xl"
      backdropClass="z-40"
    >
      <div className="space-y-4">
        {/* التبويبات والفلترة */}
        <div className="flex items-center justify-between gap-3 flex-wrap border-b border-slate-200 pb-3">
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => setTab("leaves")}
              className={`px-3.5 py-2 rounded-lg text-sm font-bold flex items-center gap-2 transition ${
                tab === "leaves"
                  ? "bg-indigo-600 text-white"
                  : "bg-slate-100 text-slate-700 hover:bg-slate-200"
              }`}
            >
              <span>🏖️ طلبات الإجازات</span>
              {pendingLeaves.length > 0 && (
                <span
                  className={`px-1.5 py-0.5 rounded-full text-xs font-extrabold ${
                    tab === "leaves"
                      ? "bg-white text-indigo-700"
                      : "bg-rose-500 text-white"
                  }`}
                >
                  {pendingLeaves.length}
                </span>
              )}
            </button>
            <button
              type="button"
              onClick={() => setTab("advances")}
              className={`px-3.5 py-2 rounded-lg text-sm font-bold flex items-center gap-2 transition ${
                tab === "advances"
                  ? "bg-indigo-600 text-white"
                  : "bg-slate-100 text-slate-700 hover:bg-slate-200"
              }`}
            >
              <span>💵 طلبات السلف</span>
              {pendingAdvances.length > 0 && (
                <span
                  className={`px-1.5 py-0.5 rounded-full text-xs font-extrabold ${
                    tab === "advances"
                      ? "bg-white text-indigo-700"
                      : "bg-rose-500 text-white"
                  }`}
                >
                  {pendingAdvances.length}
                </span>
              )}
            </button>
          </div>

          <div className="flex items-center gap-3 text-xs">
            <label className="flex items-center gap-1.5 cursor-pointer font-semibold text-slate-600">
              <input
                type="checkbox"
                checked={onlyPending}
                onChange={(e) => setOnlyPending(e.target.checked)}
                className="accent-indigo-600"
              />
              عرض الطلبات المعلقة فقط
            </label>
            <button
              type="button"
              className="btn-ghost !px-2.5 !py-1 text-xs"
              onClick={load}
              disabled={busy}
            >
              🔄 تحديث
            </button>
          </div>
        </div>

        {/* تبويب طلبات الإجازات */}
        {tab === "leaves" && (
          <div className="space-y-3 max-h-[65vh] overflow-y-auto pr-1">
            {shownLeaves.length === 0 ? (
              <div className="p-8 text-center text-sm text-slate-400 border border-dashed border-slate-200 rounded-xl">
                {onlyPending
                  ? "لا توجد طلبات إجازة قيد الانتظار حالياً ✓"
                  : "لا توجد طلبات إجازة مسجلة بعد."}
              </div>
            ) : (
              shownLeaves.map((r) => {
                const isPending = r.status === "pending";
                const paidVal = lvPaid[r.id] ?? r.paid;
                return (
                  <div
                    key={r.id}
                    className={`rounded-xl border p-4 space-y-3 ${
                      isPending
                        ? "border-indigo-200 bg-indigo-50/30"
                        : "border-slate-200 bg-white"
                    }`}
                  >
                    <div className="flex items-start justify-between gap-3 flex-wrap">
                      <div className="space-y-1">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="font-extrabold text-base">
                            {r.employee_name}
                          </span>
                          <span className="badge bg-indigo-100 text-indigo-800 font-bold">
                            {LEAVE_TYPE_NAMES[r.leave_type] ?? r.leave_type}
                          </span>
                          <span
                            className={`badge ${
                              paidVal
                                ? "bg-emerald-100 text-emerald-700"
                                : "bg-slate-200 text-slate-700"
                            }`}
                          >
                            {paidVal ? "مدفوعة" : "بدون راتب"}
                          </span>
                          <span className="text-xs text-slate-500">
                            (رصيده السنوي المتاح:{" "}
                            <b>{r.annual_leave_balance.toFixed(1)}</b> يوم)
                          </span>
                        </div>
                        <div className="text-sm text-slate-700 font-medium">
                          📅 الفترة:{" "}
                          <span className="font-mono font-bold" dir="ltr">
                            {r.from_date === r.to_date
                              ? r.from_date
                              : `${r.from_date} ← ${r.to_date}`}
                          </span>{" "}
                          ({r.days_count} يوم)
                        </div>
                        {r.note && (
                          <div className="text-xs text-slate-600 bg-white border border-slate-200 rounded-lg px-2.5 py-1.5">
                            📝 ملاحظة الموظف: {r.note}
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2">
                        {r.status === "pending" ? (
                          <span className="badge bg-amber-100 text-amber-800 font-bold">
                            ⏳ قيد المراجعة
                          </span>
                        ) : r.status === "approved" ? (
                          <span className="badge bg-emerald-100 text-emerald-700 font-bold">
                            ✅ مقبول
                          </span>
                        ) : (
                          <span className="badge bg-rose-100 text-rose-700 font-bold">
                            ❌ مرفوض
                          </span>
                        )}
                        <span
                          className="text-[11px] text-slate-400 font-mono"
                          dir="ltr"
                        >
                          {r.created_at.slice(0, 16).replace("T", " ")}
                        </span>
                      </div>
                    </div>

                    {/* المرفق (التقرير الطبي / الصورة) */}
                    {r.has_attachment && r.attachment_data && (
                      <div className="flex items-center gap-3 p-2.5 rounded-lg bg-white border border-slate-200">
                        <img
                          src={r.attachment_data}
                          alt="مرفق الإجازة"
                          className="w-16 h-16 object-cover rounded-lg border border-slate-200 cursor-pointer"
                          onClick={() =>
                            setPreviewImg({
                              src: r.attachment_data!,
                              title: `مرفق طلب إجازة — ${r.employee_name} (${r.from_date})`,
                            })
                          }
                        />
                        <div className="flex-1">
                          <div className="text-xs font-bold text-slate-700">
                            📎 مرفق صورة / تقرير طبي
                          </div>
                          <button
                            type="button"
                            className="text-xs text-indigo-600 hover:underline font-bold mt-1"
                            onClick={() =>
                              setPreviewImg({
                                src: r.attachment_data!,
                                title: `مرفق طلب إجازة — ${r.employee_name} (${r.from_date})`,
                              })
                            }
                          >
                            🔍 عرض الصورة بالحجم الكامل
                          </button>
                        </div>
                      </div>
                    )}

                    {/* أزرار القبول والرفض للطلبات المعلقة */}
                    {isPending ? (
                      <div className="pt-2 border-t border-indigo-100 flex flex-wrap items-end gap-3">
                        <label className="flex items-center gap-1.5 text-xs font-semibold text-slate-700 mb-2">
                          <input
                            type="checkbox"
                            checked={paidVal}
                            onChange={(e) =>
                              setLvPaid({
                                ...lvPaid,
                                [r.id]: e.target.checked,
                              })
                            }
                            className="accent-emerald-600"
                          />
                          إجازة مدفوعة الأجر
                        </label>
                        <div className="flex-1 min-w-[200px]">
                          <input
                            className="input !py-1.5 text-xs"
                            placeholder="ملاحظة للموظف عند القبول أو سبب الرفض (اختياري)…"
                            value={lvNotes[r.id] ?? ""}
                            onChange={(e) =>
                              setLvNotes({ ...lvNotes, [r.id]: e.target.value })
                            }
                          />
                        </div>
                        <button
                          type="button"
                          className="px-3.5 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-bold"
                          onClick={() => handleLeaveDecision(r, true)}
                          disabled={busy}
                        >
                          ✅ موافقة واعتماد
                        </button>
                        <button
                          type="button"
                          className="px-3.5 py-1.5 rounded-lg bg-rose-600 hover:bg-rose-700 text-white text-xs font-bold"
                          onClick={() => handleLeaveDecision(r, false)}
                          disabled={busy}
                        >
                          ❌ رفض
                        </button>
                      </div>
                    ) : (
                      r.admin_note && (
                        <div className="text-xs text-slate-500">
                          💬 رد الإدارة: {r.admin_note}
                        </div>
                      )
                    )}
                  </div>
                );
              })
            )}
          </div>
        )}

        {/* تبويب طلبات السلف */}
        {tab === "advances" && (
          <div className="space-y-3 max-h-[65vh] overflow-y-auto pr-1">
            {shownAdvances.length === 0 ? (
              <div className="p-8 text-center text-sm text-slate-400 border border-dashed border-slate-200 rounded-xl">
                {onlyPending
                  ? "لا توجد طلبات سلف قيد الانتظار حالياً ✓"
                  : "لا توجد طلبات سلف مسجلة بعد."}
              </div>
            ) : (
              shownAdvances.map((r) => {
                const isPending = r.status === "pending";
                const mode = advMode[r.id] ?? "single";
                const count = advCount[r.id] ?? 2;
                const start = advStart[r.id] ?? curPeriod();
                return (
                  <div
                    key={r.id}
                    className={`rounded-xl border p-4 space-y-3 ${
                      isPending
                        ? "border-amber-300 bg-amber-50/30"
                        : "border-slate-200 bg-white"
                    }`}
                  >
                    <div className="flex items-start justify-between gap-3 flex-wrap">
                      <div className="space-y-1">
                        <div className="flex items-center gap-2 flex-wrap">
                          <span className="font-extrabold text-base">
                            {r.employee_name}
                          </span>
                          <span className="badge bg-amber-100 text-amber-900 font-extrabold text-sm">
                            💵 {r.amount.toFixed(2)} {curSym}
                          </span>
                        </div>
                        {r.note && (
                          <div className="text-xs text-slate-600 bg-white border border-slate-200 rounded-lg px-2.5 py-1.5">
                            📝 ملاحظة الموظف: {r.note}
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2">
                        {isPending && onOpenEmployeeAdvance && (
                          <button
                            type="button"
                            className="px-2.5 py-1 rounded-lg bg-indigo-50 hover:bg-indigo-100 text-indigo-700 border border-indigo-200 text-xs font-bold"
                            onClick={() => onOpenEmployeeAdvance(r)}
                            title="فتح نافذة السلف الخاصة بالموظف في شاشة الموظفين مع تعبئة بيانات الطلب"
                          >
                            🔗 فتح في خانة السلف للموظف
                          </button>
                        )}
                        {r.status === "pending" ? (
                          <span className="badge bg-amber-100 text-amber-800 font-bold">
                            ⏳ قيد المراجعة
                          </span>
                        ) : r.status === "approved" ? (
                          <span className="badge bg-emerald-100 text-emerald-700 font-bold">
                            ✅ معتمد
                          </span>
                        ) : (
                          <span className="badge bg-rose-100 text-rose-700 font-bold">
                            ❌ مرفوض
                          </span>
                        )}
                        <span
                          className="text-[11px] text-slate-400 font-mono"
                          dir="ltr"
                        >
                          {r.created_at.slice(0, 16).replace("T", " ")}
                        </span>
                      </div>
                    </div>

                    {isPending ? (
                      <div className="pt-2 border-t border-amber-200/70 space-y-2.5">
                        <div className="grid sm:grid-cols-3 gap-2 text-xs">
                          <div>
                            <label className="label !text-xs">طريقة الخصم</label>
                            <select
                              className="input !py-1.5 text-xs"
                              value={mode}
                              onChange={(e) =>
                                setAdvMode({
                                  ...advMode,
                                  [r.id]: e.target.value as
                                    | "single"
                                    | "installments",
                                })
                              }
                            >
                              <option value="single">
                                دفعة واحدة على الراتب القادم
                              </option>
                              <option value="installments">
                                تقسيط على عدة أشهر
                              </option>
                            </select>
                          </div>
                          <div>
                            <label className="label !text-xs">
                              شهر بداية الخصم (YYYY-MM)
                            </label>
                            <input
                              type="month"
                              dir="ltr"
                              className="input !py-1.5 text-xs font-mono"
                              value={start}
                              onChange={(e) =>
                                setAdvStart({
                                  ...advStart,
                                  [r.id]: e.target.value,
                                })
                              }
                            />
                          </div>
                          {mode === "installments" && (
                            <div>
                              <label className="label !text-xs">
                                عدد الأقساط (الأشهر)
                              </label>
                              <input
                                type="number"
                                min={2}
                                max={60}
                                dir="ltr"
                                className="input !py-1.5 text-xs"
                                value={count}
                                onChange={(e) =>
                                  setAdvCount({
                                    ...advCount,
                                    [r.id]: Math.max(
                                      2,
                                      Number(e.target.value) || 2,
                                    ),
                                  })
                                }
                              />
                            </div>
                          )}
                        </div>

                        <div className="flex flex-wrap items-center gap-2">
                          <input
                            className="input !py-1.5 text-xs flex-1 min-w-[200px]"
                            placeholder="ملاحظة للموظف عند الاعتماد أو سبب الرفض (اختياري)…"
                            value={advNotes[r.id] ?? ""}
                            onChange={(e) =>
                              setAdvNotes({
                                ...advNotes,
                                [r.id]: e.target.value,
                              })
                            }
                          />
                          <button
                            type="button"
                            className="px-3.5 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-bold"
                            onClick={() => handleAdvanceDecision(r, true)}
                            disabled={busy}
                          >
                            ✅ موافقة واعتماد السلفة
                          </button>
                          <button
                            type="button"
                            className="px-3.5 py-1.5 rounded-lg bg-rose-600 hover:bg-rose-700 text-white text-xs font-bold"
                            onClick={() => handleAdvanceDecision(r, false)}
                            disabled={busy}
                          >
                            ❌ رفض
                          </button>
                        </div>
                      </div>
                    ) : (
                      r.admin_note && (
                        <div className="text-xs text-slate-500">
                          💬 رد الإدارة: {r.admin_note}
                        </div>
                      )
                    )}
                  </div>
                );
              })
            )}
          </div>
        )}
      </div>

      {/* نافذة معاينة المرفق بالحجم الكامل */}
      {previewImg && (
        <Modal
          onClose={() => setPreviewImg(null)}
          title={previewImg.title}
          panelClass="max-w-3xl"
          backdropClass="z-50"
        >
          <div className="flex flex-col items-center gap-3">
            <img
              src={previewImg.src}
              alt={previewImg.title}
              className="max-h-[75vh] w-auto rounded-lg border border-slate-200"
            />
            <div className="flex justify-end w-full">
              <button
                type="button"
                className="btn-ghost"
                onClick={() => setPreviewImg(null)}
              >
                إغلاق
              </button>
            </div>
          </div>
        </Modal>
      )}
    </Modal>
  );
}
