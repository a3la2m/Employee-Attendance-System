import { describe, it, expect, vi } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import Modal from "../Modal";

describe("Modal الموحّد", () => {
  it("يحمل صفات الوصول: role=dialog وaria-modal واسماً", () => {
    render(
      <Modal onClose={() => {}} title="عنوان تجريبي">
        <p>محتوى</p>
      </Modal>,
    );
    const dlg = screen.getByRole("dialog");
    expect(dlg).toHaveAttribute("aria-modal", "true");
    expect(dlg).toHaveAccessibleName("عنوان تجريبي");
  });

  it("يُغلق بمفتاح Escape", async () => {
    const onClose = vi.fn();
    render(
      <Modal onClose={onClose} title="نافذة">
        <p>محتوى</p>
      </Modal>,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("النقر داخل النافذة لا يُغلقها، ونقر الخلفية يُغلقها", async () => {
    const onClose = vi.fn();
    render(
      <Modal onClose={onClose} title="نافذة">
        <button data-testid="inner">داخلي</button>
      </Modal>,
    );
    await userEvent.click(screen.getByTestId("inner"));
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("presentation"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("زر الإغلاق ✕ يستدعي onClose", async () => {
    const onClose = vi.fn();
    render(
      <Modal onClose={onClose} title="نافذة">
        <p>محتوى</p>
      </Modal>,
    );
    await userEvent.click(screen.getByLabelText("إغلاق النافذة"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("يمكن إخفاء زر الإغلاق", () => {
    render(
      <Modal onClose={() => {}} title="نافذة" hideClose>
        <p>محتوى</p>
      </Modal>,
    );
    expect(screen.queryByLabelText("إغلاق النافذة")).not.toBeInTheDocument();
  });

  it("لا يفقد الحقل تركيزه عند الكتابة (onClose متغيّر كل تصيير)", async () => {
    function Wrapper() {
      const [v, setV] = useState("");
      // onClose دالة سهمية جديدة كل تصيير — كما في الشاشات الحقيقية
      return (
        <Modal onClose={() => {}} title="نافذة">
          <input
            aria-label="اسم"
            value={v}
            onChange={(e) => setV(e.target.value)}
          />
        </Modal>
      );
    }
    render(<Wrapper />);
    const input = screen.getByLabelText("اسم");
    await userEvent.click(input);
    await userEvent.keyboard("محمد");
    expect(input).toHaveFocus();
    expect(input).toHaveValue("محمد");
  });
});
