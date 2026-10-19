// أدوات الطباعة الموحّدة: نصوص الرأس/التذييل من الإعدادات + CSS مشترك (RTL/A4)
import type { Settings } from "./types";

/** نص رأس المستند من الإعدادات (فارغ = اسم الشركة). */
export const headerText = (cfg: Settings) =>
  (cfg.print_header || "").trim() || cfg.company_name || "اسم الشركة";

/** نص تذييل المستند من الإعدادات (فارغ = اسم الشركة). */
export const footerText = (cfg: Settings) =>
  (cfg.print_footer || "").trim() || cfg.company_name || "اسم الشركة";

/** إظهار الشعار في رأس المطبوعات (افتراضي: نعم). */
export const showLogo = (cfg: Settings) => (cfg.print_show_logo ?? "1") !== "0";

/** يوم عربي بالاسم والتاريخ بأرقام لاتينية: «الجمعة 16 أكتوبر 2026». */
export const arabicDay = (date: string) => {
  const d = new Date(`${date}T00:00:00`);
  if (Number.isNaN(d.getTime())) return date;
  return d.toLocaleDateString("ar-EG-u-nu-latn", {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
  });
};

/** HH:MM من طابع ISO أو — */
export const hhmm = (iso: string | null) =>
  iso ? (iso.split("T")[1] || "").slice(0, 5) : "—";

/** ساعات (رقم عشري واحد) من الدقائق. */
export const hrs = (min: number) => (min / 60).toFixed(1);

/** تاريخ اليوم بصيغة YYYY-MM-DD. */
export const todayISO = () => new Date().toISOString().slice(0, 10);

/** CSS الموحّد لكل المطبوعات: RTL + A4 + جداول التقارير + قواعد قسيمة الراتب. */
export const printCss = `
.report-sheet { direction: rtl; font-family: 'Tajawal', sans-serif; color: #0f172a; }
.report-table { width: 100%; border-collapse: collapse; margin-top: 8px; }
.report-table th {
  background: #f1f5f9; border: 1px solid #cbd5e1; padding: 8px 6px;
  font-size: 12px; text-align: right; font-weight: 700;
}
.report-table td { border: 1px solid #e2e8f0; padding: 7px 6px; font-size: 13px; text-align: center; }
.report-table td:nth-child(2) { text-align: right; }
.report-table .total-row td { background: #eef2ff; border-top: 2px solid #6366f1; }
.report-table tr { page-break-inside: avoid; }
.pay-slip { display: none; }
/* عرض القسيمة داخل نافذة المعاينة على الشاشة (تبقى مخفية في الطباعة العادية) */
.slip-overlay .pay-slip { display: block; }

@page { size: A4; margin: 12mm; }

@media print {
  body { background: #fff !important; }
  .print\\:hidden { display: none !important; }
  main { overflow: visible !important; }
  .report-sheet { box-shadow: none !important; max-width: none !important; padding: 0 !important; }
  .report-table thead { display: table-header-group; }
  /* وضع قسيمة الراتب: تُطبع القسيمة وحدها */
  body.print-slip .pay-main { display: none !important; }
  body.print-slip .pay-slip { display: block !important; }
}
`;
