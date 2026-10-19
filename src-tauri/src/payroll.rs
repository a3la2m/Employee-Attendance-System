//! محرك حساب الرواتب — الأجر بالساعة الفعلية بدون أي خصومات.
//!
//! القواعد:
//! 1. وقت الدوام الفعلي = (الانصراف - الحضور) - فترة الخصم (استراحة).
//! 2. الأجر الأساسي = الساعات الفعليّة × سعر الساعة (عمل ساعة = أجر ساعة).
//! 3. البدلات = البدلات اليومية × أيام الحضور.
//! 4. بدل العطلة الأسبوعية — طريقتان تُختاران من الإعدادات (`weekend_pay_mode`):
//!    أ) "weekly" (الافتراضي): الأسبوع من السبت إلى الجمعة وينتهي بيوم عطلته
//!       (الجمعة)، ومبلغ بدل العطلة (`weekend_allow` — 62) عن ذلك الأسبوع يُقسَّم
//!       على ستة أيام دوام (السبت→الخميس) ويُدفع بنسبة الدوام الفعلي: دوام كامل
//!       = 62 كاملة، وكل يوم غياب يُخصم نصيبه (62 ÷ 6). من داوم يوم عطلته
//!       ساعاته مدفوعة عادياً وبالمضاعف إن زادت، وبدل الأسبوع يبقى حسب نسبة
//!       دوام أيامه الأخرى. أيام الفترة وحدها تدخل القسمة، وأسبوع بلا دوام
//!       مستحق = بلا بدل. (شهر كامل بخمس جمع = 5 × 62 = 310)
//!    ب) "per_day": 62 كاملة عن كل يوم عطلة أسبوعية مستحق ما لم يعمل فيه،
//!       بشرط أن يكون أسبوعه فيه دوام (القاعدة القديمة — تبقى مختارةً للتوافق).
//! 5. لا خصومات إطلاقاً: لا خصم تأخير ولا خصم غياب — الغياب يُدفع بالساعات
//!    المعمَّلة فقط، والإجازة غير المدفوعة تُهمَل بلا خصم.
//! 6. العمل الإضافي: بعد تجاوز الساعات الفعلية (بعد الاستراحة) لساعات الدوام
//!    الأساسية — يُضاف بمضاعف. العمل قد يكون في أي ساعة فلا حدّ لبداية اليوم.

use crate::models::{AttendanceDay, PayrollResult, PayrollSettings};
use chrono::{Datelike, NaiveDate};

/// يحسب راتب شهر (period بصيغة "YYYY-MM") من قائمة أيام الحضور المحسوبة مسبقاً.
///
/// `days` = كل أيام attendance_days الخاص بالموظف خلال الفترة.
/// `weekend_paid` = هل العطلة الأسبوعية مدفوعة لهذا الموظف (تُلغى لموظف بعينه).
/// `weekend` = أيام العطلة أسبوعياً (0=اثنين … 5=سبت).
pub fn compute_payroll(
    emp_id: i64,
    emp_name: &str,
    pin: Option<String>,
    period: &str,
    days: &[AttendanceDay],
    weekend_paid: bool,
    weekend: &[u32],
    s: &PayrollSettings,
) -> PayrollResult {
    let mut days_present: i64 = 0;
    let mut total_worked_min: i64 = 0;
    let mut overtime_min: i64 = 0;
    let mut paid_leave_days: i64 = 0;
    let mut unpaid_leave_days: i64 = 0;
    let mut holiday_days: i64 = 0;

    for d in days {
        if d.status == "leave" {
            // إجازة: لا ساعات ولا بدلات — تُعدّ للمعلومة ولبدل العطلة إن مدفوعة
            if d.leave_paid.unwrap_or(false) {
                paid_leave_days += 1;
            } else {
                unpaid_leave_days += 1;
            }
            continue;
        }
        if d.status == "holiday" {
            holiday_days += 1; // عطلة رسمية: بلا ساعات (بدلها اليومي من بدل العطلة)
            continue;
        }
        if d.status == "weekend" {
            // عطلة أسبوعية: بلا ساعات ولا بدلات يومية (بدلها بدل العطلة أدناه)
            continue;
        }
        if d.absent || d.first_in.is_none() {
            continue;
        }
        days_present += 1;
        total_worked_min += d.worked_min;
        overtime_min += d.overtime_min;
    }

    // 1) الساعات الفعلية (بدون الإضافي — الإضافي يُحسب منفصلاً بالمضاعف)
    let regular_min = (total_worked_min - overtime_min).max(0);
    let worked_hours = regular_min as f64 / 60.0;
    let overtime_hours = overtime_min as f64 / 60.0;

    // 2) الأجر الأساسي (للوقت العادي)
    let base_pay = worked_hours * s.hourly_rate;

    // 2b) أجر العمل الإضافي بمضاعف
    let overtime_pay = overtime_hours * s.hourly_rate * s.overtime_rate_mult;

    // 3) البدلات اليومية × أيام الحضور
    let daily_allow = s.meal_allow + s.transport_allow;
    let allowances = daily_allow * days_present as f64;

    // 4) بدل العطلة الأسبوعية — حسب الطريقة المختارة في الإعدادات:
    //    "weekly" (الافتراضي): 62 عن كل أسبوع ÷ أيام السبت–خميس بالنسبة،
    //    و"per_day": 62 عن كل يوم عطلة مستحق لا يعمل فيه الموظف.
    //    انظر weekly_weekend_pay وcount_paid_weekend_days.
    let paid_workdays = count_paid_workdays(days, weekend);
    let (weekend_pay, weekend_days_paid) = if !weekend_paid {
        (0.0, 0)
    } else if s.weekend_pay_mode.trim() == "per_day" {
        let c = count_paid_weekend_days(days, weekend);
        (s.weekend_allow * c as f64, c)
    } else {
        weekly_weekend_pay(days, weekend, s.weekend_allow)
    };

    // 5) لا خصومات: التأخير والغياب لا يُخصم منهما شيء
    let late_deduction = 0.0;
    let absence_deduction = 0.0;

    // 6) الصافي
    let net_pay = base_pay + overtime_pay + allowances + weekend_pay;

    PayrollResult {
        employee_id: emp_id,
        employee_name: emp_name.to_string(),
        pin,
        period: period.to_string(),
        days_present,
        total_worked_hours: worked_hours,
        overtime_hours,
        base_pay: base_pay + overtime_pay, // الأساسي يشمل الإضافي للعرض
        allowances,
        weekend_pay,
        paid_workdays,
        weekend_days_paid,
        paid_leave_days,
        unpaid_leave_days,
        holiday_days,
        late_deduction,
        absence_deduction,
        net_pay,
    }
}

/// أيام العطلة الأسبوعية التي يستحق عنها الموظف بدل العطلة في الفترة.
///
/// **القاعدة (بدل اليوم الكامل لكل يوم عطلة):** عن كل يوم من أيام العطلة
/// الأسبوعية في الفترة (تاريخه weekday ∈ weekend) يستحق الموظف `weekend_allow`
/// (افتراضياً 62) بشرطين:
/// 1. **لم يعمل فيه** — من دوام يوم عطلته فساعاته مدفوعة بالساعات ولا بدل له
///    عن ذلك اليوم (لا يُعوّض ببدلين).
/// 2. **أسبوعه فيه دوام** — داوم (حضور فعلي، أو إجازة مدفوعة، أو عطلة رسمية)
///    في أي يوم دوام خلال الأسبوع المنتهي بذلك اليوم العطلة [D‑6 … D]؛
///    أسبوع بلا أي دوام مستحق لا يستحق بدل عطلته.
///
/// `weekend` = أيام العطلة أسبوعياً (0=اثنين … 4=جمعة 5=سبت، بحسب chrono
/// num_days_from_monday). العطلة غير المدفوعة للموظف تُصفّر البدل من compute_payroll.
pub fn count_paid_weekend_days(days: &[AttendanceDay], weekend: &[u32]) -> i64 {
    if weekend.is_empty() {
        return 0;
    }
    let mut by_date: std::collections::HashMap<NaiveDate, &AttendanceDay> =
        std::collections::HashMap::new();
    for d in days {
        if let Ok(dt) = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") {
            by_date.insert(dt, d);
        }
    }
    // يوم دوام مستحق: حضور فعلي، أو إجازة مدفوعة، أو عطلة رسمية (ليس يوم عطلة)
    let paid_workday = |d: &AttendanceDay, dt: NaiveDate| -> bool {
        if weekend.contains(&dt.weekday().num_days_from_monday()) {
            return false;
        }
        if d.status == "leave" {
            d.leave_paid.unwrap_or(false)
        } else if d.status == "holiday" {
            true
        } else {
            !d.absent && d.first_in.is_some()
        }
    };
    let mut count = 0;
    for d in days {
        let Ok(dt) = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") else {
            continue;
        };
        if !weekend.contains(&dt.weekday().num_days_from_monday()) {
            continue;
        }
        // (1) عمل يوم عطلته → لا بدل عن هذا اليوم
        if !d.absent && d.first_in.is_some() && d.worked_min > 0 {
            continue;
        }
        // (2) الأسبوع المنتهي بيوم العطلة [D‑6 … D] يجب أن يحوي دواماً مستحقاً
        let week_earned = (0..7).any(|k| {
            dt.checked_sub_signed(chrono::Duration::days(k))
                .and_then(|w| by_date.get(&w).map(|wd| paid_workday(wd, w)))
                .unwrap_or(false)
        });
        if week_earned {
            count += 1;
        }
    }
    count
}

/// بدل العطلة الأسبوعية **بالطريقة الأسبوعية** (`weekend_pay_mode="weekly"` — الافتراضي):
/// الأسبوع من السبت إلى الجمعة ينتهي بيوم عطلته (الجمعة)، ومبلغ بدل العطلة
/// (`weekend_allow` — 62) عن ذلك الأسبوع يُقسَّم على أيام الدوام الستّ (السبت→الخميس)
/// ويُصرف بنسبة الدوام المستحق فيها: دوام كامل = 62 كاملة، وكل يوم غياب يُخصم
/// نصيبه منه (62 ÷ 6). يعمل الموظف يوم عطلته؟ ساعاته تُدفع عادياً وبالمضاعف إن
/// زادت، ويبقى بدل الأسبوع حسب نسبة دوام أيامه الأخرى. أيام الفترة وحدها تدخل
/// القسمة (أسبوع حدّي عند بداية الشهر يُحتسب على أيامه المتاحة داخل الفترة)،
/// وأسبوع بلا دوام مستحق = بلا بدل. (أسبوع كامل = 62؛ شهر بخمس جمع = 310)
///
/// يُرجع (المبلغ، عدد أيام العطلة التي دُفع عنها بدل).
pub fn weekly_weekend_pay(
    days: &[AttendanceDay],
    weekend: &[u32],
    allowance: f64,
) -> (f64, i64) {
    if weekend.is_empty() {
        return (0.0, 0);
    }
    let mut by_date: std::collections::HashMap<NaiveDate, &AttendanceDay> =
        std::collections::HashMap::new();
    for d in days {
        if let Ok(dt) = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") {
            by_date.insert(dt, d);
        }
    }
    // يوم دوام مستحق داخل الأسبوع: حضور فعلي، أو إجازة مدفوعة، أو عطلة رسمية
    let paid_workday = |d: &AttendanceDay| -> bool {
        if d.status == "leave" {
            d.leave_paid.unwrap_or(false)
        } else if d.status == "holiday" {
            true
        } else {
            !d.absent && d.first_in.is_some()
        }
    };
    // نهاية الأسبوع = آخر يوم عطلة الإعدادي (الجمعة = 4 افتراضياً)
    let end_wd = *weekend.iter().max().unwrap();
    // جمّع كل يوم عطلة داخل الفترة في أسبوعه (أساس الأسبوع = آخر يوم عطلة فيه)
    let mut buckets: std::collections::BTreeMap<NaiveDate, i64> = Default::default();
    for d in days {
        let Ok(dt) = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") else {
            continue;
        };
        if !weekend.contains(&dt.weekday().num_days_from_monday()) {
            continue;
        }
        let mut e = dt;
        while e.weekday().num_days_from_monday() != end_wd {
            e = e.succ_opt().unwrap();
        }
        *buckets.entry(e).or_insert(0) += 1;
    }

    let mut pay = 0.0;
    let mut paid_weekend_days = 0i64;
    for (&end, n_weekend) in &buckets {
        // أيام الدوام [end−6 … end] الحاضرة داخل الفترة: المتوقّع ثم المدفوع
        let (mut expected, mut earned) = (0i64, 0i64);
        for k in 0..7 {
            let Some(w) = end.checked_sub_signed(chrono::Duration::days(k)) else {
                continue;
            };
            if weekend.contains(&w.weekday().num_days_from_monday()) {
                continue;
            }
            if let Some(row) = by_date.get(&w) {
                expected += 1;
                if paid_workday(row) {
                    earned += 1;
                }
            }
        }
        if expected == 0 {
            continue;
        }
        let week_pay = allowance * earned as f64 / expected as f64;
        if week_pay > 0.0 {
            pay += week_pay;
            paid_weekend_days += *n_weekend;
        }
    }
    (pay, paid_weekend_days)
}

/// عدد أيام الدوام المحضورة (أو الإجازة المدفوعة/العطلة الرسمية) في الفترة —
/// رقم إعلامي يُعرض مع الأجر ولا يدخل في معادلة بدل العطلة الآن
/// (بدل العطلة يُحسب بعدّاد أيام العطلة الأسبوعية: انظر count_paid_weekend_days).
/// أيام العطلة الأسبوعية لا تُعدّ مهما عمل فيها (ساعاتها مدفوعة أصلاً)،
/// والإجازة غير المدفوعة والغياب بلا استحقاق.
///
/// `weekend` = أيام العطلة أسبوعياً (0=اثنين … 4=جمعة 5=سبت، بحسب chrono
/// num_days_from_monday).
pub fn count_paid_workdays(days: &[AttendanceDay], weekend: &[u32]) -> i64 {
    let mut count = 0;
    for d in days {
        let Ok(dt) = NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") else {
            continue;
        };
        // يوم العطلة الأسبوعية لا يستحق بدل عطلة (ولو حضر أو كان بإجازة)
        if weekend.contains(&dt.weekday().num_days_from_monday()) {
            continue;
        }
        let earns = if d.status == "leave" {
            d.leave_paid.unwrap_or(false)
        } else if d.status == "holiday" {
            true
        } else {
            !d.absent && d.first_in.is_some()
        };
        if earns {
            count += 1;
        }
    }
    count
}

/// يحوّل إعداد "weekend_days" (مثال "4") إلى مصفوفة أيام، بافتراضي الجمعة فقط
/// (الأسبوع يبدأ السبت وينتهي الجمعة).
pub fn parse_weekend(s: Option<&str>) -> Vec<u32> {
    match s {
        Some(v) if !v.trim().is_empty() => v
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect(),
        _ => vec![4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> PayrollSettings {
        PayrollSettings {
            daily_hours: 8.5,
            break_min: 30,
            deduct_break: true,
            hourly_rate: 10.0,
            meal_allow: 7.0,
            transport_allow: 5.0,
            overtime_after_hours: 8.5,
            overtime_rate_mult: 1.5,
            grace_late_min: 0,
            weekend_allow: 62.0,
            weekend_pay_mode: "per_day".into(), // اختبارات الوضع القديم (لكل يوم عطلة)
            work_start: "09:00".into(),
            work_end: "17:30".into(),
        }
    }

    fn day(date: &str, worked: i64, late: i64, ot: i64) -> AttendanceDay {
        AttendanceDay {
            id: 0,
            employee_id: 1,
            date: date.into(),
            first_in: Some(format!("{}T09:00:00", date)),
            last_out: Some(format!("{}T17:30:00", date)),
            worked_min: worked,
            late_min: late,
            early_leave_min: 0,
            overtime_min: ot,
            absent: false,
            leave_type: None,
            status: "present".into(),
            leave_paid: None,
            manual: false,
        }
    }

    /// يوم إجازة (مدفوعة أو غير مدفوعة) بلا ساعات.
    fn leave_day(date: &str, leave_type: &str, paid: bool) -> AttendanceDay {
        AttendanceDay {
            id: 0,
            employee_id: 1,
            date: date.into(),
            first_in: None,
            last_out: None,
            worked_min: 0,
            late_min: 0,
            early_leave_min: 0,
            overtime_min: 0,
            absent: false,
            leave_type: Some(leave_type.into()),
            status: "leave".into(),
            leave_paid: Some(paid),
            manual: false,
        }
    }

    /// يوم عطلة رسمية (عيد) بلا ساعات.
    fn holiday_day(date: &str) -> AttendanceDay {
        AttendanceDay {
            id: 0,
            employee_id: 1,
            date: date.into(),
            first_in: None,
            last_out: None,
            worked_min: 0,
            late_min: 0,
            early_leave_min: 0,
            overtime_min: 0,
            absent: false,
            leave_type: None,
            status: "holiday".into(),
            leave_paid: None,
            manual: false,
        }
    }

    #[test]
    fn normal_day_basic() {
        // يوم كامل 8.5 سا: 8.5*10 = 85 + بدلات 12 = 97 (بدون عطلة مدفوعة هنا)
        let days = vec![day("2025-06-01", 510, 0, 0)];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, false, &[4, 5], &s());
        assert_eq!(r.days_present, 1);
        assert!((r.total_worked_hours - 8.5).abs() < 1e-9);
        assert!((r.base_pay - 85.0).abs() < 1e-9);
        assert!((r.allowances - 12.0).abs() < 1e-9);
        assert!((r.late_deduction).abs() < 1e-9);
        assert!((r.absence_deduction).abs() < 1e-9);
        assert!((r.weekend_pay).abs() < 1e-9);
        assert!((r.net_pay - 97.0).abs() < 1e-9);
    }

    #[test]
    fn hour_worked_is_hour_paid() {
        // عمل ساعة واحدة فقط: 60 د × 10 = 10 + بدلات 12 = 22 — بلا خصم غياب
        let days = vec![day("2025-06-01", 60, 0, 0)];
        let r = compute_payroll(1, "مها", None, "2025-06", &days, false, &[4, 5], &s());
        assert!((r.total_worked_hours - 1.0).abs() < 1e-9);
        assert!((r.base_pay - 10.0).abs() < 1e-9);
        assert!((r.absence_deduction).abs() < 1e-9);
        assert!((r.net_pay - 22.0).abs() < 1e-9);
    }

    #[test]
    fn overtime_pays_multiplier() {
        // عادي 8.5 سا (510 د) + 45 د إضافي = 555 د إجمالي
        // base = 8.5*10=85 + إضافي 0.75*10*1.5=11.25 -> 96.25
        let days = vec![day("2025-06-02", 555, 0, 45)];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, false, &[4, 5], &s());
        assert_eq!(r.overtime_hours, 0.75);
        assert!((r.total_worked_hours - 8.5).abs() < 1e-9);
        assert!((r.base_pay - (85.0 + 11.25)).abs() < 1e-9);
        // net = base(96.25) + allowances(12)
        assert!((r.net_pay - 108.25).abs() < 1e-9);
    }

    #[test]
    fn late_deducts_nothing() {
        // تأخير 30 دقيقة: لا يُخصم شيء — يُدفع ما عمله فقط
        let days = vec![day("2025-06-03", 480, 30, 0)];
        let r = compute_payroll(1, "سارة", None, "2025-06", &days, false, &[4, 5], &s());
        assert_eq!(r.late_deduction, 0.0);
        // base = 8.0*10=80 + allow(12) = 92
        assert!((r.net_pay - 92.0).abs() < 1e-9);
    }

    #[test]
    fn absence_deducts_nothing() {
        // شهر كامل غياب (بلا أي حضور): لا خصم إطلاقاً — صفر بدل سالب
        let days: Vec<AttendanceDay> = vec![];
        let r = compute_payroll(1, "خالد", None, "2025-06", &days, false, &[4, 5], &s());
        assert_eq!(r.days_present, 0);
        assert_eq!(r.absence_deduction, 0.0);
        assert!((r.net_pay).abs() < 1e-9);
    }

    /// يوم عطلة أسبوعية بلا ساعات (status="weekend") — لا يدخل الأجر اليومي.
    fn off_day(date: &str) -> AttendanceDay {
        AttendanceDay {
            id: 0,
            employee_id: 1,
            date: date.into(),
            first_in: None,
            last_out: None,
            worked_min: 0,
            late_min: 0,
            early_leave_min: 0,
            overtime_min: 0,
            absent: false,
            leave_type: None,
            status: "weekend".into(),
            leave_paid: None,
            manual: false,
        }
    }

    #[test]
    fn weekend_paid_accrues_per_attended_workday() {
        // بدل العطلة: 62 عن كل يوم عطلة أسبوعية في الأسبوع ذي الدوام.
        // داوم السبت→الخميس ثم جمعة عطلة → 62 كاملة (لا تقسيم على أيام الدوام)
        let days = vec![
            day("2025-06-07", 510, 0, 0), // السبت
            day("2025-06-08", 510, 0, 0), // الأحد
            day("2025-06-09", 510, 0, 0), // الاثنين
            off_day("2025-06-13"),        // الجمعة: عطلة أسبوعية
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.paid_workdays, 3);
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
        // net = 3 أيام (85+12 = 291) + 62 = 353
        assert!((r.net_pay - 353.0).abs() < 1e-9);
    }

    #[test]
    fn full_week_work_gets_full_allowance() {
        // داوم أيام الدوام الستة كاملة (سبت→خميس) وجُمعته عطلة → بدل 62 كامل
        let days: Vec<AttendanceDay> = ["2025-06-07", "2025-06-08", "2025-06-09", "2025-06-10", "2025-06-11", "2025-06-12"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .chain(std::iter::once(off_day("2025-06-13")))
            .collect();
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.paid_workdays, 6);
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
    }

    #[test]
    fn weekend_and_friday_saturday_split() {
        // عطلة الجمعة+السبت: لكل يوم عطلة بدله الكامل — حضر الأحد→الخميس (5 أيام)
        // وعطلته الجمعة والسبت → 2 × 62 = 124 (بدل اليوم الواحد قابل للتعديل).
        let mut days: Vec<AttendanceDay> = ["2025-06-01", "2025-06-02", "2025-06-03", "2025-06-04", "2025-06-05"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .collect();
        days.push(off_day("2025-06-06")); // الجمعة
        days.push(off_day("2025-06-07")); // السبت
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4, 5], &s());
        assert_eq!(r.paid_workdays, 5);
        assert_eq!(r.weekend_days_paid, 2);
        assert!((r.weekend_pay - 124.0).abs() < 1e-9);
    }

    #[test]
    fn week_without_any_attendance_pays_no_weekend_allowance() {
        // أسبوع كامل بلا دوام ولا إجازة مدفوعة → جمعته بلا بدل عطلة
        let days = vec![off_day("2025-06-13")];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.weekend_days_paid, 0);
        assert!((r.weekend_pay).abs() < 1e-9);
        assert!((r.net_pay).abs() < 1e-9);
    }

    #[test]
    fn worked_weekend_day_not_counted_in_allowance() {
        // عمل يوم الجمعة (عطلة) وحده: ساعاته تُدفع ولا يستحق بدل عطلة
        let days = vec![day("2025-06-06", 510, 0, 0)]; // الجمعة 6 يونيو
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.paid_workdays, 0);
        assert_eq!(r.weekend_days_paid, 0);
        assert!((r.weekend_pay).abs() < 1e-9);
        // الأساسي مدفوع: 85 + 12 = 97
        assert!((r.net_pay - 97.0).abs() < 1e-9);
    }

    #[test]
    fn weekend_unpaid_for_specific_employee() {
        // العطلة غير مدفوعة لهذا الموظف: داوم 6 أيام وجمعته عطلة بلا بدل
        let days: Vec<AttendanceDay> = ["2025-06-07", "2025-06-08", "2025-06-09", "2025-06-10", "2025-06-11", "2025-06-12"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .chain(std::iter::once(off_day("2025-06-13")))
            .collect();
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, false, &[4], &s());
        assert_eq!(r.paid_workdays, 6);
        assert!((r.weekend_pay).abs() < 1e-9);
    }

    #[test]
    fn paid_leave_day_accrues_allowance() {
        // أسبوع فيه حضور وإجازة مدفوعة وجمعة عطلة → الجمعة تستحق 62 كاملة
        let days = vec![
            day("2025-06-07", 510, 0, 0),           // السبت: حضور
            leave_day("2025-06-08", "annual", true), // الأحد: إجازة مدفوعة
            off_day("2025-06-13"),                   // الجمعة: عطلة أسبوعية
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.days_present, 1);
        assert_eq!(r.paid_leave_days, 1);
        assert_eq!(r.paid_workdays, 2);
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
        // الصافي = حضور (85+12) + بدل العطلة
        assert!((r.net_pay - (97.0 + 62.0)).abs() < 1e-9);
    }

    #[test]
    fn unpaid_leave_day_no_pay_no_deduction() {
        // إجازة غير مدفوعة: لا بدل ولا خصم — يبقى دوام اليوم الآخر كافياً
        // لاستحقاق جمعة الأسبوع (دفع بالساعات فقط).
        let days = vec![
            day("2025-06-07", 510, 0, 0),
            leave_day("2025-06-08", "annual", false),
            off_day("2025-06-13"),
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.unpaid_leave_days, 1);
        assert_eq!(r.paid_workdays, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
        assert!((r.net_pay - (97.0 + 62.0)).abs() < 1e-9);
    }

    #[test]
    fn official_holiday_day_accrues_allowance() {
        // عطلة رسمية في أسبوع دوامه: تستحق الجمعة بدلها الكامل 62
        let days = vec![
            day("2025-06-07", 510, 0, 0),
            holiday_day("2025-06-08"),
            off_day("2025-06-13"),
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.holiday_days, 1);
        assert_eq!(r.paid_workdays, 2);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
    }

    #[test]
    fn leave_on_weekend_day_not_counted() {
        // إجازة مدفوعة تسقط في يوم عطلة أسبوعية → لا تستحق (العطلة أصلاً غير مستحقة)
        let days = vec![leave_day("2025-06-06", "annual", true)]; // الجمعة
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s());
        assert_eq!(r.paid_leave_days, 1);
        assert_eq!(r.paid_workdays, 0);
        assert!((r.weekend_pay).abs() < 1e-9);
    }

    #[test]
    fn employee_without_weekend_pay_gets_no_leave_pay() {
        // العطلة غير المدفوعة لهذا الموظف: حتى إجازته المدفوعة لا تستحق بدل عطلة
        let days = vec![
            day("2025-06-07", 510, 0, 0),
            leave_day("2025-06-08", "annual", true),
            off_day("2025-06-13"),
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, false, &[4], &s());
        assert_eq!(r.paid_workdays, 2);
        assert_eq!(r.weekend_days_paid, 0);
        assert!((r.weekend_pay).abs() < 1e-9);
    }

    // ------------------------------------------- الوضع الأسبوعي (الافتراضي الجديد)

    /// إعدادات الطريقة الأسبوعية: 62 عن كل أسبوع تُقسّم على أيام السبت–الخميس.
    fn s_weekly() -> PayrollSettings {
        PayrollSettings {
            weekend_pay_mode: "weekly".into(),
            ..s()
        }
    }

    /// يوم دوام بلا بصمة (غياب) داخل أيام الدوام الستة.
    fn absent_day(date: &str) -> AttendanceDay {
        AttendanceDay {
            id: 0,
            employee_id: 1,
            date: date.into(),
            first_in: None,
            last_out: None,
            worked_min: 0,
            late_min: 0,
            early_leave_min: 0,
            overtime_min: 0,
            absent: true,
            leave_type: None,
            status: "absent".into(),
            leave_paid: None,
            manual: false,
        }
    }

    #[test]
    fn weekly_full_week_gets_full_allowance() {
        // أسبوع كامل (سبت→خميس) بدوام كامل + جمعة عطلة → 62 كاملة (لا تقسيم خاسر)
        let days: Vec<AttendanceDay> = ["2025-06-07", "2025-06-08", "2025-06-09", "2025-06-10", "2025-06-11", "2025-06-12"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .chain(std::iter::once(off_day("2025-06-13")))
            .collect();
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        assert_eq!(r.paid_workdays, 6);
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
    }

    #[test]
    fn weekly_absence_deducts_one_share_per_day() {
        // يوم غياب من ستة أيام دوام → 62 × 5/6 = 51.666… (نصيب اليوم 62÷6 يُخصم)
        let days = vec![
            day("2025-06-07", 510, 0, 0),
            day("2025-06-08", 510, 0, 0),
            absent_day("2025-06-09"), // الاثنين: غياب
            day("2025-06-10", 510, 0, 0),
            day("2025-06-11", 510, 0, 0),
            day("2025-06-12", 510, 0, 0),
            off_day("2025-06-13"),
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        let expect = 62.0 * 5.0 / 6.0;
        assert_eq!(r.weekend_days_paid, 1);
        assert!(
            (r.weekend_pay - expect).abs() < 1e-9,
            "دُفع {:.4} بدل {:.4}",
            r.weekend_pay,
            expect
        );
    }

    #[test]
    fn weekly_worked_friday_keeps_week_allowance() {
        // 3#: دوام الجمعة يُدفع ساعاته عادياً + بدل الأسبوع 62 كامل نسبة دوام أيامه الأخرى
        let mut days: Vec<AttendanceDay> = ["2025-06-07", "2025-06-08", "2025-06-09", "2025-06-10", "2025-06-11", "2025-06-12"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .collect();
        days.push(day("2025-06-13", 510, 0, 0)); // الجمعة مداومة
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        assert_eq!(r.days_present, 7, "ساعات الجمعة تُدفع عادياً");
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
        // الأجر: 7 أيام × (85 + 12) + بدل العطلة 62
        assert!((r.net_pay - (7.0 * 97.0 + 62.0)).abs() < 1e-9, "net={}", r.net_pay);
    }

    #[test]
    fn weekly_partial_boundary_week_pays_from_available_days() {
        // أسبوع حدّي عند بداية الفترة: يوم دوام واحد متاح (الخميس) وحضره → 62 كاملة
        // (الأيام خارج الفترة لا تدخل القسمة فلا يُخصم شيء منها)
        let days = vec![day("2025-06-05", 510, 0, 0), off_day("2025-06-06")];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        assert_eq!(r.weekend_days_paid, 1);
        assert!((r.weekend_pay - 62.0).abs() < 1e-9);
    }

    #[test]
    fn weekly_week_without_work_pays_nothing() {
        // أسبوع بلا أي دوام مستحق (يوم عطلة وحده داخل الفترة) → صفر
        let days = vec![off_day("2025-06-13")];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        assert_eq!(r.weekend_days_paid, 0);
        assert!((r.weekend_pay).abs() < 1e-9);
        assert!((r.net_pay).abs() < 1e-9);
    }

    #[test]
    fn weekly_unpaid_employee_gets_nothing() {
        // العطلة غير مدفوعة لهذا الموظف: صفر مهما داوم
        let days: Vec<AttendanceDay> = ["2025-06-07", "2025-06-08"]
            .iter()
            .map(|d| day(d, 510, 0, 0))
            .chain(std::iter::once(off_day("2025-06-13")))
            .collect();
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, false, &[4], &s_weekly());
        assert!((r.weekend_pay).abs() < 1e-9);
    }

    #[test]
    fn weekly_month_with_full_attendance_pays_all_fridays() {
        // جمعتان في شهر بدوام كامل لكل أسبوع → 2 × 62 = 124
        let days = vec![
            day("2025-06-07", 510, 0, 0),
            day("2025-06-08", 510, 0, 0),
            off_day("2025-06-13"), // الجمعة الأولى
            day("2025-06-14", 510, 0, 0),
            day("2025-06-15", 510, 0, 0),
            off_day("2025-06-20"), // الجمعة الثانية
        ];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, true, &[4], &s_weekly());
        assert_eq!(r.weekend_days_paid, 2);
        assert!((r.weekend_pay - 124.0).abs() < 1e-9, "pay={}", r.weekend_pay);
    }
}
