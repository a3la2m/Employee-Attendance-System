import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// نموذج أوامر Tauri: نستبدل الربط بمحاكيات
vi.mock("../../lib/api", () => ({
  setDayManual: vi.fn().mockResolvedValue({}),
  setDayFlags: vi.fn().mockResolvedValue({}),
}));

import { EditDayModal } from "../Attendance";
import { setDayManual, setDayFlags } from "../../lib/api";
import type { DailyRow } from "../../lib/types";

const row: DailyRow = {
  id: 1,
  employee_id: 7,
  name: "موظف تجريبي",
  pin: "7",
  first_in: "2026-10-19T06:00:00",
  last_out: "2026-10-19T16:30:00",
  worked_min: 600,
  late_min: 0,
  early_leave_min: 0,
  overtime_min: 0,
  absent: false,
  status: "present",
  manual: false,
};

const noop = () => {};

const renderModal = (balance = 5) =>
  render(
    <EditDayModal
      row={row}
      date="2026-10-19"
      balance={balance}
      toast={noop}
      onClose={noop}
      onSaved={noop}
    />,
  );

beforeEach(() => {
  vi.clearAllMocks();
});

describe("EditDayModal — تحويل اليوم إلى إجازة", () => {
  it("زر المرضية يحوّل اليوم بوضع sick (بلا خصم)", async () => {
    renderModal(5);
    await userEvent.click(screen.getByTitle(/إجازة مرضية/));
    expect(setDayManual).toHaveBeenCalledWith(7, "2026-10-19", "sick", null, null);
  });

  it("زر بدون أجر يحوّل اليوم بوضع unpaid", async () => {
    renderModal(5);
    await userEvent.click(screen.getByTitle(/إجازة بدون أجر/));
    expect(setDayManual).toHaveBeenCalledWith(7, "2026-10-19", "unpaid", null, null);
  });

  it("زر السنوية يعمل عند توفّر الرصيد", async () => {
    renderModal(2);
    await userEvent.click(screen.getByTitle(/إجازة سنوية/));
    expect(setDayManual).toHaveBeenCalledWith(7, "2026-10-19", "annual", null, null);
  });

  it("السنوية معطّلة عند نفاد الرصيد ولا تُستدعى", async () => {
    renderModal(0);
    const btn = screen.getByTitle(/إجازة سنوية/);
    expect(btn).toBeDisabled();
    await userEvent.click(btn);
    expect(setDayManual).not.toHaveBeenCalled();
  });
});

describe("EditDayModal — أزرار اليوم (الاستراحة والفطور)", () => {
  it("زر إلغاء خصم الاستراحة يستدعي setDayFlags بناءً no_break", async () => {
    renderModal();
    await userEvent.click(screen.getByTitle(/لا تخصم الاستراحة/));
    expect(setDayFlags).toHaveBeenCalledWith(7, "2026-10-19", true, false);
  });

  it("زر إزالة بدل الفطور يستدعي setDayFlags بناءً no_meal", async () => {
    renderModal();
    await userEvent.click(screen.getByTitle(/استبعاد بدل الفطور/));
    expect(setDayFlags).toHaveBeenCalledWith(7, "2026-10-19", false, true);
  });

  it("الضغط مرة ثانية يعكس العلم (إعادة خصم الاستراحة)", async () => {
    renderModal();
    const btn = screen.getByTitle(/لا تخصم الاستراحة/);
    await userEvent.click(btn);
    await userEvent.click(btn);
    expect(setDayFlags).toHaveBeenLastCalledWith(7, "2026-10-19", false, false);
  });
});

describe("EditDayModal — الوصول", () => {
  it("النوافذ الحوارية تُفتح كـ role=dialog (عبر Modal الموحّد)", () => {
    renderModal();
    expect(screen.getByRole("dialog")).toHaveAttribute("aria-modal", "true");
  });

  it("الوضع الافتراضي هو «حاضر» مع حقلَي الوقت", () => {
    renderModal();
    expect(screen.getByLabelText("وقت الحضور")).toHaveValue("06:00");
    expect(screen.getByLabelText("وقت الانصراف")).toHaveValue("16:30");
  });
});
