import { useEffect, useState } from "react";
import { api } from "../api";
import { t } from "../i18n";

export function SelectionMenuPage() {
  const [language, setLanguage] = useState("zh-CN");
  useEffect(() => { void api.getConfig().then(config => setLanguage(config.ui_language)).catch(() => {}); }, []);

  return <div className="flex h-full w-full items-center bg-transparent p-1">
    <div className="flex h-full w-full items-center rounded-xl border border-slate-300 bg-[#f4f4f2] px-1 shadow-[0_5px_16px_rgba(31,41,55,.16)]">
      <button type="button" className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-left text-xs font-medium text-slate-800 hover:bg-white"
        onClick={() => void api.translateMenuSelection()}>
        <span aria-hidden="true" className="text-base text-blue-700">↗</span>
        <span className="truncate">{t("翻译选中文字", language)}</span>
      </button>
      <button type="button" aria-label={t("取消", language)} className="rounded-lg px-1.5 text-sm text-slate-500 hover:bg-white hover:text-slate-800"
        onClick={() => void api.hideSelectionMenu()}>×</button>
    </div>
  </div>;
}
