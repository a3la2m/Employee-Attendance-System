//! تحويل البصمات الخام إلى أيام حضور محسوبة (حضور/غياب/تأخير/انصراف مبكر/إضافي).
use crate::models::{AttendanceDay, PayrollSettings};
use chrono::{NaiveDate, NaiveDateTime};

/// يحوّل نص ISO إلى NaiveDateTime (يتجاهل/يحوّل المنطقة الزمنية للحظة).
fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .ok()
}

/// يبني خريطة: تاريخ (YYYY-MM-DD) → (أول حضور، آخر انصراف) لكل موظف.
/// البصمات تُطوى: أي in/out متتالي يُؤخذ الحدّ الأدنى للأول والقصوى للآخر.
pub fn pair_punches(
    pins_punches: &[(String, String)], // (pin, device_time ISO)
) -> std::collections::HashMap<String, std::collections::BTreeMap<NaiveDateTime, NaiveDateTime>> {
    // pin → sorted map of punch datetime → (نستخدم القيمة للتمييز)
    let mut by_pin: std::collections::HashMap<
        String,
        std::collections::BTreeMap<NaiveDateTime, NaiveDateTime>,
    > = Default::default();
    for (pin, t) in pins_punches {
        if let Some(dt) = parse_dt(t) {
            by_pin
                .entry(pin.clone())
                .or_default()
                .insert(dt, dt);
        }
    }
    by_pin
}

/// يحسب أيام الحضور لموظف واحد من لائحة بصمات مرتّبة زمنياً.
///
/// القواعد:
/// - اليوم يبدأ من work_start وينتهي work_end (مع تجاوز منتصف الليل للورديات الليلية).
/// - first_in = أول بصمة في اليوم، last_out = آخر بصمة.
/// - وقت العمل = (last_out - first_in) - الاستراحة (break_min).
/// - التأخير = max(0, first_in - work_start) بالدقائق.
/// - الإضافي = max(0, وقت العمل - daily_hours).
/// - الانصراف المبكر = max(0, work_end - last_out).
/// - إن لم توجد بصمة: absent (ما لم تكن إجازة).
pub fn compute_days_for_employee(
    employee_id: i64,
    _pin: &str,
    punch_times: &[String], // ISO، مرتّبة تصاعدياً
    leaves: &[(NaiveDate, NaiveDate, String)], // (من، إلى، نوع)
    s: &PayrollSettings,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<AttendanceDay> {
    // اجمع البصمة حسب التاريخ (بتقويم الجهاز المحلي).
    let mut day_map: std::collections::BTreeMap<NaiveDate, Vec<NaiveDateTime>> = Default::default();
    for t in punch_times {
        if let Some(dt) = parse_dt(t) {
            day_map.entry(dt.date()).or_default().push(dt);
        }
    }

    let (sh, sm) = parse_hm(&s.work_start);
    let (eh, em) = parse_hm(&s.work_end);
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        let punches = day_map.get(&d).cloned().unwrap_or_default();
        let on_leave = leaves
            .iter()
            .any(|(f, t, _)| d >= *f && d <= *t);
        let leave_type = leaves
            .iter()
            .find(|(f, t, _)| d >= *f && d <= *t)
            .map(|(_, _, ty)| ty.clone());

        if punches.is_empty() {
            // لا بصمة
            if on_leave {
                out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, false, leave_type, "leave"));
            } else {
                out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, true, None, "absent"));
            }
        } else {
            let first = punches[0];
            let last = punches[punches.len() - 1];

            // وقت البدء المتوقع لهذا اليوم (إن كان العمل يمتد لليوم التالي، احسب الفرق)
            let start_dt = d.and_hms_opt(sh, sm, 0).unwrap();
            let mut end_dt = d.and_hms_opt(eh, em, 0).unwrap();
            if end_dt <= start_dt {
                end_dt += chrono::Duration::days(1); // وردية ليلية
            }

            // وقت العمل بالدقائق ناقص الاستراحة
            let gross = (last - first).num_minutes().max(0);
            let worked = (gross - s.break_min).max(0);

            // التأخير (نأخذ الحد الأدنى من first مقابل البدء)
            let late = (first - start_dt).num_minutes().max(0);

            // الانصراف المبكر (إذا كان آخر خروج قبل الانتهاء)
            let early = (end_dt - last).num_minutes().max(0);
            let early = if early > 0 && last < end_dt { early } else { 0 };

            // الإضافي
            let overtime = (worked - (s.daily_hours * 60.0) as i64).max(0);

            let status = if late > 0 { "late" } else { "present" };
            out.push(day_with(
                employee_id,
                d,
                Some(first),
                Some(last),
                worked,
                late,
                early.max(0),
                overtime,
                false,
                leave_type,
                status,
            ));
        }
        d = d.succ_opt().unwrap();
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn day_with(
    emp: i64,
    date: NaiveDate,
    fin: Option<NaiveDateTime>,
    fout: Option<NaiveDateTime>,
    worked: i64,
    late: i64,
    early: i64,
    ot: i64,
    absent: bool,
    leave: Option<String>,
    status: &str,
) -> AttendanceDay {
    AttendanceDay {
        id: 0,
        employee_id: emp,
        date: date.format("%Y-%m-%d").to_string(),
        first_in: fin.map(|x| x.format("%Y-%m-%dT%H:%M:%S").to_string()),
        last_out: fout.map(|x| x.format("%Y-%m-%dT%H:%M:%S").to_string()),
        worked_min: worked,
        late_min: late,
        early_leave_min: early,
        overtime_min: ot,
        absent,
        leave_type: leave,
        status: status.to_string(),
    }
}

fn parse_hm(s: &str) -> (u32, u32) {
    let mut it = s.split(':');
    let h = it.next().and_then(|x| x.parse().ok()).unwrap_or(9);
    let m = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (h, m)
}

/// يتحقق أن ساعة الجهاز مطابقة للمنطقة الزمنية المحددة (Asia/Jerusalem).
/// يرجع (الساعة الحالية بالمنطقة، فرق الدقائق عن ساعة الجهاز).
pub fn check_device_clock(device_time: &str, tz_name: &str) -> (String, i64) {
    use chrono_tz::Tz;
    let tz: Tz = tz_name.parse().unwrap_or(chrono_tz::Asia::Jerusalem);
    let now_local = chrono::Utc::now().with_timezone(&tz);
    let dev = parse_dt(device_time);
    let now_str = now_local.format("%Y-%m-%dT%H:%M:%S").to_string();
    let diff = match dev {
        Some(d) => {
            // ساعة الجهاز تعرض وقتاً محلياً (جداري) — نقارنه بوقتنا المحلي
            // لا بـ UTC، وإلا ظهر فرق يساوي إزاحة المنطقة (مثل ‑179 دقيقة)
            let now_local_naive = chrono::Utc::now().with_timezone(&tz).naive_local();
            (now_local_naive - d).num_minutes()
        }
        None => 0,
    };
    (now_str, diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// وقت الجهاز المطابق تماماً لوقتنا المحلي = فرق صفر.
    /// (كان -179 دقيقة لأن المقارنة تتم مع UTC بدل الوقت الجداري المحلي.)
    #[test]
    fn device_clock_diff_uses_wall_clock() {
        let local = chrono::Utc::now().with_timezone(&chrono_tz::Asia::Jerusalem);
        let dev = local.format("%Y-%m-%dT%H:%M:%S").to_string();
        let (_, diff) = check_device_clock(&dev, "Asia/Jerusalem");
        assert!(diff.abs() <= 1, "diff={diff} لوقت متطابق يجب أن يكون ~0");
    }

    /// ساعة متأخرة 30 دقيقة تُعطي فرقاً موجباً 30 (المحلي ناقص الجهاز).
    #[test]
    fn device_clock_diff_detects_lag() {
        let local = chrono::Utc::now().with_timezone(&chrono_tz::Asia::Jerusalem);
        let lagged = local - chrono::Duration::minutes(30);
        let dev = lagged.format("%Y-%m-%dT%H:%M:%S").to_string();
        let (_, diff) = check_device_clock(&dev, "Asia/Jerusalem");
        assert!((29..=31).contains(&diff), "diff={diff}");
    }
}
