/**
 * هياكل تحميل (skeleton) بديلة عن رسالة «لا بيانات» أثناء جلب البيانات —
 * حتى لا يظن المستخدم أن اليوم/الشهر فارغ قبل انتهاء التحميل.
 */
export function SkeletonBar({ w = "w-20" }: { w?: string }) {
  return (
    <span className={`block h-3.5 ${w} rounded bg-slate-200 animate-pulse`} />
  );
}

/** صفوف جدول نابضة أثناء التحميل. */
export function TableSkeleton({ rows = 4, cols = 6 }: { rows?: number; cols?: number }) {
  return (
    <>
      {Array.from({ length: rows }, (_, r) => (
        <tr key={r} className="animate-pulse">
          {Array.from({ length: cols }, (_, c) => (
            <td key={c} className="td">
              <SkeletonBar w={c === 0 ? "w-28" : "w-14"} />
            </td>
          ))}
        </tr>
      ))}
    </>
  );
}

/** بطاقات لوحة التحكم أثناء التحميل. */
export function CardSkeleton() {
  return (
    <div className="card flex items-center gap-4 animate-pulse">
      <div className="w-12 h-12 rounded-xl bg-slate-200" />
      <div className="space-y-2">
        <div className="h-7 w-16 rounded bg-slate-200" />
        <div className="h-3 w-24 rounded bg-slate-100" />
      </div>
    </div>
  );
}
