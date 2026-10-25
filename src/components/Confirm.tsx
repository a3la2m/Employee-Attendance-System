import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import Modal from "./Modal";

/**
 * تأكيد «نعم/لا» داخلي بدل window.confirm.
 *
 * لماذا؟ حوار النظام (window.confirm) قد يستقبل نقرة ثانية عابرة (نقر مزدوج
 * على زر الحذف مثللاً) فيُطبَّق «نعم» قبل أن يقرأ المستخدم السؤال — فتتم
 * العملية دون إجابة. هنا:
 *  - زر «لا» مُركَّز افتراضياً (Enter = إلغاء دائماً).
 *  - زر «نعم» لا يستجيب خلال أول 350ms من فتح الحوار (تلتهم النقرة الثانية
 *    العابرة) ويستدعيه ضغط فعلي متأني.
 *  - Escape أو النقر خارج النافذة = إلغاء (آمن دائماً).
 * كل عمليات الحذف/الاستعادة/المسح في التطبيق تمرّ من هنا.
 */

export interface ConfirmOpts {
  title?: string;
  /** نص زر التأكيد (افتراضي: نعم، متابعة). */
  yes?: string;
  /** نص زر الإلغاء (افتراضي: لا، إلغاء). */
  no?: string;
}

type ConfirmFn = (message: string, opts?: ConfirmOpts) => Promise<boolean>;

const ConfirmCtx = createContext<ConfirmFn | null>(null);

/** مهلة السلاح (ms): تجاهل أي ضغط على «نعم» قبلها لالتقاط النقرة العابرة. */
const ARM_MS = 350;

export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<{
    msg: string;
    opts: ConfirmOpts;
  } | null>(null);
  const resolveRef = useRef<(v: boolean) => void>(() => {});
  const armedAtRef = useRef(0);
  const noRef = useRef<HTMLButtonElement>(null);

  const confirm = useCallback<ConfirmFn>(
    (message, opts = {}) =>
      new Promise<boolean>((resolve) => {
        resolveRef.current = resolve;
        armedAtRef.current = Date.now() + ARM_MS;
        setPending({ msg: message, opts });
      }),
    [],
  );

  const answer = useCallback((v: boolean) => {
    setPending(null);
    resolveRef.current(v);
  }, []);

  const onYes = () => {
    if (Date.now() < armedAtRef.current) return; // نقرة ثانية عابرة — تُتجاهل
    answer(true);
  };

  // التركيز على «لا» بعد فتح الحوار: Enter يُلغي ولا يحذف
  useEffect(() => {
    if (pending) noRef.current?.focus();
  }, [pending]);

  return (
    <ConfirmCtx.Provider value={confirm}>
      {children}
      {pending && (
        <Modal
          onClose={() => answer(false)}
          title={pending.opts.title ?? "تأكيد العملية"}
          backdropClass="z-[70]"
        >
          <p className="text-sm leading-relaxed whitespace-pre-line text-slate-700">
            {pending.msg}
          </p>
          <div className="flex gap-3 justify-end pt-1">
            <button ref={noRef} className="btn-ghost" onClick={() => answer(false)}>
              {pending.opts.no ?? "لا، إلغاء"}
            </button>
            <button className="btn-danger" onClick={onYes}>
              {pending.opts.yes ?? "نعم، متابعة"}
            </button>
          </div>
        </Modal>
      )}
    </ConfirmCtx.Provider>
  );
}

/** خطاف التأكيد — يعمل داخل أي مكوّن تحت ConfirmProvider. */
export function useConfirm(): ConfirmFn {
  const ctx = useContext(ConfirmCtx);
  if (!ctx)
    throw new Error("useConfirm خارج ConfirmProvider — غيّر التغليف في main.tsx");
  return ctx;
}
