import type { ReactNode } from "react";
import type { Settings } from "../lib/types";
import { headerText, footerText, showLogo } from "../lib/print";

/** ترويسة كل المطبوعات: الشعار + نص الرأس (من الإعدادات) + معلومات التقرير. */
export function PrintHeader({
  cfg,
  logo,
  title,
  meta,
}: {
  cfg: Settings;
  logo: string;
  title: string;
  meta?: ReactNode;
}) {
  const logoOn = showLogo(cfg);
  return (
    <header className="flex items-center justify-between border-b-2 border-slate-800 pb-4 mb-5">
      <div className="flex items-center gap-4">
        {logoOn &&
          (logo ? (
            <img src={logo} alt="شعار" className="w-16 h-16 object-contain" />
          ) : (
            <div className="w-16 h-16 rounded-lg bg-slate-200 grid place-content-center text-2xl">
              🏢
            </div>
          ))}
        <div>
          <div className="text-xl font-extrabold whitespace-pre-line">
            {headerText(cfg)}
          </div>
          <div className="text-sm text-slate-600">نظام إدارة الحضور والرواتب</div>
        </div>
      </div>
      <div className="text-left text-sm">
        <div className="font-bold">{title}</div>
        {meta}
      </div>
    </header>
  );
}

/** تذييل كل المطبوعات: نص التذييل (من الإعدادات) + تاريخ الطباعة. */
export function PrintFooter({ cfg, date }: { cfg: Settings; date?: string }) {
  return (
    <div className="mt-4 pt-3 border-t border-slate-300 text-center text-xs text-slate-600 whitespace-pre-line">
      <div>{footerText(cfg)}</div>
      {date && <div className="mt-1 text-slate-400">تاريخ الطباعة: {date}</div>}
    </div>
  );
}
