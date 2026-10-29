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
/* ورقة كشف الرواتب — تُطبع وحدها بزر «طباعة الكشف» */
.pay-sheet { display: none; }
.pay-slip { display: none; }
/* عرض القسيمة داخل نافذة المعاينة على الشاشة (تبقى مخفية في الطباعة العادية) */
.slip-overlay .pay-slip { display: block; }
/* ورقة طباعة حركات موظف (تُطبع من نافذة الحركات بزر «طباعة الحركات») */
.punches-sheet { display: none; }
/* تقسيم الحركات إلى صفحات A4 بـ 27 صفاً: كل صفحة تكسر قبل التالية */
.print-page { break-after: page; page-break-after: always; }
.print-page:last-child { break-after: auto; page-break-after: auto; }
/* صفوف مضغوطة (≈7مم) حتى تتسع 27 بالصفحة مع الترويسة والتذييل */
.punches-sheet .report-table th { padding: 5px 6px; font-size: 11.5px; }
.punches-sheet .report-table td { padding: 4px 6px; font-size: 12.5px; }
/* ورقة قسيمة دفعة مجمّدة — تُطبع وحدها بزر «طباعة كما كانت وقت الدفع» */
.frozen-sheet { display: none; }

/* قسيمة الراتب: نسخة مضغوطة تكفي صفحة واحدة (A4) */
.pay-slip .report-table { margin-top: 6px; }
.pay-slip .report-table th { padding: 4px; font-size: 11px; }
.pay-slip .report-table td { padding: 3px 4px; font-size: 11.5px; }
/* صف إجمالي فرعي (الراتب الأساسي المجمّع) */
.pay-slip .report-table .subtotal-row td {
  background: #f8fafc; border-top: 2px solid #475569; font-weight: 800;
}
/* جدول الأيام مقسوماً إلى نصفين جنباً إلى جنب: أعمدة ثابتة وخط أصغر */
.pay-slip .slip-days { table-layout: fixed; }
.pay-slip .slip-days th, .pay-slip .slip-days td { padding: 2px 3px; font-size: 10.5px; }
.pay-slip .slip-days th { text-align: center; }
.pay-slip .slip-days th:nth-child(2) { text-align: right; }
.pay-slip .slip-days td span { font-size: 10px; padding: 0 4px; white-space: nowrap; }
.pay-slip .slip-days td:nth-child(6) { white-space: nowrap; }
/* جدولان جانبيان في القسيمة: الاستحقاقات | الساعات والاستقطاعات */
.pay-slip .slip-sum { table-layout: fixed; page-break-inside: avoid; }
.pay-slip .slip-sum th { background: #eef2ff; font-size: 11px; font-weight: 800; }
.pay-slip .slip-sum td { padding: 2.5px 4px; font-size: 10.5px; }
.pay-slip .slip-sum td:first-child { text-align: right; }
.pay-slip .slip-sum td:last-child { text-align: center; white-space: nowrap; }
.pay-slip .slip-sum .group-row td {
  background: #f1f5f9; font-weight: 800; color: #334155;
}
.pay-slip .slip-sum .net-row td {
  background: #eef2ff; border-top: 2px solid #4f46e5;
  font-size: 13px; font-weight: 900; color: #3730a3;
}
/* شريط الصافي المستحق بعرض الورقة تحت الجدولين */
.pay-slip .slip-net {
  display: flex; align-items: center; justify-content: space-between;
  margin-top: 6px; padding: 5px 12px; border-radius: 6px;
  background: #eef2ff; border: 1px solid #c7d2fe; border-top: 3px solid #4f46e5;
  font-size: 13px; font-weight: 900; color: #3730a3;
}
.pay-slip .slip-net .amount { font-size: 16px; letter-spacing: 0.3px; }
/* كشف الرواتب المطبوع: أعمدة مضغوطة لتناسب عرض A4 */
.pay-sheet .pay-table th { padding: 5px 3px; font-size: 10px; text-align: center; }
.pay-sheet .pay-table th:nth-child(2) { text-align: right; }
.pay-sheet .pay-table td { padding: 4px 3px; font-size: 10.5px; }
.pay-sheet .pay-table .total-row td { font-size: 11px; }

@page { size: A4; margin: 12mm; }

@media print {
  body { background: #fff !important; }
  .print\\:hidden { display: none !important; }
  main { overflow: visible !important; }
  /* الشريط الجانبي لا يُطبع في أي مطبوعة */
  aside { display: none !important; }
  .report-sheet { box-shadow: none !important; max-width: none !important; padding: 0 !important; }
  .report-table thead { display: table-header-group; }
  /* وضع قسيمة الراتب: تُطبع القسيمة وحدها */
  body.print-slip .pay-main { display: none !important; }
  body.print-slip .pay-slip { display: block !important; }
  /* وضع طباعة حركات موظف: تُطبع الورقة وحدها */
  body.print-punches main { display: none !important; }
  body.print-punches .punches-sheet { display: block !important; }
  /* وضع طباعة الكشف: تُطبع ورقة الكشف وحدها (بترويسة وتذييل) */
  body.print-sheet main { display: none !important; }
  body.print-sheet .pay-sheet {
    display: flex !important; flex-direction: column; min-height: 272mm;
  }
  body.print-sheet .pay-sheet .pay-signs { margin-top: auto; }
  body.print-sheet .pay-sheet .print-footer { margin-top: 16px; }
  /* في القسيمة: التوقيعات والتذييل أسفل الصفحة */
  body.print-slip main .pay-slip {
    display: flex !important; flex-direction: column; min-height: 272mm;
  }
  body.print-slip main .pay-slip .slip-signs { margin-top: auto; }
  body.print-slip main .pay-slip .print-footer { margin-top: 12px; }
  /* وضع طباعة قسيمة دفعة مجمّدة: تُطبع الورقة وحدها (كما كانت وقت الدفع) */
  body.print-frozen main { display: none !important; }
  body.print-frozen .frozen-sheet { display: block !important; }
}
`;
