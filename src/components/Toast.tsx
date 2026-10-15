import { useEffect } from "react";

export default function Toast({
  message,
  type = "ok",
  onClose,
}: {
  message: string;
  type?: "ok" | "err";
  onClose: () => void;
}) {
  useEffect(() => {
    const t = setTimeout(onClose, 4000);
    return () => clearTimeout(t);
  }, [onClose]);

  return (
    <div
      className={`fixed bottom-5 left-5 z-50 px-4 py-3 rounded-lg shadow-lg text-sm font-medium text-white animate-none ${
        type === "ok" ? "bg-emerald-600" : "bg-rose-600"
      }`}
      role="status"
    >
      <div className="flex items-center gap-3">
        <span>{type === "ok" ? "✅" : "⚠️"}</span>
        <span>{message}</span>
        <button onClick={onClose} className="opacity-70 hover:opacity-100">
          ✕
        </button>
      </div>
    </div>
  );
}
