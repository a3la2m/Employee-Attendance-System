//! تحويل البصمات الخام إلى أيام حضور محسوبة (حضور/غياب/تأخير/انصراف مبكر/إضافي).
use crate::models::{AttendanceDay, PayrollSettings};
use chrono::{Datelike, NaiveDate, NaiveDateTime};

/// يحوّل نص ISO إلى NaiveDateTime (يتجاهل/يحوّل المنطقة الزمنية للحظة).
fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .ok()
}

/// يبني خريطة: تاريخ (YYYY-MM-DD) → (أول حضور، آخر انصراف) لكل موظف.
/// البصمات تُطوى: أي in/out متتالي يُخذ الحدّ الأدنى للأول والقصوى للآخر.
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

/// هل هذه البصمة اتجاه «دخول»؟ (حضور/عودة من الاستراحة/دخول إضافي)
fn is_in_punch(punch_type: i64) -> bool {
    matches!(punch_type, 0 | 3 | 4)
}

/// مهلة الدخول بلا انصراف قبل أن يُصنَّف اليوم «بصمة ناقصة»: 14 ساعة من بصمة
/// الدخول المفتوحة. قبل انقضائها يُعتبر الموظف ما زال في دوامه (حضور).
pub const OPEN_PUNCH_GRACE_MIN: i64 = 14 * 60;

/// يحسب أيام الحضور لموظف واحد من لائحة بصمات مرتّبة زمنياً مع نوع كل بصمة.
///
/// أنواع البصمة كما يسجلها الجهاز (حقل punch في ATTLOG):
/// 0=حضور، 1=خروج، 2=مغادرة استراحة، 3=عودة من الاستراحة، 4=دخول إضافي، 5=خروج إضافي
///
/// قواعد الازدواج (التي طلبها المستخدم):
/// - تسجيل دخول متكرر أثناء فترة مفتوحة يُهمَل (يُحسب **أول** تسجيل دخول).
/// - تسجيل خروج متكرر أثناء فترة مغلقة يُهمَل (يُحسب **أول** تسجيل خروج).
/// - دخول بعد خروج (استراحة أو عودة للعمل الإضافي) يفتح فترة جديدة تُضاف
///   لساعات **اليوم نفسه**.
/// - وقت العمل = مجموع الفترات؛ الاستراحة القياسية (break_min) تُخصم مرة واحدة
///   فقط في الأيام التي لا تحوي بصمات استراحة فعلية (2/3) — فالاستراحة المسجّلة
///   تُحسب ضمن الفترات المفصولة ولا تُخصم مرتين.
/// - اليوم يبدأ من work_start وينتهي work_end (مع تجاوز منتصف الليل للورديات).
/// - التأخير معلومة في عموده فقط (لا حالة «تأخير»): الحالة «حضور»، أو — بعد
///   انتهاء العمل — «بصمة ناقصة»: خروج بلا دخول، أو دخول بلا انصراف مضى عليه
///   14 ساعة (`OPEN_PUNCH_GRACE_MIN`). الدوام الجاري (دخول حديث بلا انصراف)
///   يُعدّ «حضور» لأن الموظف ما زال في عمله.
/// - لا يُخترع وقت انصراف ليوم لم يُسجَّل فيه خروج: يبقى last_out فارغاً.
/// - الإضافي = max(0, العمل − daily_hours).
/// - الانصراف المبكر = max(0, work_end − آخر خروج) لليوم المغلق بانصراف فقط.
/// - إن لم توجد بصمة: إجازة مسجّلة ← عطلة رسمية ← **عطلة أسبوعية تلقائية**
///   (أي يوم من أيام العطلة الأسبوعية المعدّدة في `weekend` — مثل الجمعة)
///   ← غياب. أي أن يوم العطلة الأسبوعية بلا بصمة يُسجَّل مباشرة
///   status="weekend" وليست غياباً.
pub fn compute_days_for_employee(
    employee_id: i64,
    _pin: &str,
    punch_list: &[(String, i64)], // (device_time ISO, punch_type) مرتّبة تصاعدياً
    leaves: &[(NaiveDate, NaiveDate, String, bool)], // (من، إلى، نوع، مدفوعة)
    holidays: &[NaiveDate], // الإجازات الرسمية (الأعياد) — لكل الموظفين
    weekend: &[u32], // أيام العطلة الأسبوعية (0=اثنين … 4=جمعة 5=سبت)
    no_break_dates: &[NaiveDate], // أيام «إلغاء خصم الاستراحة» (زر اليوم في شاشة الحضور)
    s: &PayrollSettings,
    from: NaiveDate,
    to: NaiveDate,
    now: NaiveDateTime, // الوقت المحلي الحالي (للحكم على الدوام الجاري)
) -> Vec<AttendanceDay> {
    // اجمع البصمات حسب التاريخ (بتقويم الجهاز المحلي) مع نوع كل واحدة.
    let mut day_map: std::collections::BTreeMap<NaiveDate, Vec<(NaiveDateTime, i64)>> =
        Default::default();
    for (t, pt) in punch_list {
        if let Some(dt) = parse_dt(t) {
            day_map.entry(dt.date()).or_default().push((dt, *pt));
        }
    }

    let (sh, sm) = parse_hm(&s.work_start);
    let (eh, em) = parse_hm(&s.work_end);
    let mut out = Vec::new();
    let mut d = from;
    while d <= to {
        let punches = day_map.get(&d).cloned().unwrap_or_default();
        let leave_info = leaves
            .iter()
            .find(|(f, t, _, _)| d >= *f && d <= *t)
            .map(|(_, _, ty, paid)| (ty.clone(), *paid));
        let on_holiday = holidays.contains(&d);

        if let Some((ty, paid)) = leave_info {
            // الإجازة تتقدّم على البصمات: يوم مغطّى بإجازة يُعرض إجازةً حتى لو
            // سجّل الموظف بصمة (تحويل يوم حضور إلى إجازة من شاشة الحضور يعمل
            // مباشرة)، ولا يُعدّ غياباً إن لم يُسجّل شيئاً.
            out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, false, Some(ty), "leave", Some(paid)));
        } else if punches.is_empty() {
            // لا بصمة: عطلة رسمية، ثم عطلة أسبوعية، ثم غياب
            if on_holiday {
                out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, false, None, "holiday", None));
            } else if weekend.contains(&d.weekday().num_days_from_monday()) {
                // عطلة أسبوعية تلقائية (الجمعة عند weekend="4"): لا يُسجَّل غياباً
                out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, false, None, "weekend", None));
            } else {
                out.push(day_with(employee_id, d, None, None, 0, 0, 0, 0, true, None, "absent", None));
            }
        } else {
            // الازدواج: دخول يفتح فترة، خروج يغلقها؛ التكرار يُهمَل.
            let has_break = punches.iter().any(|(_, p)| *p == 2 || *p == 3);
            let mut segments: Vec<(NaiveDateTime, NaiveDateTime)> = Vec::new();
            let mut open: Option<NaiveDateTime> = None;
            for (t, p) in &punches {
                if is_in_punch(*p) {
                    if open.is_none() {
                        open = Some(*t);
                    }
                } else if let Some(i) = open.take() {
                    segments.push((i, *t));
                }
            }
            // بقي دخول بلا انصراف؟ لا نخترع وقت انصراف وهمياً: اليوم يُعلَّم
            // «بصمة ناقصة» ويبقى last_out فارغاً حتى يُصحَّح من شاشة الحضور
            // (وهذا ما يعتمد عليه تنبيه «خروج مفقود»).
            let missing_out = open.is_some();
            // أول دخول فعلي: بداية أول فترة، أو الدخول المفتوح، أو أول بصمة دخول.
            let first = segments
                .first()
                .map(|(a, _)| *a)
                .or(open)
                .or_else(|| punches.iter().find(|(_, p)| is_in_punch(*p)).map(|(t, _)| *t));
            // آخر انصراف فعلي: نهاية آخر فترة مغلقة (الخروج المكرر يُهمَل)،
            // وإن لم تُغلق أي فترة (خروج وحده) نعرض آخر بصمة خروج.
            let last = segments.last().map(|(_, b)| *b).or_else(|| {
                punches
                    .iter()
                    .rev()
                    .find(|(_, p)| !is_in_punch(*p))
                    .map(|(t, _)| *t)
            });
            let gross: i64 = segments
                .iter()
                .map(|(a, b)| (*b - *a).num_minutes().max(0))
                .sum();

            // الاستراحة القياسية تُخصم **دائماً** (حتى لو عمل أقل من ساعات
            // الدوام) — إلا إذا سُجّلت استراحة فعلية (2/3) مفصولة بالبصمات،
            // أو أُلغي الخصم من الإعداد «لا تخصم الاستراحة» (deduct_break=0)،
            // أو أُلغي خصمها ليوم بعينه من شاشة الحضور (no_break).
            let day_no_break = no_break_dates.contains(&d);
            let worked = if has_break || day_no_break {
                gross
            } else if s.deduct_break {
                (gross - s.break_min).max(0)
            } else {
                gross
            };

            // وقت البدء المتوقع لهذا اليوم (وردية ليلية: نهاية اليوم التالي)
            let start_dt = d.and_hms_opt(sh, sm, 0).unwrap();
            let mut end_dt = d.and_hms_opt(eh, em, 0).unwrap();
            if end_dt <= start_dt {
                end_dt += chrono::Duration::days(1);
            }

            // التأخير (معلومة فقط — لا حالة مستقلة)
            let late = first
                .map(|f| (f - start_dt).num_minutes().max(0))
                .unwrap_or(0);

            // الانصراف المبكر لا معنى له ليوم مفتوح (لا نعرف متى انصرف فعلاً)
            let early = if missing_out {
                0
            } else {
                last.map(|l| (end_dt - l).num_minutes().max(0)).unwrap_or(0)
            };

            // الإضافي: يبدأ بعد تجاوز الساعات الفعلية (بعد خصم الاستراحة إن
            // كانت مفعّلة) لساعات الدوام الأساسية — العمل قد يكون في أي ساعة
            // فلا حدّ لبداية اليوم.
            let effective_break = if s.deduct_break && !has_break && !day_no_break {
                s.break_min
            } else {
                0
            };
            let ot_threshold = ((s.daily_hours * 60.0) as i64 - effective_break).max(0);
            let overtime = (worked - ot_threshold).max(0);

            // بلا دخول (خروج وحده) ← بصمة ناقصة مباشرة، فلا معنى لانتظار.
            // دخول بلا انصراف ← الموظف في دوامه ما دامت مهلته لم تنقضِ
            // (14 ساعة من بصمة الدخول المفتوحة) فيُعدّ «حضور»، وبعدها يصبح
            // «بصمة ناقصة» ليتدخّل المسؤول يدوياً.
            let status = if first.is_none() {
                "incomplete"
            } else if let Some(open_in) = open {
                if (now - open_in).num_minutes() >= OPEN_PUNCH_GRACE_MIN {
                    "incomplete"
                } else {
                    "present"
                }
            } else {
                "present"
            };
            out.push(day_with(
                employee_id,
                d,
                first,
                last,
                worked,
                late,
                early.max(0),
                overtime,
                false,
                None,
                status,
                None,
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
    leave_paid: Option<bool>,
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
        leave_paid,
        manual: false,
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
            // ساعة الجهاز تعرض وقتاً محلياً (جدولي) — نقارنه بوقتنا المحلي
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

    /// إعدادات افتراضية للاختبارات: دوام 09:00–17:30، استراحة 30 د.
    fn ps() -> PayrollSettings {
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
            weekend_pay_mode: "weekly".into(),
            work_start: "09:00".into(),
            work_end: "17:30".into(),
        }
    }

    /// زمن الحكم الافتراضي للاختبارات: بعد كل تواريخ البصمات المستخدمة هنا
    /// — أي أن أي دخول مفتوح يُعدّ منتهي المهل (بصمة ناقصة).
    fn test_now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap().and_hms_opt(0, 0, 0).unwrap()
    }

    fn day(punches: &[(&str, i64)]) -> AttendanceDay {
        day_at(test_now(), punches)
    }

    /// يحسب يوماً واحداً (2026-10-16) بزمن حكم محدد.
    fn day_at(now: NaiveDateTime, punches: &[(&str, i64)]) -> AttendanceDay {
        let list: Vec<(String, i64)> = punches
            .iter()
            .map(|(t, p)| (t.to_string(), *p))
            .collect();
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        compute_days_for_employee(1, "9", &list, &[], &[], &[4], &[], &ps(), d, d, now)
            .into_iter()
            .next()
            .unwrap()
    }

    /// يحسب يوماً واحداً مع إجازات/أعياد محددة.
    fn day_with_ctx(
        punches: &[(&str, i64)],
        leaves: &[(NaiveDate, NaiveDate, String, bool)],
        holidays: &[NaiveDate],
    ) -> AttendanceDay {
        let list: Vec<(String, i64)> = punches
            .iter()
            .map(|(t, p)| (t.to_string(), *p))
            .collect();
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        compute_days_for_employee(
            1,
            "9",
            &list,
            leaves,
            holidays,
            &[4],
            &[],
            &ps(),
            d,
            d,
            test_now(),
        )
        .into_iter()
        .next()
        .unwrap()
    }

    /// يوم عطلة أسبوعية (الجمعة) بلا بصمة ← status="weekend" وليس غياباً.
    #[test]
    fn friday_without_punches_is_weekend_not_absent() {
        let d = day_with_ctx(&[], &[], &[]); // 2026-10-16 جمعة
        assert_eq!(d.status, "weekend");
        assert!(!d.absent, "الجمعة لا تُسجَّل غياباً");
        assert!(d.first_in.is_none());
    }

    /// يوم دوام (الاثنين) بلا بصمة وبدون إجازة ← يبقى غياباً.
    #[test]
    fn workday_without_punches_is_still_absent() {
        let list: Vec<(String, i64)> = vec![];
        let mon = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(); // الاثنين
        let d = compute_days_for_employee(1, "9", &list, &[], &[], &[4], &[], &ps(), mon, mon, test_now())
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(d.status, "absent");
        assert!(d.absent);
    }

    /// زر «إلغاء خصم الاستراحة» ليوم بعينه: اليوم يعود لدقائقه الكاملة
    /// (630 بدل 600)، والإضافي لا يتغيّر (عتبة الخصم ينزاح معه معاً).
    #[test]
    fn no_break_date_skips_break_deduction() {
        let punches: Vec<(String, i64)> = vec![
            ("2026-10-19T06:00:00".into(), 0),
            ("2026-10-19T16:30:00".into(), 1),
        ];
        let d = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(); // الاثنين
        let normal =
            compute_days_for_employee(1, "9", &punches, &[], &[], &[4], &[], &ps(), d, d, test_now())
                .into_iter()
                .next()
                .unwrap();
        let free =
            compute_days_for_employee(1, "9", &punches, &[], &[], &[4], &[d], &ps(), d, d, test_now())
                .into_iter()
                .next()
                .unwrap();
        assert_eq!(normal.worked_min, 600, "بالاستراحة: 630 − 30");
        assert_eq!(free.worked_min, 630, "بلا استراحة: كامل");
        assert_eq!(
            free.overtime_min, normal.overtime_min,
            "الإضافي محسوب من العتبة نفسها فلا يتأثر"
        );
    }

    /// الإجازة تتقدّم على البصمات: يوم له بصمات لكن تغطية إجازة ← إجازة
    /// (أساس تحويل يوم حضور إلى إجازة من شاشة الحضور).
    #[test]
    fn leave_overrides_punches() {
        let punches: Vec<(String, i64)> = vec![
            ("2026-10-19T06:00:00".into(), 0),
            ("2026-10-19T16:30:00".into(), 1),
        ];
        let d = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        for (ty, paid, want_paid) in [("annual", true, true), ("sick", true, true), ("annual", false, false)] {
            let leaves = [(d, d, ty.to_string(), paid)];
            let day = compute_days_for_employee(
                1, "9", &punches, &leaves, &[], &[4], &[], &ps(), d, d, test_now(),
            )
            .into_iter()
            .next()
            .unwrap();
            assert_eq!(day.status, "leave", "{ty}/{paid} يجب أن يتقدّم على البصمات");
            assert_eq!(day.leave_paid, Some(want_paid));
            assert!(day.first_in.is_none(), "يوم الإجازة بلا أوقات حضور");
        }
    }

    /// يوم عادي: حضور 09:00 خروج 17:30 → 510 د ناقص 30 استراحة = 480.
    #[test]
    fn normal_day_basic_pairing() {
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T17:30:00", 1),
        ]);
        assert_eq!(d.worked_min, 480);
        assert_eq!(d.late_min, 0);
        assert_eq!(d.status, "present");
        assert_eq!(d.first_in.as_deref(), Some("2026-10-16T09:00:00"));
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T17:30:00"));
    }

    /// دخول مكرر بلا خروج بينهما → يُحسب **أول** تسجيل دخول فقط.
    #[test]
    fn duplicate_ins_count_first() {
        let d = day(&[
            ("2026-10-16T08:55:00", 0),
            ("2026-10-16T09:10:00", 0), // مكرر يُهمَل
            ("2026-10-16T17:30:00", 1),
        ]);
        assert_eq!(d.first_in.as_deref(), Some("2026-10-16T08:55:00"));
        assert_eq!(d.worked_min, 485, "08:55→17:30 = 515 د ناقص 30 استراحة");
    }

    /// خروج مكرر → يُحسب **أول** تسجيل خروج (لا يمتد العمل بالتكرار).
    #[test]
    fn duplicate_outs_count_first() {
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T17:30:00", 1),
            ("2026-10-16T17:45:00", 1), // خروج ثانٍ يُهمَل
        ]);
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T17:30:00"));
        assert_eq!(d.worked_min, 480);
    }

    /// أنهى دوامه وعاد (إضافي): الفترة الثانية تُضاف لساعات اليوم نفسه.
    #[test]
    fn overtime_return_counts_same_day() {
        let d = day(&[
            ("2026-10-16T06:24:25", 0),
            ("2026-10-16T11:16:25", 1),
            ("2026-10-16T14:00:00", 0), // عاد للعمل الإضافي
            ("2026-10-16T16:00:00", 1),
        ]);
        // الفترتان: 4:52 + 2:00 = 6:52 = 412 د، ناقص استراحة 30 = 382
        assert_eq!(d.worked_min, 412 - 30);
        assert_eq!(d.first_in.as_deref(), Some("2026-10-16T06:24:25"));
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T16:00:00"));
    }

    /// مغادرة استراحة (2) وعودة (3): الفترتان تُحسبان والاستراحة لا تُخصم مرتين.
    #[test]
    fn break_out_and_back_split_segments() {
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T13:00:00", 2), // مغادرة استراحة
            ("2026-10-16T13:30:00", 3), // عودة من الاستراحة
            ("2026-10-16T17:30:00", 1),
        ]);
        // 4:00 + 4:00 = 480 د — بلا خصم إضافي (الاستراحة مسجّلة فعلياً)
        assert_eq!(d.worked_min, 480);
        assert_eq!(d.first_in.as_deref(), Some("2026-10-16T09:00:00"));
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T17:30:00"));
    }

    /// بصمات الاستراحة المتساوية عند حدود الفترات لا تكسر الازدواج.
    #[test]
    fn break_punches_do_not_create_bogus_segments() {
        let d = day(&[
            ("2026-10-16T08:58:00", 0),
            ("2026-10-16T12:30:00", 2),
            ("2026-10-16T13:00:00", 3),
            ("2026-10-16T17:35:00", 1),
        ]);
        // 08:58→12:30 = 212 د + 13:00→17:35 = 275 د
        assert_eq!(d.worked_min, 212 + 275);
        assert!(d.late_min == 0);
    }

    /// الاستراحة تُخصم دائماً حتى في اليوم القصير (7 ساعات عمل ← 6.5 محسوبة).
    #[test]
    fn break_deducted_even_on_short_day() {
        // 09:00→16:00 = 420 د (7 س) ناقص 30 استراحة = 390
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T16:00:00", 1),
        ]);
        assert_eq!(d.worked_min, 390);
    }

    /// زر «لا تخصم الاستراحة» (deduct_break=0): يُحتسب اليوم كاملاً بلا خصم.
    #[test]
    fn deduct_break_disabled_keeps_full_gross() {
        let mut s = ps();
        s.deduct_break = false;
        let list: Vec<(String, i64)> = vec![
            ("2026-10-16T09:00:00".into(), 0),
            ("2026-10-16T16:00:00".into(), 1),
        ];
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let day = compute_days_for_employee(1, "9", &list, &[], &[], &[4], &[], &s, d, d, test_now())
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(day.worked_min, 420, "بلا خصم الاستراحة: 7 ساعات كاملة");
        // حد الإضافي يرتفع أيضاً (لا خصم ← العتبة = 8.5 س صافية)
        assert_eq!(day.overtime_min, 0);
    }

    /// بصمات الاستراحة الفعلية تُحسب كدقائق استراحة مهما كان إعداد الخصم.
    #[test]
    fn recorded_break_split_persists_when_deduct_disabled() {
        let mut s = ps();
        s.deduct_break = false;
        let list: Vec<(String, i64)> = vec![
            ("2026-10-16T09:00:00".into(), 0),
            ("2026-10-16T13:00:00".into(), 2),
            ("2026-10-16T13:30:00".into(), 3),
            ("2026-10-16T17:30:00".into(), 1),
        ];
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let day = compute_days_for_employee(1, "9", &list, &[], &[], &[4], &[], &s, d, d, test_now())
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(day.worked_min, 480, "الاستراحة المسجّلة تُفصل الفترتين فعلاً");
    }

    /// الإضافي يبدأ بعد تجاوز الساعات الفعلية (بعد الاستراحة) لحد الدوام: 8.5 − 0.5 = 8 س.
    #[test]
    fn overtime_starts_after_net_daily_hours() {
        // مناوبة 09:00–18:00 = 540 د ناقص 30 استراحة = 510 د فعلية → 30 د إضافي
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T18:00:00", 1),
        ]);
        assert_eq!(d.worked_min, 510);
        assert_eq!(d.overtime_min, 30);
        // مناوبة 08:00–16:30 = 510 د ناقص 30 = 480 د فعلية = حد الدوام تماماً → بلا إضافي
        let d2 = day(&[
            ("2026-10-16T08:00:00", 0),
            ("2026-10-16T16:30:00", 1),
        ]);
        assert_eq!(d2.worked_min, 480);
        assert_eq!(d2.overtime_min, 0);
    }

    /// يوم بلا بصمة ضمن إجازة سنوية مدفوعة → حالة "leave" ومقصود دفعها.
    #[test]
    fn paid_leave_day_marked() {
        let d0 = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let leaves = vec![(d0, d0, "annual".to_string(), true)];
        let d = day_with_ctx(&[], &leaves, &[]);
        assert_eq!(d.status, "leave");
        assert_eq!(d.leave_type.as_deref(), Some("annual"));
        assert_eq!(d.leave_paid, Some(true));
        assert!(!d.absent);
    }

    /// إجازة غير مدفوعة → leave_paid = Some(false).
    #[test]
    fn unpaid_leave_day_marked() {
        let d0 = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let leaves = vec![(d0, d0, "sick".to_string(), false)];
        let d = day_with_ctx(&[], &leaves, &[]);
        assert_eq!(d.status, "leave");
        assert_eq!(d.leave_paid, Some(false));
        assert!(!d.absent);
    }

    /// يوم عطلة رسمية بلا بصمة → حالة "holiday" وليس غياباً.
    #[test]
    fn official_holiday_day_marked() {
        let d0 = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let d = day_with_ctx(&[], &[], &[d0]);
        assert_eq!(d.status, "holiday");
        assert!(!d.absent);
    }

    /// بصمات تدخل يوم عطلة رسمية → يُعامَل حضوراً عادياً (ساعاته مدفوعة).
    #[test]
    fn punches_on_holiday_are_present() {
        let d0 = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let d = day_with_ctx(
            &[
                ("2026-10-16T09:00:00", 0),
                ("2026-10-16T17:30:00", 1),
            ],
            &[],
            &[d0],
        );
        assert_eq!(d.status, "present");
        assert_eq!(d.worked_min, 480);
    }

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

    /// دوام جارٍ: بصمة دخول حديثة بلا انصراف (مضى عليها أقل من 14 ساعة)
    /// ← «حضور» وليست «بصمة ناقصة»، لأن الموظف ما زال في عمله.
    #[test]
    fn ongoing_shift_with_recent_open_punch_is_present() {
        let now = NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(21, 0, 0)
            .unwrap(); // 13:10 + 7:50
        let d = day_at(now, &[("2026-10-16T13:10:00", 0)]);
        assert_eq!(d.status, "present", "الدوام لم ينتهِ بعد");
        assert_eq!(d.last_out, None, "لا يُخترع وقت انصراف");
        assert_eq!(d.worked_min, 0, "لا انصراف ⇒ لا ساعات محسوبة بعد");
        assert_eq!(d.early_leave_min, 0, "لا انصراف مبكر لدوام جارٍ");
        assert!(!d.absent);
    }

    /// حدّ المهلة: قبل 14 ساعة من الدخول المفتوح ← «حضور»، وعندها ← «بصمة ناقصة».
    #[test]
    fn open_punch_flips_to_incomplete_at_grace_boundary() {
        let base = NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(13, 10, 0)
            .unwrap();
        let punches = [("2026-10-16T13:10:00", 0)];
        let before = day_at(base + chrono::Duration::minutes(OPEN_PUNCH_GRACE_MIN - 1), &punches);
        assert_eq!(before.status, "present", "دقيقة قبل انتهاء المهلة");
        let at = day_at(base + chrono::Duration::minutes(OPEN_PUNCH_GRACE_MIN), &punches);
        assert_eq!(at.status, "incomplete", "عند انتهاء المهلة");
    }

    /// بصمة دخول وحيدة بلا انصراف: لا وقت انصراف وهمي، صفر ساعات،
    /// والحالة «بصمة ناقصة» (لا «تأخير»).
    #[test]
    fn single_in_punch_is_incomplete_without_phantom_out() {
        let d = day(&[("2026-10-16T13:10:00", 0)]);
        assert_eq!(d.first_in.as_deref(), Some("2026-10-16T13:10:00"));
        assert_eq!(d.last_out, None, "لا يُخترع وقت انصراف من وقت الدخول");
        assert_eq!(d.worked_min, 0);
        assert_eq!(d.early_leave_min, 0, "لا انصراف مبكر وهمي");
        assert_eq!(d.late_min, 250, "13:10 − 09:00 = 250 دقيقة");
        assert_eq!(d.status, "incomplete");
        assert!(!d.absent, "وجود بصمة لا يعني غياباً");
    }

    /// بصمة خروج وحيدة بلا دخول: يُحفظ وقت الخروج الحقيقي مع «بصمة ناقصة»،
    /// ويُحتسب الانصراف المبكر من وقت الخروج الحقيقي (بلا ساعات عمل).
    #[test]
    fn single_out_punch_keeps_out_time_and_is_incomplete() {
        let d = day(&[("2026-10-16T15:00:00", 1)]);
        assert_eq!(d.first_in, None);
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T15:00:00"));
        assert_eq!(d.worked_min, 0);
        assert_eq!(d.late_min, 0);
        assert_eq!(d.early_leave_min, 150, "17:30 − 15:00 = 150 دقيقة");
        assert_eq!(d.status, "incomplete");
    }

    /// نسي الخروج بعد فترة مغلقة: تُحفظ الفترة الحقيقية، ولا يُحتسب
    /// انصراف مبكر وهمي، والحالة «بصمة ناقصة».
    #[test]
    fn forgot_out_keeps_closed_segment_without_fake_early_leave() {
        let d = day(&[
            ("2026-10-16T09:00:00", 0),
            ("2026-10-16T12:00:00", 1),
            ("2026-10-16T13:00:00", 0), // دخل ولم يسجّل خروجاً بعدها
        ]);
        assert_eq!(d.worked_min, 150, "3 ساعات − استراحة 30 د");
        assert_eq!(d.last_out.as_deref(), Some("2026-10-16T12:00:00"));
        assert_eq!(d.early_leave_min, 0);
        assert_eq!(d.status, "incomplete");
    }

    /// التأخير معلومة في عموده فقط — يوم متأخر ومغلق يبقى حالته «حضور».
    #[test]
    fn late_day_keeps_present_status() {
        let d = day(&[
            ("2026-10-16T10:00:00", 0),
            ("2026-10-16T18:00:00", 1),
        ]);
        assert_eq!(d.late_min, 60);
        assert_eq!(d.status, "present");
    }
}
