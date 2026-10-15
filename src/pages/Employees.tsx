import { useEffect, useState } from "react";
import { listEmployees, upsertEmployee, deleteEmployee } from "../lib/api";
import type { Employee } from "../lib/types";

type Toast = (m: string, t?: "ok" | "err") => void;

const EMPTY: Employee = {
  id: 0,
  pin: "",
  name: "",
  job_title: "",
  phone: "",
  daily_hours: 8.5,
  break_min: 30,
  hourly_rate: 10,
  meal_allow: 7,
  transport_allow: 5,
  active: true,
};

export default function Employees({ toast }: { toast: Toast }) {
  const [rows, setRows] = useState<Employee[]>([]);
  const [form, setForm] = useState<Employee | null>(null);
  const [q, setQ] = useState("");

  const load = () =>
    listEmployees()
      .then(setRows)
      .catch((e) => toast(String(e), "err"));

  useEffect(() => {
    load();
  }, []);

  const save = async () => {
    if (!form) return;
    if (!form.name.trim()) return toast("الاسم مطلوب", "err");
    try {
      await upsertEmployee(form);
      toast(form.id ? "تم تحديث الموظف" : "تمت إضافة الموظف");
      setForm(null);
      load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const remove = async (emp: Employee) => {
    if (!confirm(`حذف الموظف «${emp.name}» مع سجلاته؟`)) return;
    try {
      await deleteEmployee(emp.id);
      toast("تم الحذف");
      load();
    } catch (e) {
      toast(String(e), "err");
    }
  };

  const filtered = rows.filter(
    (r) =>
      r.name.includes(q) ||
      (r.pin ?? "").includes(q) ||
      r.job_title.includes(q),
  );

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-extrabold">الموظفون</h1>
        <div className="flex gap-2">
          <input
            className="input !w-56"
            placeholder="🔎 بحث بالاسم أو رقم البصمة…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
          />
          <button className="btn-primary" onClick={() => setForm({ ...EMPTY })}>
            + موظف جديد
          </button>
        </div>
      </div>

      {/* نموذج */}
      {form && (
        <div className="card border-indigo-200 bg-indigo-50/40">
          <h2 className="font-bold mb-4">
            {form.id ? "تعديل موظف" : "إضافة موظف جديد"}
          </h2>
          <div className="grid sm:grid-cols-3 gap-4">
            <div>
              <label className="label">الاسم الكامل *</label>
              <input
                className="input"
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
            </div>
            <div>
              <label className="label">رقم البصمة (PIN على الجهاز)</label>
              <input
                className="input"
                value={form.pin ?? ""}
                placeholder="مثال: 99999"
                onChange={(e) => setForm({ ...form, pin: e.target.value })}
              />
            </div>
            <div>
              <label className="label">الوظيفة</label>
              <input
                className="input"
                value={form.job_title}
                onChange={(e) => setForm({ ...form, job_title: e.target.value })}
              />
            </div>
            <div>
              <label className="label">الهاتف</label>
              <input
                className="input"
                value={form.phone}
                onChange={(e) => setForm({ ...form, phone: e.target.value })}
              />
            </div>
            <div>
              <label className="label">ساعات العمل اليومية</label>
              <input
                type="number"
                step="0.25"
                className="input"
                value={form.daily_hours}
                onChange={(e) =>
                  setForm({ ...form, daily_hours: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="label">الاستراحة (دقيقة)</label>
              <input
                type="number"
                className="input"
                value={form.break_min}
                onChange={(e) =>
                  setForm({ ...form, break_min: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="label">سعر الساعة (شيكل)</label>
              <input
                type="number"
                step="0.5"
                className="input"
                value={form.hourly_rate}
                onChange={(e) =>
                  setForm({ ...form, hourly_rate: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="label">بدل فطور/مواصلات يومي</label>
              <div className="flex gap-2">
                <input
                  type="number"
                  className="input"
                  value={form.meal_allow}
                  onChange={(e) =>
                    setForm({ ...form, meal_allow: Number(e.target.value) })
                  }
                />
                <input
                  type="number"
                  className="input"
                  value={form.transport_allow}
                  onChange={(e) =>
                    setForm({ ...form, transport_allow: Number(e.target.value) })
                  }
                />
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2 mt-4">
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={form.active}
                onChange={(e) => setForm({ ...form, active: e.target.checked })}
              />
              نشط
            </label>
            <div className="flex-1" />
            <button className="btn-ghost" onClick={() => setForm(null)}>
              إلغاء
            </button>
            <button className="btn-primary" onClick={save}>
              💾 حفظ
            </button>
          </div>
        </div>
      )}

      {/* الجدول */}
      <div className="card !p-0 overflow-hidden">
        <table className="w-full">
          <thead>
            <tr>
              <th className="th">الاسم</th>
              <th className="th">رقم البصمة</th>
              <th className="th">الوظيفة</th>
              <th className="th">ساعات/يوم</th>
              <th className="th">سعر الساعة</th>
              <th className="th">الحالة</th>
              <th className="th">إجراءات</th>
            </tr>
          </thead>
          <tbody>
            {filtered.length === 0 && (
              <tr>
                <td className="td text-center text-slate-400 py-8" colSpan={7}>
                  لا يوجد موظفون — اضغط «+ موظف جديد» أو اسحب من الجهاز.
                </td>
              </tr>
            )}
            {filtered.map((r) => (
              <tr key={r.id} className="hover:bg-slate-50">
                <td className="td font-medium">{r.name}</td>
                <td className="td font-mono">{r.pin || "—"}</td>
                <td className="td">{r.job_title || "—"}</td>
                <td className="td">{r.daily_hours}</td>
                <td className="td">{r.hourly_rate}</td>
                <td className="td">
                  <span
                    className={`badge ${
                      r.active
                        ? "bg-emerald-100 text-emerald-700"
                        : "bg-slate-200 text-slate-500"
                    }`}
                  >
                    {r.active ? "نشط" : "موقوف"}
                  </span>
                </td>
                <td className="td">
                  <div className="flex gap-2">
                    <button
                      className="text-indigo-600 hover:underline text-xs"
                      onClick={() => setForm(r)}
                    >
                      تعديل
                    </button>
                    <button
                      className="text-rose-600 hover:underline text-xs"
                      onClick={() => remove(r)}
                    >
                      حذف
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
