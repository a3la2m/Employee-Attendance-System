// نصوص موحّدة لحالات الأيام وأنواع الإجازات — تستخدمها الحضور والرواتب والتقارير
// حتى لا تختلف الصياغة بين الشاشات (خصوصاً بعد إضافة العطلة التعويضية «يوم بدل»).

/** اسم نوع الإجازة (من غير حالة الدفع). */
export const LEAVE_TYPE_LABEL: Record<string, string> = {
  annual: "إجازة سنوية",
  sick: "إجازة مرضية",
  comp: "عطلة تعويضية (يوم بدل)",
};

/** نوع إجازة بصيغة مختصرة لشارات الرواتب. */
export const LEAVE_SHORT_LABEL: Record<string, string> = {
  annual: "سنوية",
  sick: "مرضية",
  comp: "عطلة تعويضية",
};

/** نص الإجازة الكامل: النوع + (غير مدفوعة) عند اللزوم. */
export function leaveLabel(
  type: string | null | undefined,
  paid?: boolean | null,
): string {
  const base = LEAVE_TYPE_LABEL[type ?? ""] ?? "إجازة";
  return paid === false ? `${base} (غير مدفوعة)` : base;
}

/** حالة اليوم كما تُعرض في الرواتب/التقارير — نص + ألوان الشارة (Tailwind). */
export function dayStatusInfo(d: {
  status: string;
  leave_type?: string | null;
  leave_paid?: boolean | null;
  absent?: boolean;
  first_in?: string | null;
  last_out?: string | null;
}): [string, string] {
  if (d.status === "leave") {
    return [
      leaveLabel(d.leave_type, d.leave_paid),
      d.leave_type === "comp"
        ? "bg-fuchsia-100 text-fuchsia-700"
        : "bg-sky-100 text-sky-700",
    ];
  }
  if (d.status === "holiday") return ["عطلة رسمية", "bg-violet-100 text-violet-700"];
  if (d.status === "weekend") return ["عطلة أسبوعية", "bg-slate-200 text-slate-700"];
  if (d.absent || d.status === "absent") return ["غياب", "bg-rose-100 text-rose-700"];
  if (d.status === "incomplete") return ["بصمة ناقصة", "bg-amber-100 text-amber-700"];
  if (d.first_in) {
    return d.last_out
      ? ["حضور", "bg-emerald-100 text-emerald-700"]
      : ["حضور (قيد العمل)", "bg-emerald-100 text-emerald-700"];
  }
  return ["—", "text-slate-400"];
}

/** الحالة كنص فقط (للتقارير المطبوعة). */
export const dayStatusText = (d: Parameters<typeof dayStatusInfo>[0]) =>
  dayStatusInfo(d)[0];

/** هل هذا اليوم دوام فعلي في عطلة رسمية؟ (يُعوّض نقداً أو بيوم بدل) */
export function workedOnHoliday(d: {
  holiday?: boolean;
  status: string;
  absent?: boolean;
  first_in?: string | null;
}): boolean {
  return !!d.holiday && !d.absent && d.status !== "leave" && !!d.first_in;
}
