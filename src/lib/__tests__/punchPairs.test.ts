import { describe, it, expect } from "vitest";
import { pairPunches, fmtDur } from "../punchPairs";
import type { PunchRow } from "../api";

/** حركة بصمة سريعة الاختيار (الوقت بصيغة ISO كما يحفظها النظام). */
const p = (
  device_time: string,
  punch_type: number,
  raw_status = 0,
  work_code = 0,
): PunchRow => ({ device_time, punch_type, raw_status, work_code });

describe("pairPunches — دمج دخول/خروج في صف واحد", () => {
  it("دخول ثم خروج في نفس اليوم ⇒ صف واحد بمدة صحيحة", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0),
      p("2026-10-20T16:30:00", 1),
    ]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      date: "2026-10-20",
      inTime: "08:00:00",
      outTime: "16:30:00",
      inType: 0,
      outType: 1,
      minutes: 510,
    });
  });

  it("أزواج متعددة في اليوم نفسه ⇒ كل زوج في صف لوحده", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0),
      p("2026-10-20T12:00:00", 1),
      p("2026-10-20T13:00:00", 0),
      p("2026-10-20T17:00:00", 1),
    ]);
    expect(rows).toHaveLength(2);
    // الأحدث أولاً كالواجهة
    expect(rows[0]).toMatchObject({
      inTime: "13:00:00",
      outTime: "17:00:00",
      minutes: 240,
    });
    expect(rows[1]).toMatchObject({
      inTime: "08:00:00",
      outTime: "12:00:00",
      minutes: 240,
    });
  });

  it("دخول دون خروج ⇒ صف لوحده بلا مدة", () => {
    const rows = pairPunches([p("2026-10-20T08:00:00", 0)]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      inTime: "08:00:00",
      outTime: null,
      outType: null,
      minutes: null,
    });
    expect(fmtDur(rows[0].minutes)).toBe("—");
  });

  it("خروج دون دخول ⇒ صف لوحده بلا مدة", () => {
    const rows = pairPunches([p("2026-10-20T16:00:00", 1)]);
    expect(rows).toHaveLength(1);
    expect(rows[0]).toMatchObject({
      inTime: null,
      inType: null,
      outTime: "16:00:00",
      minutes: null,
    });
  });

  it("خروجان متعاقبان دون دخول بينهما ⇒ كل خروج في صف لوحده", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0),
      p("2026-10-20T12:00:00", 1),
      p("2026-10-20T16:00:00", 1),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({ inTime: null, outTime: "16:00:00" });
    expect(rows[1]).toMatchObject({ inTime: "08:00:00", outTime: "12:00:00" });
  });

  it("دخولان متتاليان دون خروج بينهما ⇒ الأول بلا خروج والثاني يُقابله", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0),
      p("2026-10-20T08:05:00", 0),
      p("2026-10-20T16:00:00", 1),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      inTime: "08:05:00",
      outTime: "16:00:00",
      minutes: 475,
    });
    expect(rows[1]).toMatchObject({ inTime: "08:00:00", outTime: null });
  });

  it("استراحة (2 ثم 3) تُقسم اليوم إلى مدةَي دخول/خروج", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0),
      p("2026-10-20T12:00:00", 2),
      p("2026-10-20T12:30:00", 3),
      p("2026-10-20T16:30:00", 1),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      inTime: "12:30:00",
      outTime: "16:30:00",
      inType: 3,
      outType: 1,
      minutes: 240,
    });
    expect(rows[1]).toMatchObject({
      inTime: "08:00:00",
      outTime: "12:00:00",
      inType: 0,
      outType: 2,
      minutes: 240,
    });
  });

  it("لا يقترن دخول آخر النهار بخروج اليوم التالي (كل يوم بذاته)", () => {
    const rows = pairPunches([
      p("2026-10-20T22:00:00", 0),
      p("2026-10-21T06:00:00", 1),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      date: "2026-10-21",
      inTime: null,
      outTime: "06:00:00",
    });
    expect(rows[1]).toMatchObject({
      date: "2026-10-20",
      inTime: "22:00:00",
      outTime: null,
    });
  });

  it("يعمل بترتيب الحركات المدخل (يرتّب تصاعدياً ثم يعرض الأحدث أولاً)", () => {
    const rows = pairPunches([
      p("2026-10-21T16:00:00", 1),
      p("2026-10-21T08:00:00", 0),
      p("2026-10-20T16:00:00", 1),
      p("2026-10-20T08:00:00", 0),
    ]);
    expect(rows.map((r) => r.date)).toEqual(["2026-10-21", "2026-10-20"]);
    expect(rows.every((r) => r.minutes === 480)).toBe(true);
  });

  it("يحتفظ برموز الجهاز للدخول والخروج", () => {
    const rows = pairPunches([
      p("2026-10-20T08:00:00", 0, 1, 2),
      p("2026-10-20T16:00:00", 1, 3, 4),
    ]);
    expect(rows[0].inDevice).toBe("1/2");
    expect(rows[0].outDevice).toBe("3/4");
  });

  it("القائمة الفارغة ⇒ لا صفوف", () => {
    expect(pairPunches([])).toEqual([]);
  });
});

describe("fmtDur — صياغة المدة بالعربية", () => {
  it("ساعات ودقائق / ساعات فقط / دقائق فقط", () => {
    expect(fmtDur(510)).toBe("8 س 30 د");
    expect(fmtDur(480)).toBe("8 س");
    expect(fmtDur(45)).toBe("45 د");
    expect(fmtDur(0)).toBe("0 د");
  });

  it("null أو NaN ⇒ شرطة", () => {
    expect(fmtDur(null)).toBe("—");
    expect(fmtDur(NaN)).toBe("—");
  });
});
