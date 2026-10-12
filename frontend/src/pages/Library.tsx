import { useEffect, useMemo, useRef, useState } from "react";
import { api, type WordRow } from "../api";
import { t } from "../i18n";

const PAGE_SIZE = 50;
type Props = { language: string; source: string; target: string; onBack: () => void };

export function LibraryPage({ language, source, target, onBack }: Props) {
  const [rows, setRows] = useState<WordRow[]>([]);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [advanced, setAdvanced] = useState(false);
  const [rawJson, setRawJson] = useState("");
  const [jsonErrorLine, setJsonErrorLine] = useState(0);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exportName, setExportName] = useState("user_words_export.json");
  const [importOpen, setImportOpen] = useState(false);
  const [importJson, setImportJson] = useState("");
  const importInput = useRef<HTMLInputElement>(null);
  const label = (key: string) => t(key, language);

  useEffect(() => {
    let active = true;
    api.reviewVoiceCache().catch(error => { if (active) setMessage(String(error)); })
      .then(() => api.listWords(source, target))
      .then(items => { if (active) setRows(items); })
      .catch(error => { if (active) setMessage(String(error)); });
    return () => { active = false; };
  }, [source, target]);
  const filtered = useMemo(() => rows.map((row, index) => ({ row, index })).filter(({ row }) =>
    [row.key, row.value, row.pronunciation].some(value => value.toLocaleLowerCase().includes(search.toLocaleLowerCase()))), [rows, search]);
  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const visible = filtered.slice(Math.min(page, pageCount - 1) * PAGE_SIZE, (Math.min(page, pageCount - 1) + 1) * PAGE_SIZE);

  const updateRow = (index: number, field: "key" | "value" | "pronunciation", value: string) =>
    setRows(current => current.map((row, currentIndex) => currentIndex === index ? { ...row, [field]: value } : row));
  const addRow = () => { setRows(current => [{ key: "", value: "", pronunciation: "", created_at: 0, created_at_label: "" }, ...current]); setSearch(""); setPage(0); };
  const copyRow = (index: number) => setRows(current => [{ ...current[index], created_at: 0, created_at_label: "" }, ...current]);
  const deleteRow = (index: number) => setRows(current => current.filter((_, currentIndex) => currentIndex !== index));
  const reportJsonError = (error: unknown) => {
    const text = String(error);
    const line = /line\s+(\d+)/i.exec(text)?.[1];
    const position = /position\s+(\d+)/i.exec(text)?.[1];
    const found = line ? Number(line) : position ? rawJson.slice(0, Number(position)).split("\n").length : 0;
    setJsonErrorLine(found);
    setMessage(`${label("保存失败，JSON格式错误")}：${text}`);
  };
  const switchMode = () => {
    if (!advanced) {
      setRawJson(JSON.stringify(Object.fromEntries(rows.filter(row => row.key.trim() && row.value.trim()).map(row => [row.key, row.value])), null, 2));
      setJsonErrorLine(0);
      setAdvanced(true);
    } else {
      try {
        const parsed: unknown = JSON.parse(rawJson);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error(label("JSON 格式错误"));
        setRows(Object.entries(parsed).map(([key, value]) => {
          if (typeof value !== "string") throw new Error(label("key 和 value 都必须是字符串"));
          const old = rows.find(row => row.key === key);
          return { key, value, pronunciation: old?.pronunciation ?? "", created_at: old?.created_at ?? 0, created_at_label: old?.created_at_label ?? "" };
        }));
        setAdvanced(false);
        setMessage("");
      } catch (error) { reportJsonError(error); }
    }
  };
  const save = async () => {
    setBusy(true);
    try {
      let draft = rows;
      if (advanced) {
        try {
          const parsed: Record<string, unknown> = JSON.parse(rawJson);
          if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error(label("JSON 格式错误"));
          draft = Object.entries(parsed).map(([key, value]) => {
            if (typeof value !== "string") throw new Error(label("key 和 value 都必须是字符串"));
            const old = rows.find(row => row.key === key);
            return { key, value, pronunciation: old?.pronunciation ?? "", created_at: old?.created_at ?? 0, created_at_label: old?.created_at_label ?? "" };
          });
        } catch (error) { reportJsonError(error); return; }
      }
      const seen = new Set<string>();
      const cleaned = draft.filter(row => {
        const key = row.key.trim();
        if (!key || !row.value.trim() || seen.has(key)) return false;
        seen.add(key); return true;
      });
      await api.saveWords(source, target, cleaned);
      setRows(await api.listWords(source, target));
      setMessage(label("保存成功"));
    } catch (error) { setMessage(`${label("保存失败")}：${String(error)}`); }
    finally { setBusy(false); }
  };
  const importFile = async (file?: File) => {
    if (!file) return;
    try {
      setImportJson(await file.text());
      setMessage("");
    } catch (error) { setMessage(String(error)); }
  };
  const applyImport = () => {
    try {
      const parsed: unknown = JSON.parse(importJson);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error(label("JSON 格式错误"));
      const imported: WordRow[] = Object.entries(parsed).map(([key, value]) => {
        if (typeof value !== "string") throw new Error(label("key 和 value 都必须是字符串"));
        return { key, value, pronunciation: "", created_at: 0, created_at_label: "" };
      });
      setRows(current => [...imported, ...current]); setSearch(""); setPage(0);
      setImportOpen(false); setImportJson("");
      setMessage(`${label("已批量导入")} ${imported.length}`);
    } catch (error) { setMessage(`${label("批量导入失败，JSON 格式错误：{error}")} ${String(error)}`); }
  };

  return <main className="flex h-full min-w-[650px] flex-col bg-[#eeedea] p-6 text-slate-800">
    <div className="mb-5 flex shrink-0 items-center gap-4">
      <button type="button" className="button" onClick={onBack}>← {label("返回设置")}</button>
      <h1 className="text-xl font-semibold">{label("词库编辑器")}</h1>
    </div>
    <div className="surface-card flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-200 p-4">
        <h2 className="mr-auto text-base font-semibold">{label("词条")}</h2>
        <button type="button" className="button button-primary" onClick={addRow}>＋ {label("新增")}</button>
        <button type="button" className="button" onClick={() => setImportOpen(true)}>{label("批量导入")}</button>
        <button type="button" className="button" onClick={() => setExportOpen(true)}>{label("批量导出")}</button>
        <button type="button" className="button" onClick={switchMode}>{advanced ? label("结构化表格") : label("高级 JSON 编辑")}</button>
      </div>
      {advanced ? <div className="flex min-h-0 flex-1 flex-col p-4"><textarea className="app-scrollbar min-h-0 flex-1 resize-none rounded-xl border border-slate-300 bg-white p-4 font-mono text-sm outline-none focus:border-blue-500"
        spellCheck={false} value={rawJson} onChange={event => { setRawJson(event.target.value); setJsonErrorLine(0); }} />
        {jsonErrorLine > 0 && <div className="mt-3 rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-800">
          <span>{label("第")} {jsonErrorLine} {label("行")}：</span>
          <code className="ml-2 whitespace-pre-wrap rounded bg-red-100 px-1.5 py-0.5">{rawJson.split("\n")[jsonErrorLine - 1]}</code>
        </div>}
      </div> : <>
        <div className="p-4"><div className="flex h-10 items-center gap-2 rounded-full border border-slate-300 bg-white px-4 shadow-sm focus-within:border-blue-500 focus-within:ring-2 focus-within:ring-blue-100">
          <span aria-hidden="true" className="text-slate-500">⌕</span>
          <input className="min-w-0 flex-1 bg-transparent text-sm outline-none" placeholder={label("搜索译文、原文或读音…")}
            value={search} onChange={event => { setSearch(event.target.value); setPage(0); }} />
          {search && <button type="button" aria-label={label("清除搜索")} className="text-lg text-slate-500 hover:text-slate-900" onClick={() => { setSearch(""); setPage(0); }}>×</button>}
        </div></div>
        <div className="app-scrollbar min-h-0 flex-1 overflow-auto">
          <table className="w-full min-w-[620px] table-fixed border-collapse text-left text-sm">
            <colgroup><col className="w-[31%]" /><col className="w-[38%]" /><col className="w-[16%]" /><col className="w-[15%]" /></colgroup>
            <thead className="sticky top-0 z-10 bg-[#484c51] text-slate-50"><tr>
              {[label("译文"), label("原文"), label("读音"), ""].map((name, index) =>
                <th key={index} className="border-r border-slate-600 px-4 py-3 font-medium last:border-r-0">{name}</th>)}
            </tr></thead>
            <tbody>{visible.map(({ row, index }, visibleIndex) => <tr key={`${index}-${row.created_at}`} className={visibleIndex % 2 ? "bg-slate-100" : "bg-white"}>
              {(["key", "value", "pronunciation"] as const).map(field => <td key={field} className="border-b border-r border-slate-300 px-2 py-1.5 align-middle">
                <input className="w-full truncate rounded-md bg-transparent px-2 py-1.5 outline-none focus:bg-white focus:ring-2 focus:ring-blue-200"
                  title={row[field]} value={row[field]} onChange={event => updateRow(index, field, event.target.value)} />
              </td>)}
              <td className="border-b border-slate-300 px-2 text-right"><div className="flex justify-end gap-1">
                <button type="button" className="rounded-md bg-sky-100 px-2 py-1 text-xs font-semibold text-sky-900 hover:bg-sky-200" onClick={() => copyRow(index)}>{label("复制")}</button>
                <button type="button" className="rounded-md bg-red-50 px-2 py-1 text-xs font-semibold text-red-700 hover:bg-red-100" onClick={() => deleteRow(index)}>{label("删除")}</button>
              </div></td>
            </tr>)}</tbody>
          </table>
          {!visible.length && <div className="grid h-40 place-items-center text-sm text-slate-500">{label("没有符合筛选条件的词条")}</div>}
        </div>
        <div className="flex items-center justify-between border-t border-slate-200 px-4 py-3 text-sm text-slate-500">
          <span>{filtered.length} / {rows.length}</span><div className="flex items-center gap-3">
            <button type="button" disabled={!page} className="button disabled:opacity-40" onClick={() => setPage(page - 1)}>{label("上一页")}</button>
            <span>{Math.min(page, pageCount - 1) + 1} / {pageCount}</span>
            <button type="button" disabled={page >= pageCount - 1} className="button disabled:opacity-40" onClick={() => setPage(page + 1)}>{label("下一页")}</button>
          </div>
        </div>
      </>}
    </div>
    <footer className="mt-4 flex shrink-0 items-center gap-3">
      <span role="status" className="mr-auto truncate text-sm text-slate-600">{message}</span>
      <button type="button" className="button" onClick={onBack}>{label("取消")}</button>
      <button type="button" className="button button-primary" disabled={busy} onClick={save}>{label("保存")}</button>
    </footer>
    {exportOpen && <div className="fixed inset-0 z-30 grid place-items-center bg-slate-900/35 p-6"><div className="surface-card w-full max-w-md p-6">
      <h2 className="mb-3 text-lg font-semibold">{label("批量导出")}</h2>
      <input className="field mb-5" value={exportName} onChange={event => setExportName(event.target.value)} />
      <div className="flex justify-end gap-2"><button type="button" className="button" onClick={() => setExportOpen(false)}>{label("取消")}</button>
        <button type="button" className="button button-primary" onClick={async () => {
          try { const path = await api.exportWords(exportName, rows); setMessage(path); setExportOpen(false); }
          catch (error) { setMessage(String(error)); }
        }}>{label("导出")}</button></div>
    </div></div>}
    {importOpen && <div className="fixed inset-0 z-30 grid place-items-center bg-slate-900/35 p-6"><div className="surface-card flex max-h-[85vh] w-full max-w-2xl flex-col bg-white p-6">
      <h2 className="mb-4 text-lg font-semibold">{label("批量导入")}</h2>
      <button type="button" className="button mb-4 w-fit" onClick={() => importInput.current?.click()}>{label("选择 JSON 文件")}</button>
      <input ref={importInput} type="file" accept=".json,application/json" hidden onChange={event => { void importFile(event.target.files?.[0]); event.target.value = ""; }} />
      <textarea className="app-scrollbar min-h-48 flex-1 resize-y rounded-xl border border-slate-300 bg-white p-4 font-mono text-sm outline-none focus:border-blue-500"
        spellCheck={false} value={importJson} onChange={event => setImportJson(event.target.value)} placeholder={'{"译文": "原文"}'} />
      <p className="mt-3 text-xs text-slate-500">{label("导入后仍需点击保存，才会写入词库。")}</p>
      <div className="mt-4 flex justify-end gap-2"><button type="button" className="button" onClick={() => { setImportOpen(false); setImportJson(""); }}>{label("取消")}</button>
        <button type="button" className="button button-primary" disabled={!importJson.trim()} onClick={applyImport}>{label("导入")}</button></div>
    </div></div>}
  </main>;
}
