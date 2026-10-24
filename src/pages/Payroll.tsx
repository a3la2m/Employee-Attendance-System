import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  getPayroll,
  archivePayroll,
  payrollMonths,
  employeePayrollHistory,
  recomputeDays,
  getSettings,
  saveSettingsBulk,
  getMonthDays,
  listEmployees,
  readLogoDataUri,
  salaryPayments,
  markSalaryPaid,
  unmarkSalaryPaid,
  savePayslipSnapshot,
  payslipSnapshots,
} from "../lib/api";
import type {
  EmployeePayrollRun,
  PayrollMonth,
  PayrollResult,
  AttendanceDay,
  Employee,
  Settings,
  PayslipSnapshot,
  SalaryPayment,
} from "../lib/types";
import { PrintHeader, PrintFooter } from "../components/PrintChrome";
import { printCss, hhmm, hrs, todayISO } from "../lib/print";
import { useToday, monthLocal } from "../lib/useToday";
import { dayStatusInfo } from "../lib/labels";
import {
  readSlipCfg,
  slipToSettings,
  splitSlip,
  slipPctSum,
  slipSumOk,
  holidayHoursOf,
  type SlipCfg,
} from "../lib/slipSplit";

type Toast = (m: string, t?: "ok" | "err") => void;

const n = (v: number) =>
  v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });

const AR_MONTHS = [
  "يناير", "فبراير", "مارس", "أبريل", "مايو", "يونيو",
  "يوليو", "أغسطس", "سبتمبر", "أكتوبر", "نوفمبر", "ديسمبر",
];

/** "2026-10" ← "أكتوبر 2026" (للعرض في التنقّل وسجل الموظف). */
const pLabel = (p: string) => {
  const [y, m] = p.split("-");
  return `${AR_MONTHS[Number(m) - 1] ?? m} ${y}`;
};

/** شارة الحصالة: نص + ألوان (موحّدة مع بقية الشاشات — راجع lib/labels). */
const stInfo = (d: AttendanceDay): [string, string] => dayStatusInfo(d);

/** الشهر الذي قبل شهر معيّن (YYYY-MM) — الافتراضي في نافذة سجل المدفوعات. */
const prevMonthOf = (m: string): string => {
  const [y, mo] = m.split("-").map(Number);
  const d = new Date(y, mo - 2, 1);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
};

/** نسخة مجمّدة من القسيمة وقت الطباعة/الدفع — تُحفظ JSON داخل اللقطة
 * (sheet) وتُعاد طباعتها لاحقاً بالضبط كما كانت وقتها. */
interface FrozenSheet {
  period: string;
  selName: string;
  selJob: string;
  pin: string;
  cfg: Settings;
  sel: PayrollResult;
  slipDays: AttendanceDay[];
  slip: SlipCfg;
}

export default function Payroll({ toast }: { toast: Toast }) {
  const { month, rolledFrom } = useToday();
  const [period, setPeriod] = useState(month);
  const [rows, setRows] = useState<PayrollResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [cfg, setCfg] = useState<Settings>({});
  const [logo, setLogo] = useState("");
  const [emps, setEmps] = useState<Employee[]>([]);
  const [selId, setSelId] = useState<number | "">("");
  const [days, setDays] = useState<AttendanceDay[]>([]);
  const [slipOpen, setSlipOpen] = useState(false);
  // الأشهر المؤرشفة + سجل رواتب موظف بعينه
  const [months, setMonths] = useState<PayrollMonth[]>([]);
  const [histOpen, setHistOpen] = useState(false);
  const [hist, setHist] = useState<EmployeePayrollRun[]>([]);
  const [histName, setHistName] = useState("");
  const [histEmpId, setHistEmpId] = useState<number | "">("");
  // إعدادات تقسيم القسيمة للطباعة — مسودة محلية تُحفظ في settings عند الحفظ
  const [slipDraft, setSlipDraft] = useState<SlipCfg | null>(null);
  // دفع الرواتب: مدفوعي الشهر الحالي + نافذة تعليم راتب موظف كمدفوع
  const [payments, setPayments] = useState<SalaryPayment[]>([]);
  const [payEmp, setPayEmp] = useState<PayrollResult | null>(null);
  const [payBusy, setPayBusy] = useState(false);
  // لقطات إعدادات طباعة القسيمة المحفوظة (لحظة الطباعة/الدفع) لكل موظف
  const [snaps, setSnaps] = useState<PayslipSnapshot[]>([]);
  // عرض المدفوعات التفصيلي (كل الشهور) مع ترشيح بالموظف والشهر
  const [paysOpen, setPaysOpen] = useState(false);
  const [allPays, setAllPays] = useState<SalaryPayment[]>([]);
  const [paysEmp, setPaysEmp] = useState<number | "">("");
  const [paysMonth, setPaysMonth] = useState(""); // "" = كل الأشهر
  // تأكيد داخل التطبيق (نعم/لا) — بديل نافذة النظام: سلوك مضمون (زر «لا»
  // يُلغي فعلاً). تُستخدم لكل تأكيدات الرواتب: السؤال بعد الطباعة، إلغاء
  // الدفع، وتعليم الشهر كاملاً.
  const [ask, setAsk] = useState<{
    title: string;
    body: ReactNode;
    yes: string;
    danger?: boolean;
    run: () => void | Promise<void>;
  } | null>(null);
  // تفصيل دفعة: اللقطة المجمّدة كما كانت وقت التعليم كمدفوع + طباعتها
  const [frozenRec, setFrozenRec] = useState<{
    pay: SalaryPayment;
    snap: PayslipSnapshot | null;
  } | null>(null);

  const cur = cfg.currency || "ILS";
  const curSym = cur === "USD" ? "$" : cur === "JOD" ? "د.أ" : "₪";
  const isArchived = months.some((m) => m.period === period);

  // عند تبدّل الشهر (التطبيق مفتوح ليلاً): رحّل الفترة المعروضة تلقائياً من
  // الشهر المنصرم إلى الشهر الجديد — بلا حاجة لإعادة التشغيل.
  useEffect(() => {
    if (rolledFrom && month && period === monthLocal(new Date(rolledFrom))) {
      setPeriod(month);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rolledFrom, month]);

  const loadMonths = () =>
    payrollMonths().then(setMonths).catch(() => {});

  // تنقّل بين الأشهر (لا تتجاوز الشهر الحالي)
  const shiftMonth = (delta: number) => {
    const [y, m] = period.split("-").map(Number);
    const d = new Date(y, m - 1 + delta, 1);
    const p = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
    if (p > month) return;
    setPeriod(p);
  };

  // فتح سجل رواتب موظف في كل الأشهر المؤرشفة
  const openHist = async (empId: number) => {
    const emp = emps.find((e) => e.id === empId);
    const inRows = rows.find((r) => r.employee_id === empId);
    setHistEmpId(empId);
    setHistName(emp?.name ?? inRows?.employee_name ?? `#${empId}`);
    setHist([]);
    setHistOpen(true);
    try {
      setHist(await employeePayrollHistory(empId));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const load = async () => {
    try {
      setRows(await getPayroll(period));
    } catch (e) {
      toast(String(e), "err");
    }
    // حالة الدفع لشهر العرض (مدفوع/غير مدفوع لكل موظف)
    salaryPayments(period).then(setPayments).catch(() => {});
  };

  useEffect(() => {
    load();
    loadMonths();
    getSettings()
      .then((s) => {
        setCfg(s);
        if (s.company_logo) {
          readLogoDataUri(s.company_logo).then(setLogo).catch(() => setLogo(""));
        }
      })
      .catch(() => {});
    listEmployees().then(setEmps).catch(() => {});
    payslipSnapshots(period).then(setSnaps).catch(() => {});
  }, [period]);

  // أيام دوام الموظف المحدّد (حضور/انصراف لكل يوم في الفترة)
  useEffect(() => {
    if (selId === "") {
      setDays([]);
      return;
    }
    getMonthDays(period)
      .then((all) =>
        setDays(
          all
            .filter((d) => d.employee_id === selId)
            .sort((a, b) => a.date.localeCompare(b.date)),
        ),
      )
      .catch((e) => toast(String(e), "err"));
  }, [selId, period]);

  const recalc = async () => {
    setBusy(true);
    const [y, m] = period.split("-").map(Number);
    const last = new Date(y, m, 0).getDate();
    try {
      await recomputeDays(`${period}-01`, `${period}-${String(last).padStart(2, "0")}`);
      await load();
      if (selId !== "") {
        const all = await getMonthDays(period);
        setDays(all.filter((d) => d.employee_id === selId));
      }
      toast("تم تحديث الرواتب من سجلات الحضور");
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  const archive = async () => {
    setBusy(true);
    try {
      const c = await archivePayroll(period);
      toast(`أُرشف راتب ${c} موظف لشهر ${period}`);
      loadMonths(); // يظهر الشهر فوراً في قائمة الأشهر المؤرشفة
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setBusy(false);
    }
  };

  /** فتح سجل المدفوعات: يجلب كل الدفعات من كل الشهور — والترشيح الافتراضي
   * على الشهر السابق للشهر الحالي (مدفوعات عادةً أُثرت سابقاً). */
  const openPays = async () => {
    setPaysOpen(true);
    setAllPays([]);
    setPaysEmp("");
    setPaysMonth(prevMonthOf(month));
    try {
      setAllPays(await salaryPayments());
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** إلغاء دفع من نافذة المدفوعات (لأي شهر) — بتأكيد داخل التطبيق. */
  const unpayRec = (p: SalaryPayment) => {
    const name = p.employee_name ?? `#${p.employee_id}`;
    setAsk({
      title: "↩ إلغاء الدفع",
      body: (
        <>
          إلغاء تعليم راتب <b>{name}</b> لشهر <b>{p.period}</b> كمدفوع؟ يعود
          الصافي إلى «غير مدفوع».
        </>
      ),
      yes: "نعم، إلغاء الدفع",
      danger: true,
      run: async () => {
        try {
          await unmarkSalaryPaid(p.employee_id, p.period);
          toast(
            `أُلغي تعليم راتب ${name} لشهر ${p.period} — عاد «غير مدفوع»`,
            "ok",
          );
          setAllPays(await salaryPayments());
          if (p.period === period) setPayments(await salaryPayments(period));
        } catch (e) {
          toast(String(e), "err");
        }
      },
    });
  };

  /** فتح تفصيل دفعة: يجلب اللقطة المحفوظة وقت التعليم كمدفوع (إعدادات +
   * أرقام + نسخة القسيمة المجمّدة) لعرضها وطباعتها كما كانت تماماً. */
  const openFrozen = async (p: SalaryPayment) => {
    setFrozenRec({ pay: p, snap: null });
    try {
      const list = await payslipSnapshots(p.period);
      const snap =
        list.find((s) => s.employee_id === p.employee_id) ?? null;
      setFrozenRec({ pay: p, snap });
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** طباعة ورقة الدفعة المجمّدة: تُطبع الورقة وحدها (body.print-frozen). */
  const printFrozen = () => {
    document.body.classList.add("print-frozen");
    try {
      window.print();
    } finally {
      document.body.classList.remove("print-frozen");
    }
  };

  // نسخة القسيمة المجمّدة من داخل اللقطة (إن وُجدت وقُدِّر قراءتها) —
  // إعادة إنتاج SlipSheet بنفس مكوّن الطباعة = طباعة مطابقة تماماً
  const fSheet: FrozenSheet | null = (() => {
    const raw = frozenRec?.snap?.sheet;
    if (!raw) return null;
    try {
      return JSON.parse(raw) as FrozenSheet;
    } catch {
      return null;
    }
  })();

  /** محتوى ورقة الدفعة المجمّدة: القسيمة كما كانت (أو ملخصها) + ختم الدفع. */
  const frozenNode = frozenRec ? (
    <>
      {fSheet ? (
        <SlipSheet
          cfg={fSheet.cfg}
          logo={logo}
          period={fSheet.period}
          selName={fSheet.selName}
          selJob={fSheet.selJob}
          pin={fSheet.pin}
          sel={fSheet.sel}
          slipDays={fSheet.slipDays}
          slip={fSheet.slip}
        />
      ) : frozenRec.snap ? (
        /* لقطة قديمة بلا نسخة مجمّدة: ملخص أرقامها وقت الدفع */
        <div className="text-sm">
          <div className="font-extrabold mb-2">ملخص القسيمة وقت الدفع</div>
          <table className="w-full text-sm">
            <tbody>
              <tr className="border-b border-slate-200">
                <td className="py-1">الأساسي (كما طُبع)</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.basic)}
                </td>
              </tr>
              <tr className="border-b border-slate-200">
                <td className="py-1">الإضافي</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.overtime)}
                </td>
              </tr>
              <tr className="border-b border-slate-200">
                <td className="py-1">بدل الإجازات والانتقالات</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.allowances)}
                </td>
              </tr>
              <tr className="border-b border-slate-200">
                <td className="py-1">أجر عطلة أسبوعية</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.weekend_pay)}
                </td>
              </tr>
              <tr className="border-b border-slate-200">
                <td className="py-1">عمل يوم عطلة</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.holiday_work)}
                </td>
              </tr>
              <tr className="border-b border-slate-200">
                <td className="py-1">بدل تفرّغ</td>
                <td className="py-1 text-left font-bold">
                  {n(frozenRec.snap.comp_off)}
                </td>
              </tr>
              {frozenRec.snap.enabled && (
                <>
                  <tr className="border-b border-slate-200">
                    <td className="py-1">حصة العمل</td>
                    <td className="py-1 text-left font-bold">
                      {n(frozenRec.snap.share_work)}
                    </td>
                  </tr>
                  <tr className="border-b border-slate-200">
                    <td className="py-1">حصة المخاطرة</td>
                    <td className="py-1 text-left font-bold">
                      {n(frozenRec.snap.share_risk)}
                    </td>
                  </tr>
                  <tr className="border-b border-slate-200">
                    <td className="py-1">حصة الحافز</td>
                    <td className="py-1 text-left font-bold">
                      {n(frozenRec.snap.share_bonus)}
                    </td>
                  </tr>
                </>
              )}
              <tr className="font-extrabold border-t-2 border-slate-300">
                <td className="pt-2">الصافي</td>
                <td className="pt-2 text-left">
                  {n(frozenRec.snap.net)} {curSym}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      ) : (
        <div className="text-sm text-slate-500">
          لا توجد لقطة محفوظة لهذه الدفعة (سُجّلت قبل تفعيل اللقطات) —
          تظهر شريط بيانات الدفع أسفل الصفحة.
        </div>
      )}
      {/* ختم: هذه النسخة المطبوعة هي نسخة مؤرشفة كما كانت وقت تعليم الدفع */}
      <div className="border-2 border-emerald-300 bg-emerald-50 rounded-lg p-3 mt-4 text-center">
        <span className="font-extrabold text-emerald-700 text-lg">
          نسخة مؤرشفة
        </span>
      </div>
    </>
  ) : null;

  // عرض الكشف: مفلتراً لموظف واحد إن رُشّح، وإلا للجميع
  const viewRows =
    selId === "" ? rows : rows.filter((r) => r.employee_id === selId);
  const sel = selId === "" ? null : (rows.find((r) => r.employee_id === selId) ?? null);
  const selEmp = selId === "" ? null : (emps.find((e) => e.id === selId) ?? null);
  const selName = sel?.employee_name ?? selEmp?.name ?? `#${selId}`;
  const selJob = selEmp?.job_title || "";

  const totalNet = viewRows.reduce((a, r) => a + r.net_pay, 0);
  const totalBase = viewRows.reduce((a, r) => a + r.base_pay, 0);
  const totalAllow = viewRows.reduce((a, r) => a + r.allowances, 0);
  const totalWeekend = viewRows.reduce((a, r) => a + r.weekend_pay, 0);

  // حالة الدفع: سجل دفع هذا الموظف في هذا الشهر (إن وُجد فهو «مدفوع ✓»)
  const paidOf = (empId: number) =>
    payments.find((p) => p.employee_id === empId) ?? null;
  const paidTotal = viewRows.reduce(
    (a, r) => (paidOf(r.employee_id) ? a + r.net_pay : a),
    0,
  );
  // آخر لقطة محفوظة لقسيمة الموظف المحدد (إعدادات الطباعة لحظة آخر طباعة/دفع)
  const lastSnap =
    selId === "" ? null : (snaps.find((s) => s.employee_id === selId) ?? null);

  // نافذة المدفوعات: الترشيحات والإجماليات
  const paysShown = allPays.filter(
    (p) =>
      (paysEmp === "" || p.employee_id === paysEmp) &&
      (paysMonth === "" || p.period === paysMonth),
  );
  const paysTotal = paysShown.reduce((a, p) => a + p.net, 0);
  // خيارات القائمة المنسدلة: كل شهور الدفعات + الشهر الافتراضي المختار
  // (حتى لو لا توجد دفعات فيه بعد — فيظهر «لا مدفوعات» بدل اختيار خاطئ)
  const paysMonths = [
    ...new Set([...allPays.map((p) => p.period), paysMonth]),
  ]
    .filter(Boolean)
    .sort()
    .reverse();

  // إعدادات التقسيم: المسودة غير المحفوظة إن وُجدت، وإلا المحفوظة في settings
  const slipCfg = slipDraft ?? readSlipCfg(cfg);

  /** تعديل رقمي في المسودة (حقل فارغ أو خاطئ → صفر). */
  const setSlipNum = (k: keyof SlipCfg, v: string) => {
    const x = Number(v);
    setSlipDraft({
      ...slipCfg,
      [k]: Number.isFinite(x) && x >= 0 ? x : 0,
    } as SlipCfg);
  };

  /** حفظ إعدادات التقسيم في settings — للطباعة فقط ولا تمس احتساب الرواتب. */
  const saveSlip = async () => {
    if (!slipSumOk(slipCfg)) {
      toast("مجموع نِسب البدلات الثلاثة يجب أن يساوي 100%", "err");
      return;
    }
    try {
      await saveSettingsBulk(slipToSettings(slipCfg));
      const s = await getSettings();
      setCfg(s);
      setSlipDraft(null);
      toast("حُفظت إعدادات الطباعة لقسيمة الراتب");
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** طباعة الورقة: slip=true لقسيمة الموظف وحدها، وإلا الكشف الكامل. */
  // أيام الدوام المطبوعة في القسيمة: الحضور الفعلي + الإجازات + العطل الرسمية
  // + العطل الأسبوعية (تُعرض كعطلة أسبوعية لا كغياب)
  const slipDays = days.filter(
    (d) =>
      d.first_in ||
      d.status === "leave" ||
      d.status === "holiday" ||
      d.status === "weekend",
  );

  // تقسيم القسيمة للطباعة (null إن كان ملغى أو النِسب ≠ 100%)
  const slipSplit = sel ? splitSlip(sel, days, slipCfg) : null;

  /** لقطة كاملة لحظة الطباعة أو لحظة الدفع: إعدادات الطباعة ونسبها وأرقامها
   * + نسخة JSON مجمّدة من القسيمة كلها (أيام الجدول + البنود + الإعدادات)
   * — تُطبع لاحقاً بالضبط كما كانت وقتها حتى لو تغيّرت الإعدادات أو سُجّل
   * حضور بعدها. إن كان التقسيم ملغياً وقتها تُحفظ اللقطة بـ enabled=false
   * (بنود الرواتب المحسوبة كما هي) فلا يحدث أي فرق عند إعادة الطباعة. */
  const snapFor = async (
    r: PayrollResult,
    empDays?: AttendanceDay[],
  ): Promise<PayslipSnapshot> => {
    const ds = empDays ?? (await empDaysFor(r.employee_id));
    const s = splitSlip(r, ds, slipCfg);
    // الجدول كما يُطبع: أيام الدخول/الخروج + الإجازات والعطلات (بلا أيام فارغة)
    const frozenDays = ds.filter(
      (d) =>
        d.first_in ||
        d.status === "leave" ||
        d.status === "holiday" ||
        d.status === "weekend",
    );
    const sheet: FrozenSheet = {
      period,
      selName: r.employee_name,
      selJob: emps.find((e) => e.id === r.employee_id)?.job_title || "",
      pin: r.pin ?? "",
      cfg,
      sel: r,
      slipDays: frozenDays,
      slip: slipCfg,
    };
    return {
      employee_id: r.employee_id,
      period,
      rate: slipCfg.rate,
      ot_mult: slipCfg.otMult,
      p_work: slipCfg.pWork,
      p_risk: slipCfg.pRisk,
      p_bonus: slipCfg.pBonus,
      // إن كان التقسيم مفعلاً: قيم التقسيم؛ وإن كان ملغياً: الأرقام كما
      // تُطبع في القسيمة العادية (الأساسي شامل الإضافي، بلا حصص)
      basic: s ? s.basic : r.base_pay,
      overtime: s ? s.ot : 0,
      weekend_pay: r.weekend_pay,
      allowances: r.allowances,
      holiday_work: r.holiday_work_pay ?? 0,
      comp_off: r.comp_off_pay ?? 0,
      share_work: s?.shareWork ?? 0,
      share_risk: s?.shareRisk ?? 0,
      share_bonus: s?.shareBonus ?? 0,
      net: r.net_pay,
      enabled: s !== null,
      sheet: JSON.stringify(sheet),
    };
  };

  /** أيام موظف بعينه لحساب لقطته: من الأيام المحمّلة إن كان محدّداً، وإلا تُجلب. */
  const empDaysFor = async (empId: number) =>
    empId === selId
      ? days
      : (await getMonthDays(period)).filter((d) => d.employee_id === empId);

  /** تعليم سريع كمدفوع بعد سؤال «نعم/لا» بعد الطباعة — بلا نافذة إضافية
   * (تُحفظ لقطة كاملة: إعدادات + أرقام + نسخة القسيمة المجمّدة). */
  const quickPay = async (r: PayrollResult) => {
    try {
      await markSalaryPaid(
        r.employee_id,
        period,
        r.net_pay,
        null,
        "تعليم بعد طباعة القسيمة",
      );
      // حفظ اللقطة الكاملة (إن طُبعت ورقة عادية بلا قسيمة — تُحفظ هنا)؛
      // فشلها لا يُلغي الدفع
      try {
        await savePayslipSnapshot(await snapFor(r));
      } catch {
        /* الدفع نجح — اللقطة فشلت */
      }
      toast(
        `عُلِّم راتب ${r.employee_name} لشهر ${period} كمدفوع ✓ — ${n(r.net_pay)} ${curSym}`,
        "ok",
      );
      setPayments(await salaryPayments(period));
    } catch (e) {
      toast(String(e), "err");
    }
  };

  /** طباعة الكشف أو القسيمة — ومع حفظ اللقطة الكاملة لحظة الطباعة نفسها
   * (تتضمن النسخة المجمّدة من القسيمة لإعادة إنتاجها حرفياً لاحقاً). */
  const printSheet = async (slip: boolean) => {
    if (slip && slipCfg.enabled && !slipSumOk(slipCfg)) {
      toast("لا يمكن طباعة القسيمة: نِسب البدلات الثلاثة لا تساوي 100%", "err");
      return;
    }
    // حفظ اللقطة أولاً — لا توقف الطباعة إن فشل الحفظ، لكن يُنبَّه المستخدم
    if (slip && sel) {
      try {
        await savePayslipSnapshot(await snapFor(sel, days));
      } catch (e) {
        toast(`تعذر حفظ لقطة القسيمة: ${String(e)}`, "err");
      }
    }
    if (slip) document.body.classList.add("print-slip");
    try {
      window.print();
    } finally {
      document.body.classList.remove("print-slip");
    }

    // بعد الطباعة مباشرة: نافذة نعم/لا داخل التطبيق — هل يُعلَّم الشهر كمدفوع؟
    // (لا تسأل إن كان مُعلَّماً كمدفوع مسبقاً — والتعليم يتم حصراً بضغط «نعم»)
    if (sel && !paidOf(sel.employee_id)) {
      const target = sel;
      setAsk({
        title: "💵 تعليم الشهر كمدفوع؟",
        body: (
          <>
            هل تريد تعليم شهر <b>{period}</b> لـ<b>{selName}</b> كمدفوع؟
            <div className="mt-2 text-xl font-extrabold text-emerald-700">
              {n(target.net_pay)} {curSym}
            </div>
            <div className="text-xs text-slate-400 mt-1">
              «نعم» = يُعلَّم فوراً كمدفوع بتاريخ اليوم · «لا» = يبقى غير مدفوع
            </div>
          </>
        ),
        yes: "نعم، تعليم كمدفوع ✓",
        run: () => quickPay(target),
      });
    }
  };

  /** تعليم راتب موظف كمدفوع: يحفظ المبلغ وطريقة الدفع وتاريخها، ولقطة
   * إعدادات الطباعة السارية وقتها إن كان التقسيم مفعلاً. */
  const doPay = async (r: PayrollResult, method: string, note: string) => {
    setPayBusy(true);
    try {
      await markSalaryPaid(
        r.employee_id,
        period,
        r.net_pay,
        method,
        note || null,
      );
      // لقطة كاملة لحظة الدفع: إعدادات وأرقام + نسخة القسيمة المجمّدة
      // (تُحفظ دائماً — حتى لو كان التقسيم ملغياً — فلا يختلف أي طباعة لاحقة)
      try {
        await savePayslipSnapshot(await snapFor(r));
      } catch {
        /* لقطة الدفع فشلت — الدفع نفسه نجح */
      }
      toast(
        `عُلِّم راتب ${r.employee_name} كمدفوع — ${n(r.net_pay)} ${curSym}`,
        "ok",
      );
      setPayEmp(null);
      setPayments(await salaryPayments(period));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setPayBusy(false);
    }
  };

  /** إلغاء تعليم راتب موظف كمدفوع (يعود الصافي إلى «غير مدفوع») — تأكيد داخلي. */
  const unpay = (r: PayrollResult) => {
    setAsk({
      title: "↩ إلغاء الدفع",
      body: (
        <>
          إلغاء تعليم راتب <b>{r.employee_name}</b> لشهر <b>{period}</b>{" "}
          كمدفوع؟ يعود الصافي إلى «غير مدفوع».
        </>
      ),
      yes: "نعم، إلغاء الدفع",
      danger: true,
      run: async () => {
        try {
          await unmarkSalaryPaid(r.employee_id, period);
          toast(`أُلغي تعليم راتب ${r.employee_name} — عاد «غير مدفوع»`, "ok");
          setPayments(await salaryPayments(period));
        } catch (e) {
          toast(String(e), "err");
        }
      },
    });
  };

  /** تنفيذ تعليم الشهر كاملاً كمدفوع (يُستدعى بعد تأكيد داخل التطبيق). */
  const payAllNow = async () => {
    const unpaid = viewRows.filter(
      (r) => !paidOf(r.employee_id) && r.net_pay !== 0,
    );
    if (unpaid.length === 0) return;
    const sum = unpaid.reduce((a, r) => a + r.net_pay, 0);
    setPayBusy(true);
    try {
      // أيام كل الموظفين مرة واحدة (لكل لقطة دفع كاملة — إعدادات + نسخة مجمّدة)
      const allDays = await getMonthDays(period);
      for (const r of unpaid) {
        await markSalaryPaid(
          r.employee_id,
          period,
          r.net_pay,
          null,
          "تعليم شهر كامل",
        );
        try {
          await savePayslipSnapshot(
            await snapFor(
              r,
              allDays.filter((d) => d.employee_id === r.employee_id),
            ),
          );
        } catch {
          /* الدفع نجح — اللقطة فشلت */
        }
      }
      toast(
        `عُلِّم ${unpaid.length} موظفاً كمدفوعي شهر ${period} (${n(sum)} ${curSym})`,
        "ok",
      );
      setPayments(await salaryPayments(period));
    } catch (e) {
      toast(String(e), "err");
    } finally {
      setPayBusy(false);
    }
  };

  /** تعليم شهر كامل كمدفوع: يسأل أولاً بتأكيد داخل التطبيق (نعم/لا). */
  const payAll = () => {
    const unpaid = viewRows.filter(
      (r) => !paidOf(r.employee_id) && r.net_pay !== 0,
    );
    if (unpaid.length === 0) {
      toast("كل الموظفين في الكشف مُعلَّمون كمدفوعين", "err");
      return;
    }
    const sum = unpaid.reduce((a, r) => a + r.net_pay, 0);
    setAsk({
      title: "💵 تعليم الشهر كمدفوعاً",
      body: (
        <>
          تعليم <b>{unpaid.length}</b> موظفاً بمجموع{" "}
          <b>
            {n(sum)} {curSym}
          </b>{" "}
          كمدفوعي شهر <b>{period}</b>؟
        </>
      ),
      yes: "نعم، تعليم الكل كمدفوع",
      run: payAllNow,
    });
  };

  return (
    <div className="p-6 space-y-5">
      <style>{printCss}</style>

      <div className="flex items-center justify-between print:hidden">
        <h1 className="text-2xl font-extrabold">كشف الرواتب</h1>
        <div className="flex gap-2 items-center flex-wrap">
          {/* تنقّل بين الأشهر: ‹ السابق | حقل الشهر | التالي › */}
          <div className="flex items-center gap-1">
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none"
              onClick={() => shiftMonth(-1)}
              title="الشهر السابق"
            >
              ‹
            </button>
            <input
              type="month"
              className="input !w-40"
              value={period}
              max={month}
              onChange={(e) => e.target.value && setPeriod(e.target.value)}
            />
            <button
              className="btn-ghost !px-2.5 !py-1.5 text-lg font-bold leading-none disabled:opacity-30"
              onClick={() => shiftMonth(1)}
              disabled={period >= month}
              title="الشهر التالي"
            >
              ›
            </button>
          </div>
          {period !== month && (
            <button className="btn-ghost" onClick={() => setPeriod(month)}>
              📅 هذا الشهر
            </button>
          )}
          {/* الأشهر المؤرشفة — اضغط للانتقال إليها */}
          <select
            className="input !w-60"
            value={isArchived ? period : ""}
            onChange={(e) => e.target.value && setPeriod(e.target.value)}
            title="الأشهر المؤرشفة — اختر واحداً لعرضه"
          >
            <option value="">🗃️ الأشهر المؤرشفة ({months.length})</option>
            {months.map((m) => (
              <option key={m.period} value={m.period}>
                {pLabel(m.period)} — {m.employees} موظف · {n(m.net_total)}{" "}
                {curSym}
              </option>
            ))}
          </select>
          {isArchived && (
            <span className="text-xs font-bold text-emerald-600 bg-emerald-50 px-2 py-1 rounded-full">
              ✅ مؤرشف
            </span>
          )}
          <button className="btn-ghost" onClick={recalc} disabled={busy}>
            🔄 تحديث
          </button>
          <button className="btn-ghost" onClick={archive} disabled={busy}>
            🗃️ أرشفة
          </button>
          <button
            className="btn-ghost"
            onClick={payAll}
            disabled={payBusy}
            title="تعليم كل الموظفين غير المدفوعين في الكشف كمدفوعي هذا الشهر"
          >
            💵 تعليم الشهر كمدفوعاً
          </button>
          <button
            className="btn-ghost"
            onClick={openPays}
            title="عرض كل المدفوعات (كل الشهور) مع ترشيح بالموظف والشهر"
          >
            📒 عرض المدفوعات
          </button>
          <button
            className="btn-primary"
            onClick={() => {
              if (rows.length === 0)
                return toast("لا بيانات رواتب — اضغط «تحديث» أولاً", "err");
              printSheet(false);
            }}
          >
            🖨️ طباعة الكشف
          </button>
        </div>
      </div>

      {/* اختيار موظف: عرض الأيام والقسيمة لشخص واحد */}
      <div className="card flex flex-wrap items-center gap-3 print:hidden">
        <span className="text-sm font-bold">📄 الموظف:</span>
        <select
          className="input !w-72"
          value={selId}
          onChange={(e) =>
            setSelId(e.target.value === "" ? "" : Number(e.target.value))
          }
        >
          <option value="">— كل الموظفين (كشف عام) —</option>
          {(emps.length > 0
            ? emps.map((e) => ({
                id: e.id,
                label: `${e.name}${e.pin ? ` (#${e.pin})` : ""}${e.active ? "" : " — موقوف"}`,
              }))
            : rows.map((r) => ({
                id: r.employee_id,
                label: `${r.employee_name}${r.pin ? ` (#${r.pin})` : ""}`,
              }))
          ).map((o) => (
            <option key={o.id} value={o.id}>
              {o.label}
            </option>
          ))}
        </select>
        {selId !== "" && (
          <button
            className="btn-primary"
            onClick={() => setSlipOpen(true)}
            title="عرض قسيمة الراتب على الشاشة قبل الطباعة"
          >
            👁️ عرض قسيمة الراتب
          </button>
        )}
        {selId !== "" && (
          <button
            className="btn-ghost"
            onClick={() => openHist(selId)}
            title="سجل رواتب هذا الموظف في كل الأشهر المؤرشفة"
          >
            📜 سجل الشهور
          </button>
        )}
        {selId !== "" && (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-slate-500">شهر القسيمة:</span>
            <input
              type="month"
              className="input !w-40"
              value={period}
              onChange={(e) => setPeriod(e.target.value)}
            />
          </div>
        )}
        <button
          className="btn-primary"
          onClick={() => {
            if (selId === "") return toast("اختر موظفاً أولاً لطباعة قسيمته", "err");
            printSheet(true);
          }}
          title="طباعة قسيمة الراتب للموظف المحدّد برأس المؤسسة وشعارها"
        >
          📄 طباعة قسيمة الراتب
        </button>
        {selId !== "" && (
          <button className="btn-ghost" onClick={() => setSelId("")}>
            ✕ عرض الكل
          </button>
        )}
      </div>

      {/* مجاميع */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4 pay-main">
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي الأساسي</div>
          <div className="text-xl font-extrabold">
            {n(totalBase)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">إجمالي البدلات</div>
          <div className="text-xl font-extrabold text-emerald-600">
            {n(totalAllow)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card">
          <div className="text-xs text-slate-500">
            إجمالي بدل العطلة والأيام المدفوعة
          </div>
          <div className="text-xl font-extrabold text-violet-600">
            {n(totalWeekend)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
        <div className="card border-indigo-200 bg-indigo-50">
          <div className="text-xs text-indigo-600">إجمالي الصافي</div>
          <div className="text-xl font-extrabold text-indigo-700">
            {n(totalNet)}{" "}
            <span className="text-base font-bold opacity-60">{curSym}</span>
          </div>
        </div>
      </div>

      {/* ===== إعدادات الطباعة لقسيمة الراتب (للطباعة فقط — لا تمس الحساب) ===== */}
      <div className="card space-y-4 print:hidden">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div className="font-extrabold text-lg">
              🖨️ إعدادات الطباعة لقسيمة الراتب
            </div>
            <div className="text-xs text-slate-500">
              تقسيم المبلغ المطبوع قانونياً بعد الحساب: الأساسي بسعر ساعة الحد
              الأدنى، والإضافي بمضاعفه، وباقي المبلغ على بدلات مسمّاة — لا
              يغيّر الرواتب المحسوبة والصافي.
            </div>
          </div>
          <label className="flex items-center gap-2 text-sm font-bold cursor-pointer">
            <input
              type="checkbox"
              className="accent-indigo-600 w-4 h-4"
              checked={slipCfg.enabled}
              onChange={(e) =>
                setSlipDraft({ ...slipCfg, enabled: e.target.checked })
              }
            />
            {slipCfg.enabled ? "مُفعَّل للطباعة" : "ملغى (كما هو)"}
          </label>
        </div>

        {slipCfg.enabled && (
          <>
            <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
              <label className="text-sm font-medium">
                سعر الساعة الأساسي
                <input
                  type="number"
                  className="input mt-1"
                  min={0}
                  step={0.25}
                  value={slipCfg.rate}
                  onChange={(e) => setSlipNum("rate", e.target.value)}
                />
              </label>
              <label className="text-sm font-medium">
                معامل ساعة الإضافي
                <input
                  type="number"
                  className="input mt-1"
                  min={0}
                  step={0.1}
                  value={slipCfg.otMult}
                  onChange={(e) => setSlipNum("otMult", e.target.value)}
                />
              </label>
              <label className="text-sm font-medium">
                بدل طبيعة العمل %
                <input
                  type="number"
                  className="input mt-1"
                  min={0}
                  max={100}
                  step={0.5}
                  value={slipCfg.pWork}
                  onChange={(e) => setSlipNum("pWork", e.target.value)}
                />
              </label>
              <label className="text-sm font-medium">
                بدل المخاطرة والسلامة المهنية %
                <input
                  type="number"
                  className="input mt-1"
                  min={0}
                  max={100}
                  step={0.5}
                  value={slipCfg.pRisk}
                  onChange={(e) => setSlipNum("pRisk", e.target.value)}
                />
              </label>
              <label className="text-sm font-medium">
                مكافأة الإنجاز / الإنتاجية %
                <input
                  type="number"
                  className="input mt-1"
                  min={0}
                  max={100}
                  step={0.5}
                  value={slipCfg.pBonus}
                  onChange={(e) => setSlipNum("pBonus", e.target.value)}
                />
              </label>
            </div>

            <div className="flex flex-wrap items-center justify-between gap-2">
              <span
                className={`px-3 py-1 rounded-full text-sm font-extrabold ${
                  slipSumOk(slipCfg)
                    ? "bg-emerald-100 text-emerald-700"
                    : "bg-red-100 text-red-700"
                }`}
              >
                مجموع النِسب: {slipPctSum(slipCfg)}%
                {slipSumOk(slipCfg) ? " ✓" : " — يجب أن يساوي 100%"}
              </span>
              <button
                className="btn-primary"
                disabled={!slipSumOk(slipCfg)}
                onClick={saveSlip}
              >
                💾 حفظ الإعدادات
              </button>
            </div>

            <div className="text-xs text-slate-500 leading-relaxed">
              الأساسي = ساعات الدوام × سعر الساعة · الإضافي = ساعات الإضافي ×
              المعامل × سعر الساعة · وباقي المبلغ المستحق يُقسَّم على البدلات
              الثلاثة بنسبة كل بدل. بدلات الفطور والمواصلات وبدل العطلة ودوام
              العطلة الرسمية والأيام التعويضية تُعرض كما هي، والصافي المستحق لا
              يتغيّر.
            </div>

            {sel && slipSplit && (
              <div className="rounded-lg border border-indigo-200 bg-indigo-50 p-3">
                <div className="text-xs font-bold text-indigo-700 mb-2">
                  معاينة تقسيم قسيمة {selName} — {period}
                </div>
                <div className="grid grid-cols-2 lg:grid-cols-4 gap-2 text-sm">
                  {[
                    ["الأساسي (المطبوع)", n(slipSplit.basic)],
                    ["الإضافي (المطبوع)", n(slipSplit.ot)],
                    ["يُقسَّم على البدلات", n(slipSplit.pool)],
                    ["الصافي (لا يتغير)", n(sel.net_pay)],
                  ].map(([k, v]) => (
                    <div
                      key={k}
                      className="rounded-lg bg-white border border-slate-200 p-2"
                    >
                      <div className="text-[11px] text-slate-500">{k}</div>
                      <div className="font-extrabold">
                        {v} {curSym}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {lastSnap && (
              <div className="text-xs text-slate-600 bg-slate-50 border border-slate-200 rounded-lg p-2.5 leading-relaxed">
                🕓 آخر قسيمة محفوظة لـ<b>{selName}</b> (لقطة إعدادات الطباعة
                السارية وقتها): {lastSnap.printed_at?.slice(0, 10) || "—"} — سعر
                الساعة {n(lastSnap.rate)}، معامل الإضافي {n(lastSnap.ot_mult)}،
                النِسب {lastSnap.p_work}/{lastSnap.p_risk}/{lastSnap.p_bonus}%،
                صافي {n(lastSnap.net)} {curSym}
                <span className="text-slate-400">
                  {" "}
                  — تُحفظ مع كل طباعة قسيمة وكل تعليم كمدفوع، فلا تتأثر بتغيير
                  الإعدادات لاحقاً.
                </span>
              </div>
            )}
          </>
        )}
      </div>

      {/* ===== بطاقة تفصيل الموظف: أيام الدوام بدخولها وخروجها ===== */}
      {selId !== "" && (
        <div className="card space-y-4 pay-main">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="font-extrabold text-lg">
                {selName}
                {selJob && (
                  <span className="text-sm font-normal text-slate-500">
                    {" "}
                    — {selJob}
                  </span>
                )}
              </div>
              <div className="text-xs text-slate-500">
                رقم البصمة: {sel?.pin || selEmp?.pin || "—"} · الفترة: {period}
              </div>
            </div>
            {sel && (
              <div className="text-left">
                <div className="text-xs text-slate-500">صافي الراتب</div>
                <div className="text-2xl font-extrabold text-indigo-700">
                  {n(sel.net_pay)}{" "}
                  <span className="text-base opacity-70">{curSym}</span>
                </div>
              </div>
            )}
          </div>

          {sel ? (
            <>
            <div className="grid grid-cols-2 lg:grid-cols-6 gap-3">
              {[
                ["أيام الحضور", String(sel.days_present), ""],
                ["ساعات فعلية", n(sel.total_worked_hours), ""],
                ["إضافي (ساعة)", n(sel.overtime_hours), ""],
                ["الأساسي", n(sel.base_pay), "text-slate-900"],
                ["البدلات", n(sel.allowances), "text-emerald-600"],
                [
                  "بدل العطلة الأسبوعية",
                  sel.weekend_pay ? n(sel.weekend_pay) : "—",
                  "text-violet-600",
                ],
              ].map(([label, val, cls]) => (
                <div key={label} className="rounded-lg border border-slate-200 p-3">
                  <div className="text-[11px] text-slate-500">{label}</div>
                  <div className={`font-extrabold ${cls}`}>
                    {val} {curSym}
                  </div>
                </div>
              ))}
            </div>
            {(sel.paid_leave_days > 0 ||
              sel.unpaid_leave_days > 0 ||
              sel.holiday_days > 0 ||
              (sel.comp_off_days ?? 0) > 0) && (
              <div className="text-xs text-slate-500">
                إجازات مدفوعة: <b>{sel.paid_leave_days}</b> يوم · غير مدفوعة:{" "}
                <b>{sel.unpaid_leave_days}</b> · إجازات رسمية:{" "}
                <b>{sel.holiday_days}</b>
                {(sel.comp_off_days ?? 0) > 0 && (
                  <>
                    {" "}
                    · أيام بدل تعويضية: <b>{sel.comp_off_days}</b>
                  </>
                )}
              </div>
            )}
            {(sel.holiday_work_days ?? 0) > 0 && (
              <div className="text-xs text-fuchsia-600">
                دوام في عطلة رسمية: <b>{sel.holiday_work_days}</b> يوم — أجر العطلة:{" "}
                <b>{n(sel.holiday_work_pay ?? 0)}</b>
              </div>
            )}
            {sel.weekend_days_paid > 0 && (
              <div className="text-xs text-slate-500">
                بدل العطلة الأسبوعية: <b>{n(sel.weekend_pay)}</b> عن{" "}
                <b>{sel.weekend_days_paid}</b> يوم عطلة · أيام الدوام المدفوعة:{" "}
                <b>{sel.paid_workdays}</b>
              </div>
            )}
            </>
          ) : (
            <div className="p-3 rounded-lg bg-amber-50 border border-amber-300 text-sm text-amber-800">
              لا يوجد كشف راتب لهذا الموظف في الفترة {period} (قد يكون موقوفاً أو
              أُضيف حديثاً) — أيام الدوام أدناه تُعرض من سجلات الحضور مباشرة.
            </div>
          )}

          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="bg-slate-100 text-xs">
                  <th className="th">#</th>
                  <th className="th">اليوم والتاريخ</th>
                  <th className="th">الحضور</th>
                  <th className="th">الانصراف</th>
                  <th className="th">الساعات</th>
                  <th className="th">الحالة</th>
                </tr>
              </thead>
              <tbody>
                {days.length === 0 && (
                  <tr>
                    <td
                      className="td text-center text-slate-400 py-6"
                      colSpan={6}
                    >
                      لا توجد أيام محسوبة لهذه الفترة — اضغط «🔄 تحديث»
                      لاحتسابها من سجلات الحضور.
                    </td>
                  </tr>
                )}
                {days.map((d, i) => {
                  const [label, cls] = stInfo(d);
                  return (
                    <tr key={d.id} className="hover:bg-slate-50">
                      <td className="td">{i + 1}</td>
                      <td className="td font-mono text-right">{d.date}</td>
                      <td className="td font-mono">{hhmm(d.first_in)}</td>
                      <td className="td font-mono">
                        {hhmm(d.last_out)}
                        {d.last_out && d.last_out.slice(0, 10) !== d.date && (
                          <div className="text-[10px] text-slate-400">
                            {d.last_out.slice(0, 10)}
                          </div>
                        )}
                      </td>
                      <td className="td">{hrs(d.worked_min)}</td>
                      <td className="td">
                        <span
                          className={`px-2 py-0.5 rounded-full text-xs font-bold ${cls}`}
                        >
                          {label}
                        </span>
                        {d.overtime_min > 0 && (
                          <span className="text-[10px] text-indigo-500 mr-1">
                            +{d.overtime_min} د إضافي
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      <div className="card !p-0 overflow-hidden pay-main">
        <table className="w-full">
          <thead>
            <tr>
              <th className="th">الموظف</th>
              <th className="th">أيام الحضور</th>
              <th className="th">ساعات فعلية</th>
              <th className="th">إضافي</th>
              <th className="th">الأساسي ({curSym})</th>
              <th className="th">البدلات ({curSym})</th>
              <th className="th">بدل العطلة ({curSym})</th>
              <th className="th">الصافي ({curSym})</th>
              <th className="th">الحالة</th>
            </tr>
          </thead>
          <tbody>
            {viewRows.length === 0 && (
              <tr>
                <td className="td text-center text-slate-400 py-8" colSpan={9}>
                  {selId !== ""
                    ? "لا راتب محسوب لهذا الموظف في هذه الفترة."
                    : "لا بيانات رواتب — اضغط «تحديث» بعد سحب الحضور."}
                </td>
              </tr>
            )}
            {viewRows.map((r) => {
              const p = paidOf(r.employee_id);
              return (
                <tr key={r.employee_id} className="hover:bg-slate-50">
                  <td className="td font-medium">
                    <button
                      className="hover:text-indigo-600 underline decoration-dotted underline-offset-4"
                      onClick={() => setSelId(r.employee_id)}
                      title="عرض أيام الدوام وقسيمة هذا الموظف"
                    >
                      {r.employee_name}
                    </button>
                    <span className="text-slate-400 font-mono text-xs mr-1">
                      {r.pin ? `#${r.pin}` : ""}
                    </span>
                    <button
                      className="text-xs text-slate-400 hover:text-indigo-600 mr-1"
                      onClick={() => openHist(r.employee_id)}
                      title="سجل رواتب هذا الموظف في كل الأشهر المؤرشفة"
                    >
                      📜
                    </button>
                  </td>
                  <td className="td">{r.days_present}</td>
                  <td className="td">{n(r.total_worked_hours)}</td>
                  <td className="td">{n(r.overtime_hours)}</td>
                  <td className="td">{n(r.base_pay)}</td>
                  <td className="td text-emerald-600">{n(r.allowances)}</td>
                  <td className="td text-violet-600">
                    {r.weekend_days_paid
                      ? `${n(r.weekend_pay)} (${r.weekend_days_paid} يوم عطلة)`
                      : "—"}
                  </td>
                  <td className="td font-extrabold">{n(r.net_pay)}</td>
                  <td className="td">
                    <div className="flex items-center gap-1 justify-center">
                      {p ? (
                        <>
                          <span
                            className="badge bg-emerald-100 text-emerald-700 text-xs"
                            title={`دُفع بتاريخ ${p.paid_at.slice(0, 10)}${
                              p.method ? ` · ${p.method}` : ""
                            }${p.note ? ` · ${p.note}` : ""} — ${n(p.net)} ${curSym}`}
                          >
                            مدفوع ✓
                          </span>
                          <button
                            className="text-xs text-slate-400 hover:text-rose-600"
                            title="إلغاء تعليم الدفع (يعود «غير مدفوع»)"
                            onClick={() => unpay(r)}
                          >
                            ↩
                          </button>
                        </>
                      ) : (
                        <>
                          <span className="badge bg-rose-100 text-rose-600 text-xs">
                            غير مدفوع
                          </span>
                          <button
                            className="text-xs text-slate-400 hover:text-emerald-600"
                            title="تعليم كمدفوع بتاريخ اليوم"
                            onClick={() => setPayEmp(r)}
                          >
                            💵
                          </button>
                        </>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
            {viewRows.length > 0 && (
              <tr className="bg-slate-50 font-extrabold">
                <td className="td" colSpan={7}>
                  {selId === "" ? "الإجمالي العام" : "إجمالي الموظف"}
                </td>
                <td className="td text-indigo-700">{n(totalNet)}</td>
                <td className="td text-xs font-bold">
                  <span className="text-emerald-700">مدفوع {n(paidTotal)}</span>
                  <span className="text-slate-400"> · </span>
                  <span className="text-rose-600">
                    متبقٍّ {n(totalNet - paidTotal)}
                  </span>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <p className="text-xs text-slate-500 leading-relaxed print:hidden">
        المعادلات: العمل الفعلي = (الانصراف − الحضور) − الاستراحة · الأساسي = الساعات الفعلية ×
        سعر الساعة · البدلات = (فطور + مواصلات) × أيام الحضور · الإضافي يبدأ بعد تجاوز الساعات
        الفعلية (بعد الاستراحة) لساعات الدوام الأساسية (8.5 − 0.5 = 8 ساعات مثلاً) ويُدفع بمضاعف ·
        بدل العطلة (الطريقة الأسبوعية الافتراضية): الأسبوع من السبت إلى الجمعة والجمعة عطلة
        أسبوعية مدفوعة تُسجَّل تلقائياً — مبلغ الأسبوع (62) × (أيام الدوام المستحق من السبت–الخميس
        ÷ أيام الدوام المتاحة بالأسبوع)، فالغياب يُخصم نصيبه (62 ÷ 6) ودوام الجمعة يُضاف عادياً؛
        وإن اخترت «لكل يوم عطلة» من الإعدادات يُدفع 62 عن كل يوم عطلة مستحق · الإجازة المدفوعة/
        غير المدفوعة تُحدَّد عند تسجيلها من بطاقة الموظف · التأخير والانصراف المبكر معلومة في
        الإشعارات فقط ولا يؤثران على الرواتب ·
        الصافي = الأساسي + البدلات + بدل العطلة (لا خصومات إطلاقاً).
      </p>

      {/* ===== سجل رواتب موظف بعينه في الأشهر المؤرشفة ===== */}
      {histOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-50 overflow-y-auto p-4 print:hidden"
          onClick={() => setHistOpen(false)}
        >
          <div
            className="card max-w-5xl mx-auto mt-10"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between mb-4">
              <h2 className="font-bold text-lg">
                📜 سجل الرواتب — {histName}
                <span className="text-sm font-normal text-slate-500 mr-2">
                  (الأشهر المؤرشفة — الأحدث أولاً)
                </span>
              </h2>
              <button
                className="text-slate-400 hover:text-slate-700 text-sm"
                onClick={() => setHistOpen(false)}
                aria-label="إغلاق سجل الرواتب"
                title="إغلاق (Esc)"
              >
                ✕
              </button>
            </div>
            {hist.length === 0 ? (
              <p className="text-sm text-slate-400 py-8 text-center">
                لا يوجد أرشيف رواتب لهذا الموظف بعد — افتح أي شهر من أعلى
                الصفحة واضغط «🗃️ أرشفة» ليُحفظ في هذا السجل.
              </p>
            ) : (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-slate-100 text-xs">
                      <th className="th">الشهر</th>
                      <th className="th">أيام الحضور</th>
                      <th className="th">ساعات فعلية</th>
                      <th className="th">إضافي</th>
                      <th className="th">الأساسي ({curSym})</th>
                      <th className="th">البدلات ({curSym})</th>
                      <th className="th">بدل العطلة ({curSym})</th>
                      <th className="th">الصافي ({curSym})</th>
                      <th className="th"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {hist.map((h) => (
                      <tr key={h.period} className="hover:bg-slate-50">
                        <td className="td font-bold">{pLabel(h.period)}</td>
                        <td className="td">{h.days_present}</td>
                        <td className="td">{n(h.total_worked_hours)}</td>
                        <td className="td">{n(h.overtime_hours)}</td>
                        <td className="td">{n(h.base_pay)}</td>
                        <td className="td text-emerald-600">
                          {n(h.allowances)}
                        </td>
                        <td className="td text-violet-600">
                          {n(h.weekend_pay)}
                        </td>
                        <td className="td font-extrabold">{n(h.net_pay)}</td>
                        <td className="td">
                          <button
                            className="text-xs text-indigo-600 hover:underline font-bold"
                            onClick={() => {
                              setPeriod(h.period);
                              setSelId(histEmpId);
                              setHistOpen(false);
                            }}
                            title="عرض كشف هذا الشهر للموظف"
                          >
                            عرض ↩
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        </div>
      )}

      {/* ===== قسيمة الراتب (نسخة الطباعة — مخفية على الشاشة) ===== */}
      {selId !== "" && (
        <div className="report-sheet pay-slip p-8">
          <SlipSheet
            cfg={cfg}
            logo={logo}
            period={period}
            selName={selName}
            selJob={selJob}
            pin={sel?.pin || selEmp?.pin || "—"}
            sel={sel}
            slipDays={slipDays}
            slip={slipCfg}
          />
        </div>
      )}

      {/* ===== عرض قسيمة الراتب على الشاشة ===== */}
      {slipOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-50 overflow-y-auto p-4 print:hidden"
          onClick={() => setSlipOpen(false)}
        >
          <div
            className="max-w-5xl mx-auto my-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2 mb-2">
              <div className="flex items-center gap-2">
                <span className="text-sm font-bold text-white">شهر القسيمة:</span>
                <input
                  type="month"
                  className="input !w-40 bg-white"
                  value={period}
                  onChange={(e) => setPeriod(e.target.value)}
                />
              </div>
              <div className="flex gap-2">
                <button className="btn-primary" onClick={() => printSheet(true)}>
                  🖨️ طباعة
                </button>
                <button
                  className="btn-ghost bg-white"
                  onClick={() => setSlipOpen(false)}
                  aria-label="إغلاق عرض كشف الراتب"
                >
                  ✕ إغلاق
                </button>
              </div>
            </div>
            <div className="slip-overlay">
              <div className="report-sheet pay-slip p-8 bg-white shadow-2xl rounded-xl">
                <SlipSheet
                  cfg={cfg}
                  logo={logo}
                  period={period}
                  selName={selName}
                  selJob={selJob}
                  pin={sel?.pin || selEmp?.pin || "—"}
                  sel={sel}
                  slipDays={slipDays}
                  slip={slipCfg}
                />
              </div>
            </div>
          </div>
        </div>
      )}

      {/* نافذة عرض سجل المدفوعات (كل الشهور + ترشيح بالموظف والشهر) */}
      {paysOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-50 overflow-y-auto p-4 print:hidden"
          onClick={() => setPaysOpen(false)}
        >
          <div
            className="card max-w-5xl mx-auto mt-8"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-3 flex-wrap mb-4">
              <h2 className="font-bold text-lg">
                📒 سجل المدفوعات
                <span className="text-sm font-normal text-slate-500 mr-2">
                  (كل الشهور — الافتراضي: الشهر السابق · انقر أي صف لعرض
                  القسيمة كما كانت وقت الدفع وطباعتها)
                </span>
              </h2>
              <button
                className="text-slate-400 hover:text-slate-700 text-sm"
                onClick={() => setPaysOpen(false)}
                aria-label="إغلاق سجل المدفوعات"
                title="إغلاق (Esc)"
              >
                ✕
              </button>
            </div>

            {/* الترشيحات: الموظف + الشهر + الإجمالي المفلتر */}
            <div className="flex flex-wrap items-end gap-3 p-3 bg-slate-50 rounded-lg border border-slate-200 mb-3">
              <label className="text-sm font-medium">
                الموظف
                <select
                  className="input mt-1 !w-56"
                  value={paysEmp}
                  onChange={(e) =>
                    setPaysEmp(
                      e.target.value === "" ? "" : Number(e.target.value),
                    )
                  }
                >
                  <option value="">— كل الموظفين —</option>
                  {emps.map((e) => (
                    <option key={e.id} value={e.id}>
                      {e.name}
                      {e.pin ? ` (#${e.pin})` : ""}
                    </option>
                  ))}
                </select>
              </label>
              <label className="text-sm font-medium">
                الشهر
                <select
                  className="input mt-1 !w-44"
                  value={paysMonth}
                  onChange={(e) => setPaysMonth(e.target.value)}
                >
                  <option value="">— كل الأشهر —</option>
                  {paysMonths.map((m) => (
                    <option key={m} value={m}>
                      {pLabel(m)}
                    </option>
                  ))}
                </select>
              </label>
              {(paysEmp !== "" || paysMonth !== "") && (
                <button
                  className="btn-ghost !py-1.5 text-xs"
                  onClick={() => {
                    setPaysEmp("");
                    setPaysMonth("");
                  }}
                >
                  ✕ مسح الترشيح
                </button>
              )}
              <div className="mr-auto">
                <span className="badge bg-emerald-100 text-emerald-700 text-sm font-bold">
                  {paysShown.length} دفعة · {n(paysTotal)} {curSym}
                </span>
              </div>
            </div>

            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="bg-slate-100">
                    <th className="th">#</th>
                    <th className="th">الشهر</th>
                    <th className="th">الموظف</th>
                    <th className="th">المبلغ ({curSym})</th>
                    <th className="th">الطريقة</th>
                    <th className="th">الملاحظة</th>
                    <th className="th">تاريخ الدفع</th>
                    <th className="th">إجراء</th>
                  </tr>
                </thead>
                <tbody>
                  {paysShown.length === 0 && (
                    <tr>
                      <td
                        className="td text-center text-slate-400 py-8"
                        colSpan={8}
                      >
                        {allPays.length === 0
                          ? "لا مدفوعات مسجّلة بعد — علّم راتباً كمدفوع من كشف الرواتب أو بعد الطباعة."
                          : "لا مدفوعات ضمن الترشيح الحالي."}
                      </td>
                    </tr>
                  )}
                  {paysShown.map((p, i) => {
                    const pin = emps.find(
                      (e) => e.id === p.employee_id,
                    )?.pin;
                    return (
                      <tr
                        key={`${p.employee_id}-${p.period}`}
                        className="hover:bg-slate-50 cursor-pointer"
                        title="عرض تفاصيل الدفعة والقسيمة كما كانت وقت التعليم كمدفوع"
                        onClick={() => openFrozen(p)}
                      >
                        <td className="td">{i + 1}</td>
                        <td className="td">{pLabel(p.period)}</td>
                        <td className="td font-medium">
                          {p.employee_name ?? `#${p.employee_id}`}
                          {pin && (
                            <span className="text-slate-400 font-mono text-xs mr-1">
                              #{pin}
                            </span>
                          )}
                        </td>
                        <td className="td font-extrabold text-emerald-700">
                          {n(p.net)}
                        </td>
                        <td className="td">{p.method ?? "—"}</td>
                        <td className="td text-xs text-slate-500">
                          {p.note ?? "—"}
                        </td>
                        <td className="td font-mono text-xs">
                          {p.paid_at.slice(0, 10)}
                        </td>
                        <td className="td text-center">
                          <button
                            className="text-xs text-slate-400 hover:text-rose-600"
                            title="إلغاء تعليم الدفع (يعود «غير مدفوع»)"
                            onClick={(e) => {
                              e.stopPropagation(); // لا يفتح تفصيل الدفعة
                              unpayRec(p);
                            }}
                          >
                            ↩ إلغاء الدفع
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                  {paysShown.length > 0 && (
                    <tr className="bg-slate-50 font-extrabold">
                      <td className="td" colSpan={3}>
                        الإجمالي
                      </td>
                      <td className="td text-emerald-700">{n(paysTotal)}</td>
                      <td className="td text-xs" colSpan={4}>
                        {paysShown.length} دفعة
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* تفصيل دفعة مجمّدة: القسيمة كاملة كما كانت وقت التعليم كمدفوع
          (أيام الجدول + البنود + الإعدادات) + زر طباعتها دون أي فرق لاحق */}
      {frozenRec && (
        <div
          className="fixed inset-0 bg-black/50 z-[60] overflow-y-auto p-4 print:hidden"
          onClick={() => setFrozenRec(null)}
        >
          <div
            className="max-w-5xl mx-auto my-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2 mb-2">
              <div className="text-sm font-bold text-white">
                🧾{" "}
                {frozenRec.pay.employee_name ?? `#${frozenRec.pay.employee_id}`}{" "}
                — {pLabel(frozenRec.pay.period)} (كما كانت وقت الدفع)
              </div>
              <div className="flex gap-2">
                <button className="btn-primary" onClick={printFrozen}>
                  🖨️ طباعة كما كانت وقت الدفع
                </button>
                <button
                  className="btn-ghost bg-white text-slate-700"
                  onClick={() => setFrozenRec(null)}
                >
                  ✕ إغلاق
                </button>
              </div>
            </div>
            <div className="slip-overlay">
              <div className="report-sheet pay-slip p-8 bg-white shadow-2xl rounded-xl">
                {frozenNode}
              </div>
            </div>
          </div>
        </div>
      )}
      {/* ورقة الطباعة المجمّدة — تُرفع خارج main لتصبح ورقة تُطبع وحدها */}
      {frozenRec &&
        createPortal(
          <div className="frozen-sheet report-sheet pay-slip p-8">
            {frozenNode}
          </div>,
          document.body,
        )}

      {/* نافذة تعليم راتب موظف كمدفوع */}
      {payEmp && (
        <PayModal
          empName={payEmp.employee_name}
          period={period}
          net={payEmp.net_pay}
          sym={curSym}
          busy={payBusy}
          onCancel={() => setPayEmp(null)}
          onConfirm={(method, note) => doPay(payEmp, method, note)}
        />
      )}

      {/* تأكيد داخل التطبيق (نعم/لا) — بديل مضمون لنافذة النظام:
          «نعم» وحدها تنفّذ الإجراء، و«لا»/النقر خارجها يلغيان تماماً */}
      {ask && (
        <div
          className="fixed inset-0 bg-black/50 z-[60] flex items-center justify-center p-4 print:hidden"
          onClick={() => setAsk(null)}
        >
          <div
            className="card w-full max-w-sm space-y-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="font-extrabold text-lg">{ask.title}</div>
            <div className="text-sm text-slate-600 leading-relaxed">
              {ask.body}
            </div>
            <div className="flex gap-2 justify-end">
              <button
                className="btn-ghost"
                autoFocus
                onClick={() => setAsk(null)}
              >
                لا
              </button>
              <button
                className={
                  ask.danger
                    ? "btn-primary !bg-rose-600 hover:!bg-rose-700"
                    : "btn-primary"
                }
                onClick={async () => {
                  const r = ask;
                  setAsk(null);
                  await r.run();
                }}
              >
                {ask.yes}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** نافذة تعليم راتب موظف كمدفوع: المبلغ + طريقة الدفع + ملاحظة — ويُحفظ معه
 * تاريخ الدفع ولقطة إعدادات طباعة القسيمة السارية وقتها (إن كان التقسيم مفعلاً). */
function PayModal({
  empName,
  period,
  net,
  sym,
  busy,
  onCancel,
  onConfirm,
}: {
  empName: string;
  period: string;
  net: number;
  sym: string;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (method: string, note: string) => void;
}) {
  const [method, setMethod] = useState("نقدي");
  const [note, setNote] = useState("");
  return (
    <div
      className="fixed inset-0 bg-black/50 z-50 flex items-center justify-center p-4 print:hidden"
      onClick={onCancel}
    >
      <div
        className="card w-full max-w-sm space-y-4"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="font-extrabold text-lg">💵 تعليم كمدفوع</div>
        <div className="text-sm text-slate-600">
          {empName} — صافي شهر {period}
          <div className="text-2xl font-extrabold text-emerald-700 mt-1">
            {n(net)} {sym}
          </div>
        </div>
        <label className="text-sm font-medium block">
          طريقة الدفع
          <select
            className="input mt-1"
            value={method}
            onChange={(e) => setMethod(e.target.value)}
          >
            <option>نقدي</option>
            <option>تحويل مصرفي</option>
            <option>شيك</option>
            <option>أخرى</option>
          </select>
        </label>
        <label className="text-sm font-medium block">
          ملاحظة (اختياري)
          <input
            className="input mt-1"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            placeholder="مثال: رقم حوولة / اسم المصرف…"
          />
        </label>
        <p className="text-xs text-slate-500 leading-relaxed">
          يُحفظ المبلغ وتاريخ الدفع، وتُحفظ معه إعدادات طباعة القسيمة ونسبها
          السارية وقتها — لا تتأثر الأرشفة ولا الرواتب المحسوبة.
        </p>
        <div className="flex gap-2 justify-end">
          <button className="btn-ghost" onClick={onCancel}>
            إلغاء
          </button>
          <button
            className="btn-primary"
            disabled={busy}
            onClick={() => onConfirm(method, note)}
          >
            {busy ? "جارٍ…" : "✓ تأكيد الدفع"}
          </button>
        </div>
      </div>
    </div>
  );
}

/** محتوى قسيمة الراتب — مشترك بين الطباعة والعرض على الشاشة. */
function SlipSheet({
  cfg,
  logo,
  period,
  selName,
  selJob,
  pin,
  sel,
  slipDays,
  slip,
}: {
  cfg: Settings;
  logo: string;
  period: string;
  selName: string;
  selJob: string;
  pin: string;
  sel: PayrollResult | null;
  slipDays: AttendanceDay[];
  slip: SlipCfg;
}) {
  const curSym =
    cfg.currency === "USD" ? "$" : cfg.currency === "JOD" ? "د.أ" : "₪";
  // تقسيم قانوني للطباعة فقط: null إن كان ملغى أو النِسب ≠ 100%
  const split = sel ? splitSlip(sel, slipDays, slip) : null;

  // ملخص الساعات أسفل جدول الأيام: الأساسية ×1.0 والإضافية ×1.5 (وعطلة رسمية إن وُجدت)
  const holHours = holidayHoursOf(slipDays);
  const basicHours = sel
    ? Math.max(0, sel.total_worked_hours - sel.overtime_hours - holHours)
    : 0;

  // جدول الأيام: يُقسَّم إلى نصفين جنباً إلى جنب إن كان طويلاً (لتناسب صفحة واحدة)
  const splitDays = slipDays.length > 14;
  const half = Math.ceil(slipDays.length / 2);
  const dayTable = (list: AttendanceDay[], from: number) => (
    <table className={`report-table${splitDays ? " slip-days" : ""}`}>
      {splitDays && (
        <colgroup>
          <col style={{ width: "7%" }} />
          <col style={{ width: "33%" }} />
          <col style={{ width: "15%" }} />
          <col style={{ width: "15%" }} />
          <col style={{ width: "13%" }} />
          <col style={{ width: "17%" }} />
        </colgroup>
      )}
      <thead>
        <tr>
          <th>#</th>
          <th>اليوم والتاريخ</th>
          <th>الحضور</th>
          <th>الانصراف</th>
          <th>الساعات</th>
          <th>الحالة</th>
        </tr>
      </thead>
      <tbody>
        {list.length === 0 && (
          <tr>
            <td colSpan={6} className="text-center text-slate-400">
              لا توجد أيام حضور محسوبة لهذه الفترة.
            </td>
          </tr>
        )}
        {list.map((d, i) => {
          const [label, cls] = stInfo(d);
          return (
            <tr key={d.id}>
              <td>{from + i + 1}</td>
              <td className="font-mono">{d.date}</td>
              <td className="font-mono">{hhmm(d.first_in)}</td>
              <td className="font-mono">
                {hhmm(d.last_out)}
                {d.last_out && d.last_out.slice(0, 10) !== d.date && (
                  <span className="block text-[9px] text-slate-400">
                    {d.last_out.slice(0, 10)}
                  </span>
                )}
              </td>
              <td>{hrs(d.worked_min)}</td>
              <td>
                <span
                  className={`px-2 py-0.5 rounded-full text-xs font-bold ${cls}`}
                >
                  {label}
                </span>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
  return (
    <>
          <PrintHeader
            cfg={cfg}
            logo={logo}
            title="قسيمة راتب"
            meta={
              <>
                <div className="text-slate-600">الفترة: {period}</div>
                <div className="text-slate-500 text-xs">
                  تاريخ الطباعة: {todayISO()}
                </div>
              </>
            }
          />

          <div className="grid grid-cols-2 gap-3 text-sm mb-4">
            <div className="border border-slate-200 rounded p-3">
              <div className="text-slate-500 text-xs">الموظف</div>
              <div className="font-extrabold">{selName}</div>
              <div className="text-slate-600">
                رقم البصمة: {pin}
                {selJob ? ` · ${selJob}` : ""}
              </div>
            </div>
            <div className="border border-slate-200 rounded p-3">
              <div className="text-slate-500 text-xs">الفترة</div>
              <div className="font-extrabold">{period}</div>
              <div className="text-slate-600">
                {sel ? `${sel.days_present} يوم حضور · ${n(sel.total_worked_hours)} ساعة فعلية` : "—"}
              </div>
            </div>
          </div>

          <div className="text-sm font-bold mb-1">
            أيام الدوام وأوقات الدخول والخروج
          </div>
          {splitDays ? (
            <div className="grid grid-cols-2 gap-x-3">
              {dayTable(slipDays.slice(0, half), 0)}
              {dayTable(slipDays.slice(half), half)}
            </div>
          ) : (
            dayTable(slipDays, 0)
          )}

          {/* ملخص الساعات: الأساسية + الإضافية (×1.5) + إجمالي الفعلي */}
          {sel && (
            <div className="flex flex-wrap gap-2 mt-3 text-xs font-bold">
              <span className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-1.5">
                ساعات الدوام الأساسية: {n(basicHours)} ساعة
              </span>
              <span className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-1.5 text-amber-700">
                ساعات إضافية (×{n(slip.otMult)}): {n(sel.overtime_hours)} ساعة
              </span>
              {holHours > 0 && (
                <span className="rounded-lg border border-fuchsia-200 bg-fuchsia-50 px-3 py-1.5 text-fuchsia-700">
                  دوام عطلة رسمية (×1.5): {n(holHours)} ساعة
                </span>
              )}
              <span className="rounded-lg border border-indigo-200 bg-indigo-50 px-3 py-1.5 text-indigo-700">
                إجمالي الساعات الفعلية: {n(sel.total_worked_hours)} ساعة
              </span>
            </div>
          )}

          {sel && (
            <>
              {slip.enabled && !slipSumOk(slip) && (
                <div className="print:hidden mt-4 rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm font-bold text-amber-700">
                  ⚠️ نِسب البدلات الثلاثة ({slipPctSum(slip)}%) لا تساوي 100% —
                  التقسيم غير مطبَّق. عدّلها من «إعدادات الطباعة لقسيمة الراتب».
                </div>
              )}
              <table className="report-table mt-4">
                <tbody>
                  {split ? (
                    <>
                      <tr>
                        <td
                          colSpan={2}
                          className="font-extrabold text-right"
                          style={{ background: "#f1f5f9" }}
                        >
                          الراتب الأساسي
                        </td>
                      </tr>
                      <tr>
                        <td className="text-right w-1/2">
                          الأساسي (ساعات الدوام × سعر الساعة {n(slip.rate)})
                        </td>
                        <td>
                          {n(split.basic)} {curSym}
                        </td>
                      </tr>
                      <tr>
                        <td
                          className="text-right"
                          style={{ borderTop: "1px dashed #94a3b8" }}
                        >
                          بدل العطلة والأيام المدفوعة ({sel.paid_workdays} يوم
                          دوام)
                        </td>
                        <td
                          className="text-violet-600"
                          style={{ borderTop: "1px dashed #94a3b8" }}
                        >
                          {sel.weekend_pay
                            ? `+ ${n(sel.weekend_pay)} ${curSym}`
                            : "—"}
                        </td>
                      </tr>
                      <tr className="subtotal-row">
                        <td className="text-right">إجمالي الراتب الأساسي</td>
                        <td>
                          {n(split.basic + sel.weekend_pay)} {curSym}
                        </td>
                      </tr>
                      <tr>
                        <td className="font-bold text-right">
                          العمل الإضافي ({n(sel.overtime_hours)} ساعة ×{" "}
                          {n(slip.otMult)} × {n(slip.rate)})
                        </td>
                        <td className="text-amber-700">
                          + {n(split.ot)} {curSym}
                        </td>
                      </tr>
                    </>
                  ) : (
                    <tr>
                      <td className="font-bold text-right w-1/2">الأساسي</td>
                      <td>
                        {n(sel.base_pay)} {curSym}
                      </td>
                    </tr>
                  )}
                  <tr>
                    <td className="font-bold text-right">
                      البدلات (فطور + مواصلات)
                    </td>
                    <td>
                      {n(sel.allowances)} {curSym}
                    </td>
                  </tr>
                  {!split && (
                    <tr>
                      <td className="font-bold text-right">
                        بدل العطلة والأيام المدفوعة ({sel.paid_workdays} يوم
                        دوام)
                      </td>
                      <td className="text-violet-600">
                        {sel.weekend_pay
                          ? `+ ${n(sel.weekend_pay)} ${curSym}`
                          : "—"}
                      </td>
                    </tr>
                  )}
                  {(sel.holiday_work_days ?? 0) > 0 && (
                    <tr>
                      <td className="font-bold text-right">
                        دوام في عطلة رسمية ({sel.holiday_work_days} يوم — 62 +
                        ساعات × 1.5)
                      </td>
                      <td className="text-fuchsia-600">
                        + {n(sel.holiday_work_pay ?? 0)} {curSym}
                      </td>
                    </tr>
                  )}
                  {(sel.comp_off_days ?? 0) > 0 && (
                    <tr>
                      <td className="font-bold text-right">
                        أيام بدل تعويضية ({sel.comp_off_days} يوم — أجر يوم
                        العطلة 62)
                      </td>
                      <td className="text-fuchsia-600">
                        + {n(sel.comp_off_pay ?? 0)} {curSym}
                      </td>
                    </tr>
                  )}
                  {split && (
                    <>
                      <tr>
                        <td className="font-bold text-right">
                          بدل طبيعة العمل
                        </td>
                        <td className="text-emerald-700">
                          + {n(split.shareWork)} {curSym}
                        </td>
                      </tr>
                      <tr>
                        <td className="font-bold text-right">
                          بدل المخاطرة والسلامة المهنية
                        </td>
                        <td className="text-emerald-700">
                          + {n(split.shareRisk)} {curSym}
                        </td>
                      </tr>
                      <tr>
                        <td className="font-bold text-right">
                          مكافأة الإنجاز / الإنتاجية
                        </td>
                        <td className="text-emerald-700">
                          + {n(split.shareBonus)} {curSym}
                        </td>
                      </tr>
                    </>
                  )}
                  {(sel.paid_leave_days > 0 ||
                    sel.unpaid_leave_days > 0 ||
                    sel.holiday_days > 0 ||
                    (sel.comp_off_days ?? 0) > 0) && (
                    <tr>
                      <td className="text-right text-xs text-slate-500">
                        إجازات: مدفوعة {sel.paid_leave_days} · غير مدفوعة{" "}
                        {sel.unpaid_leave_days} · رسمية {sel.holiday_days}
                        {(sel.comp_off_days ?? 0) > 0
                          ? ` · بدل ${sel.comp_off_days}`
                          : ""}{" "}
                        (المدفوعة محسوبة ضمن البدل أعلاه)
                      </td>
                      <td className="text-xs text-slate-500">—</td>
                    </tr>
                  )}
                  <tr className="total-row">
                    <td className="font-extrabold text-right">الصافي المستحق</td>
                    <td className="font-extrabold text-base">
                      {n(sel.net_pay)} {curSym}
                    </td>
                  </tr>
                </tbody>
              </table>
            </>
          )}

          <div className="grid grid-cols-2 gap-8 mt-6 text-sm text-center">
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">توقيع المستلم</div>
            </div>
            <div>
              <div className="h-12 border-b border-dotted border-slate-400" />
              <div className="mt-1 font-bold">مسؤول الرواتب</div>
            </div>
          </div>

          <PrintFooter cfg={cfg} />
    </>
  );
}
