import { useEffect, useRef, type ReactNode } from "react";

/**
 * نافذة حوارية موحّدة لكل الشاشات:
 * - إغلاق بمفتاح Escape وبالنقر على الخلفية.
 * - role="dialog" + aria-modal + عنوان (aria-label) لقارئ الشاشة.
 * - تركيز أولي على النافذة (Tab يبدأ من داخلها).
 */
export default function Modal({
  onClose,
  title,
  children,
  /** فئات الحاوية الداخلية (العرض/الهوامش/الأنماط الخاصة). */
  panelClass = "max-w-md",
  /** فئات الخلفية (مثل z-40 للنوافذ المتداخلة). */
  backdropClass = "z-50",
  /** إخفاء زر الإغلاق ✕ (نوافذ تُغلق بزر خارجي فقط). */
  hideClose = false,
}: {
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  panelClass?: string;
  backdropClass?: string;
  hideClose?: boolean;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  // نحتفظ بأحدث onClose في مرجع حتى لا يعيد التأثير التشغيل عند كل تصيير
  // (onClose يُمرَّر غالباً كدالة سهمية جديدة كل مرة — وإلا فقد الحقل تركيزه
  // بعد كل حرف لأن التأثير كان يعيد التركيز على النافذة).
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeRef.current();
    };
    window.addEventListener("keydown", onKey);
    panelRef.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div
      className={`fixed inset-0 bg-black/40 ${backdropClass} overflow-y-auto p-4 flex`}
      onClick={onClose}
      role="presentation"
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : undefined}
        tabIndex={-1}
        className={`bg-white rounded-2xl shadow-2xl w-full ${panelClass} p-5 space-y-4 outline-none m-auto`}
        onClick={(e) => e.stopPropagation()}
      >
        {title !== undefined && (
          <div className="flex items-center justify-between">
            <h3 className="font-extrabold">{title}</h3>
            {!hideClose && (
              <button
                className="text-slate-400 hover:text-slate-600 text-xl leading-none"
                onClick={onClose}
                aria-label="إغلاق النافذة"
                title="إغلاق (Esc)"
              >
                ✕
              </button>
            )}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
