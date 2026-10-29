/** اختبارات ورقة طباعة حركات الموظف + استخراج Excel (نافذة «عرض الحركات»).
 *
 * المعايير:
 * 1. تقسيم الطباعة إلى صفحات A4 بحد أقصى 27 صفاً في الصفحة مع ترقيم متواصل
 *    وترويسة/رقم صفحة على كل صفحة.
 * 2. عمود «ملاحظة» يُطبع فارغاً (يُملأ يدوياً على الورق).
 * 3. زر «استخراج Excel» يُنزّل ملف CSV بترميز UTF-8 (BOM) بعناوينه وإجماليه.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/react";
import type { PunchRow } from "../../lib/api";

/** حركات ثابتة: 30 يوماً من الشهر الحالي (دخول 06:00 + خروج 16:30). */
const { punchesFixture } = vi.hoisted(() => {
  const now = new Date();
  const y = now.getFullYear();
  const m = now.getMonth();
  const days = new Date(y, m + 1, 0).getDate();
  const punchesFixture: PunchRow[] = [];
  for (let d = 1; d <= Math.min(30, days); d++) {
    const date = `${y}-${String(m + 1).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
    punchesFixture.push(
      { device_time: `${date}T06:00:00`, punch_type: 0, work_code: 0, raw_status: 1 },
      { device_time: `${date}T16:30:00`, punch_type: 1, work_code: 0, raw_status: 1 },
    );
  }
  return { punchesFixture };
});

vi.mock("../../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/api")>();
  return {
    ...actual,
    employeePunches: vi.fn(async () => punchesFixture),
    getSettings: vi.fn(async () => ({ company_name: "شركة الاختبار" })),
    listLeaves: vi.fn(async () => []),
    holidaysList: vi.fn(async () => []),
    readLogoDataUri: vi.fn(async () => ""),
  };
});

import { ConfirmProvider } from "../../components/Confirm";
import { PunchesModal } from "../Employees";

const EMP = {
  id: 1,
  pin: "81",
  name: "خليل",
  name_en: "Khalil",
  job_title: "",
  phone: "",
  daily_hours: 8.5,
  break_min: 30,
  hourly_rate: 15,
  meal_allow: 7,
  transport_allow: 0,
  active: true,
  overtime_enabled: true,
  meal_enabled: true,
  transport_enabled: false,
  weekend_paid: true,
  annual_leave_balance: 0,
  annual_leave_days: 14,
  leave_accrual_month: null,
  shift_id: null,
  device_privilege: 0,
  bot_allowed: true,
};

/** عدّ صفحات الطباعة وصفوف كل صفحة بعد انتهاء التحميل. */
async function renderedPages(): Promise<Element[]> {
  let pages: Element[] = [];
  await waitFor(
    () => {
      pages = [...document.querySelectorAll(".punches-sheet .print-page")];
      expect(pages.length).toBeGreaterThan(0);
    },
    { timeout: 3000 },
  );
  return pages;
}

const rowsOf = (page: Element) => [...page.querySelectorAll("tbody tr")];

describe("ورقة طباعة الحركات", () => {
  beforeEach(() => cleanup());

  it("تقسم إلى صفحات A4 بـ 27 صفاً كحد أقصى بترقيم متواصل ورقم صفحة", async () => {
    render(
      <ConfirmProvider>
        <PunchesModal emp={EMP as never} onClose={() => {}} toast={() => {}} />
      </ConfirmProvider>,
    );
    const pages = await renderedPages();
    // 30 صفاً مدموجاً ← صفحتان: 27 + 3
    expect(pages.length).toBe(2);
    expect(rowsOf(pages[0]).length).toBe(27);
    expect(rowsOf(pages[1]).length).toBe(3);
    // الترقيم يواصل بين الصفحات: آخر صف في الأولى = 27 وأول صف في الثانية = 28
    expect(rowsOf(pages[0])[26].firstElementChild?.textContent).toBe("27");
    expect(rowsOf(pages[1])[0].firstElementChild?.textContent).toBe("28");
    // ترويسة كل صفحة تحمل رقمها
    const sheet = document.querySelector(".punches-sheet")!;
    expect(sheet.textContent).toContain("صفحة 1 من 2");
    expect(sheet.textContent).toContain("صفحة 2 من 2");
  });

  it("يطبع عمود الملاحظة فارغاً في كل الصفوف", async () => {
    render(
      <ConfirmProvider>
        <PunchesModal emp={EMP as never} onClose={() => {}} toast={() => {}} />
      </ConfirmProvider>,
    );
    const pages = await renderedPages();
    const notes = pages
      .flatMap((p) => rowsOf(p))
      .map((tr) => tr.lastElementChild?.textContent?.trim());
    expect(notes.length).toBe(30);
    expect(notes.every((n) => n === "")).toBe(true);
  });

  it("زر استخراج Excel يُنزّل CSV بترميز UTF-8 وبعناوين وإجمالي", async () => {
    const blobs: Blob[] = [];
    const clicks: number[] = [];
    // jsdom بلا createObjectURL — نُعرّفه كما في المتصفح الحقيقي
    (URL as unknown as Record<string, unknown>).createObjectURL = (b: Blob) => {
      blobs.push(b);
      return "blob:test";
    };
    (URL as unknown as Record<string, unknown>).revokeObjectURL = () => {};
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicks.push(1);
    });
    const toast = vi.fn();
    render(
      <ConfirmProvider>
        <PunchesModal emp={EMP as never} onClose={() => {}} toast={toast} />
      </ConfirmProvider>,
    );
    await renderedPages();
    const btn = [...document.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("استخراج Excel"),
    );
    expect(btn).toBeTruthy();
    fireEvent.click(btn!);
    await waitFor(() => expect(blobs.length).toBe(1));
    // jsdom بلا Blob.text() — نقرأ عبر FileReader كما يفعل المستخدم فعلياً
    const read = (mode: "text" | "bytes") =>
      new Promise<string | Uint8Array>((resolve, reject) => {
        const r = new FileReader();
        r.onload = () =>
          resolve(
            mode === "bytes"
              ? new Uint8Array(r.result as ArrayBuffer)
              : String(r.result),
          );
        r.onerror = () => reject(r.error);
        mode === "bytes" ? r.readAsArrayBuffer(blobs[0]) : r.readAsText(blobs[0]);
      });
    // البادئة BOM (EF BB BF) في البайتات — تجعل Excel يفتح العربية صحيحة
    const bytes = (await read("bytes")) as Uint8Array;
    expect([bytes[0], bytes[1], bytes[2]]).toEqual([0xef, 0xbb, 0xbf]);
    const text = (await read("text")) as string;
    const lines = text.split("\r\n");
    expect(lines[0]).toBe("#,التاريخ,الدخول,الخروج,المدة,ملاحظة");
    expect(lines.length).toBe(1 + punchesFixture.length / 2 + 1); // عنوان + 30 صف + إجمالي
    expect(lines[lines.length - 1]).toContain("الإجمالي");
    expect(clicks.length).toBe(1);
    expect(toast).toHaveBeenCalledWith(expect.stringContaining("Excel"));
    // تنظيف بدائل jsdom
    delete (URL as unknown as Record<string, unknown>).createObjectURL;
    delete (URL as unknown as Record<string, unknown>).revokeObjectURL;
  });
});
