import { describe, it, expect } from "vitest";
import {
  leaveLabel,
  dayStatusInfo,
  dayStatusText,
  workedOnHoliday,
} from "../labels";

describe("leaveLabel", () => {
  it("يسمّي الأنواع الثلاثة بالعربية", () => {
    expect(leaveLabel("annual", true)).toBe("إجازة سنوية");
    expect(leaveLabel("sick", true)).toBe("إجازة مرضية");
    expect(leaveLabel("comp", true)).toBe("عطلة تعويضية (يوم بدل)");
  });
  it("يضيف (غير مدفوعة) عند عدم الدفع", () => {
    expect(leaveLabel("annual", false)).toBe("إجازة سنوية (غير مدفوعة)");
    expect(leaveLabel("comp", false)).toBe(
      "عطلة تعويضية (يوم بدل) (غير مدفوعة)",
    );
  });
  it("يتعامل مع نوع مجهول", () => {
    expect(leaveLabel(null, true)).toBe("إجازة");
  });
});

describe("dayStatusInfo", () => {
  it("يوم البدل يُعرض عطلة تعويضية بلون مميّز", () => {
    const [txt, cls] = dayStatusInfo({
      status: "leave",
      leave_type: "comp",
      leave_paid: true,
    });
    expect(txt).toBe("عطلة تعويضية (يوم بدل)");
    expect(cls).toContain("fuchsia");
  });
  it("الإجازة السنوية غير المدفوعة", () => {
    expect(
      dayStatusInfo({ status: "leave", leave_type: "annual", leave_paid: false })[0],
    ).toBe("إجازة سنوية (غير مدفوعة)");
  });
  it("حضور قيد العمل بلا انصراف", () => {
    expect(
      dayStatusText({ status: "present", first_in: "2026-10-19T06:00:00", last_out: null }),
    ).toBe("حضور (قيد العمل)");
    expect(
      dayStatusText({
        status: "present",
        first_in: "2026-10-19T06:00:00",
        last_out: "2026-10-19T16:30:00",
      }),
    ).toBe("حضور");
  });
  it("بصمة ناقصة وغياب وعطلة رسمية", () => {
    expect(dayStatusText({ status: "incomplete" })).toBe("بصمة ناقصة");
    expect(dayStatusText({ status: "absent", absent: true })).toBe("غياب");
    expect(dayStatusText({ status: "holiday" })).toBe("عطلة رسمية");
  });
});

describe("workedOnHoliday", () => {
  it("صحيح فقط ليوم عطلة رسمية فيه بصمة دخول ودوام", () => {
    expect(
      workedOnHoliday({
        holiday: true,
        status: "present",
        first_in: "2026-10-26T06:00:00",
      }),
    ).toBe(true);
    // بلا بصمة
    expect(workedOnHoliday({ holiday: true, status: "holiday", first_in: null })).toBe(
      false,
    );
    // يوم عادي ليس عطلة
    expect(
      workedOnHoliday({
        holiday: false,
        status: "present",
        first_in: "2026-10-19T06:00:00",
      }),
    ).toBe(false);
    // إجازة على تاريخ عطلة (لا دوام فعلي)
    expect(
      workedOnHoliday({
        holiday: true,
        status: "leave",
        first_in: "2026-10-26T06:00:00",
      }),
    ).toBe(false);
  });
});
