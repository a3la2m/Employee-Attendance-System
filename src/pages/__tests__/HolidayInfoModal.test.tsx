import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";

// نموذج أوامر Tauri: لا حاجة للربط الحقيقي في هذا الاختبار
vi.mock("../../lib/api", () => ({}));

import { HolidayInfoModal } from "../Attendance";

const noop = () => {};

describe("HolidayInfoModal — شرح تعويض دوام العطلة الرسمية", () => {
  it("الخيار النقدي: يشرح المعادلة (62 + الساعات × 1.5)", () => {
    render(
      <HolidayInfoModal
        row={{ name: "خليل", worked_min: 600, holiday_work_choice: null }}
        date="2026-10-26"
        weekendAllow={62}
        onClose={noop}
        onAssign={noop}
      />,
    );
    // فقرة النقدي الفريدة (لا تظهر في خيار البدل)
    expect(screen.getByText("يُدفع للموظف في راتبه:", { exact: false })).toBeTruthy();
    expect(screen.getByText("10.00 سا", { exact: false })).toBeTruthy();
  });

  it("خيار البدل غير المسند: يحثّ على الإسناد ويظهر زر الإسناد", () => {
    render(
      <HolidayInfoModal
        row={{ name: "خليل", worked_min: 600, holiday_work_choice: "comp" }}
        date="2026-10-26"
        weekendAllow={62}
        onClose={noop}
        onAssign={noop}
      />,
    );
    expect(screen.getByText("لم يُسند بعد", { exact: false })).toBeTruthy();
    expect(screen.getByText("🎁 إسناد يوم البدل")).toBeTruthy();
  });
  it("خيار البدل المسند: يذكر تاريخ يوم البدل ولا يعرض زر الإسناد", () => {
    render(
      <HolidayInfoModal
        row={{
          name: "خليل",
          worked_min: 600,
          holiday_work_choice: "comp",
          comp_off: "2026-11-03",
        }}
        date="2026-10-26"
        weekendAllow={62}
        onClose={noop}
        onAssign={noop}
      />,
    );
    expect(screen.getByText("أُسند يوم البدل", { exact: false })).toBeTruthy();
    expect(screen.getByText("2026-11-03", { exact: false })).toBeTruthy();
    expect(screen.queryByText("🎁 إسناد يوم البدل")).toBeNull();
  });

  it("يوم عطلة تعويضية (بدل): يوضّح أنه بدل عن عطلة ويعرض العطلة المعوّضة", () => {
    render(
      <HolidayInfoModal
        row={{
          name: "خليل",
          worked_min: 0,
          status: "leave",
          leave_type: "comp",
          comp_for: "2026-10-26",
        }}
        date="2026-11-03"
        weekendAllow={62}
        onClose={noop}
        onAssign={noop}
      />,
    );
    expect(screen.getByText("بدلاً من دوامه في عطلة", { exact: false })).toBeTruthy();
    expect(screen.getByText("2026-10-26", { exact: false })).toBeTruthy();
  });
});
