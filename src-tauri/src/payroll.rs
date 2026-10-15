//! محرك حساب الرواتب — المعادلات كما في المتطلبات، قابلة للتعديل.
//!
//! القواعد:
//! 1. وقت الدوام الفعلي = (الانصراف - الحضور) - فترة الخصم (استراحة).
//! 2. الأجر الأساسي = الساعات الفعلية × سعر الساعة.
//! 3. البدلات = البدلات اليومية × أيام الحضور.
//! 4. الخصومات = (ساعات التأخير + ساعات الغياب) × سعر الساعة.
//! 5. صافي الراتب = الأساسي + البدلات - الخصومات.
//! 6. العمل الإضافي: أي تجاوز لـ (ساعات العمل الأساسية) يُضاف بمضاعف.

use crate::models::{AttendanceDay, PayrollResult, PayrollSettings};
use chrono::{Datelike, NaiveDate};

/// يحسب راتب شهر (period بصيغة "YYYY-MM") من قائمة أيام الحضور المحسوبة مسبقاً.
///
/// `days` = كل أيام attendance_days الخاص بالموظف خلال الفترة.
/// `expected_workdays` = عدد أيام العمل المتوقعة في الشهر (للخصم بسبب الغياب).
pub fn compute_payroll(
    emp_id: i64,
    emp_name: &str,
    pin: Option<String>,
    period: &str,
    days: &[AttendanceDay],
    expected_workdays: i64,
    s: &PayrollSettings,
) -> PayrollResult {
    let mut days_present: i64 = 0;
    let mut total_worked_min: i64 = 0;
    let mut overtime_min: i64 = 0;
    let mut late_min: i64 = 0;

    for d in days {
        if d.status == "leave" {
            continue; // إجازة لا تُحسب حضوراً ولا غياباً
        }
        if d.absent || d.first_in.is_none() {
            continue;
        }
        days_present += 1;
        total_worked_min += d.worked_min;
        overtime_min += d.overtime_min;
        late_min += d.late_min;
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

    // 4) الخصومات
    //    أ) التأخير: دقائق التأخير ÷ 60 × سعر الساعة (بعد مرور المهلة)
    let effective_late = (late_min - s.grace_late_min * days_present).max(0);
    let late_deduction = (effective_late as f64 / 60.0) * s.hourly_rate;
    //    ب) الغياب: أيام الغياب المتوقعة × ساعات العمل الأساسية × سعر الساعة
    let absent_days = (expected_workdays - days_present).max(0);
    let absence_deduction =
        absent_days as f64 * s.daily_hours * s.hourly_rate;

    // 5) الصافي
    let net_pay = base_pay + overtime_pay + allowances - late_deduction - absence_deduction;

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
        late_deduction,
        absence_deduction,
        net_pay,
    }
}

/// عدد أيام العمل المتوقعة في شهر "YYYY-MM".
///
/// `weekend` = أيام العطلة أسبوعياً (0=اثنين … 4=جمعة 5=سبت، بحسب chrono
/// num_days_from_monday). الافتراضي للمنطقة العربية/الإسرائيلية: الجمعة والسبت.
pub fn expected_workdays_in_month(period: &str, weekend: &[u32]) -> i64 {
    let Some((y, m)) = period.split_once('-') else { return 0 };
    let (Ok(y), Ok(m)) = (y.parse::<i32>(), m.parse::<u32>()) else { return 0 };
    let mut d = match NaiveDate::from_ymd_opt(y, m, 1) {
        Some(d) => d,
        None => return 0,
    };
    let mut count = 0;
    while d.month() == m {
        let wd = d.weekday().num_days_from_monday();
        if !weekend.contains(&wd) {
            count += 1;
        }
        d = match d.succ_opt() {
            Some(nd) => nd,
            None => break,
        };
    }
    count
}

/// يحوّل إعداد "weekend_days" (مثال "4,5") إلى مصفوفة أيام، بافتراضي الجمعة+السبت.
pub fn parse_weekend(s: Option<&str>) -> Vec<u32> {
    match s {
        Some(v) if !v.trim().is_empty() => v
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect(),
        _ => vec![4, 5],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> PayrollSettings {
        PayrollSettings {
            daily_hours: 8.5,
            break_min: 30,
            hourly_rate: 10.0,
            meal_allow: 7.0,
            transport_allow: 5.0,
            overtime_after_hours: 8.5,
            overtime_rate_mult: 1.5,
            grace_late_min: 0,
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
            status: if late > 0 { "late" } else { "present" }.into(),
        }
    }

    #[test]
    fn normal_day_basic() {
        // يوم كامل 8.5 سا: 8.5*10 = 85 + بدلات 12 = 97
        // متوقّع = 1 حضور فقط (لا غياب في هذا الاختبار)
        let days = vec![day("2025-06-01", 510, 0, 0)];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, 1, &s());
        assert_eq!(r.days_present, 1);
        assert!((r.total_worked_hours - 8.5).abs() < 1e-9);
        assert!((r.base_pay - 85.0).abs() < 1e-9);
        assert!((r.allowances - 12.0).abs() < 1e-9);
        assert!((r.late_deduction).abs() < 1e-9);
        assert!((r.absence_deduction).abs() < 1e-9);
        assert!((r.net_pay - 97.0).abs() < 1e-9);
    }

    #[test]
    fn overtime_pays_multiplier() {
        // عادي 8.5 سا (510 د) + 45 د إضافي = 555 د إجمالي
        // base = 8.5*10=85 + إضافي 0.75*10*1.5=11.25 -> 96.25
        let days = vec![day("2025-06-02", 555, 0, 45)];
        let r = compute_payroll(1, "أحمد", None, "2025-06", &days, 1, &s());
        assert_eq!(r.overtime_hours, 0.75);
        assert!((r.total_worked_hours - 8.5).abs() < 1e-9);
        assert!((r.base_pay - (85.0 + 11.25)).abs() < 1e-9);
        // net = base(96.25) + allowances(12) - 0
        assert!((r.net_pay - 108.25).abs() < 1e-9);
    }

    #[test]
    fn late_deduction() {
        // تأخير 30 دقيقة = 0.5 * 10 = 5 خصم؛ حضور 8 سا (480 د)
        let days = vec![day("2025-06-03", 480, 30, 0)];
        let r = compute_payroll(1, "سارة", None, "2025-06", &days, 1, &s());
        assert!((r.late_deduction - 5.0).abs() < 1e-9);
        // base = 8.0*10=80 + allow(12) - 5 = 87
        assert!((r.net_pay - 87.0).abs() < 1e-9);
    }

    #[test]
    fn absence_deduction() {
        // حضور يوم واحد من 22 متوقعة = 21 غياب * 8.5 * 10
        let days = vec![day("2025-06-04", 510, 0, 0)];
        let r = compute_payroll(1, "خالد", None, "2025-06", &days, 22, &s());
        assert_eq!(r.days_present, 1);
        assert!((r.absence_deduction - 21.0 * 8.5 * 10.0).abs() < 1e-6);
        // net = 85 + 12 - 1785 = -1688
        assert!((r.net_pay - (85.0 + 12.0 - 21.0 * 8.5 * 10.0)).abs() < 1e-6);
    }

    #[test]
    fn grace_period_waives_late() {
        // مهلة 15 دقيقة: تأخير 10 دقائق لا يُخصم منه شيء
        let mut st = s();
        st.grace_late_min = 15;
        let days = vec![day("2025-06-05", 500, 10, 0)];
        let r = compute_payroll(1, "رنا", None, "2025-06", &days, 1, &st);
        assert_eq!(r.late_deduction, 0.0);
    }

    #[test]
    fn weekend_excludes_friday_and_saturday() {
        // يونيو 2025 = 30 يوماً، منها 4 جمعات + 4 سبتات = 8 عطل -> 22 يوم عمل
        let wk = parse_weekend(Some("4,5"));
        assert_eq!(expected_workdays_in_month("2025-06", &wk), 22);
        // بعطلة الجمعة فقط (افتراضي قديم): 26
        let fri = parse_weekend(Some("4"));
        assert_eq!(expected_workdays_in_month("2025-06", &fri), 26);
        // بدون أي عطلة: 30
        assert_eq!(expected_workdays_in_month("2025-06", &[]), 30);
    }
}
