import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type SearchHit, type UiConfig } from "../api";

function Highlight({ text, query }: { text: string; query: string }) {
  if (!query) return <>{text}</>;
  const index = text.toLocaleLowerCase().indexOf(query.toLocaleLowerCase());
  if (index < 0) return <>{text}</>;
  return <>{text.slice(0, index)}<mark className="rounded-sm bg-amber-200/80 text-inherit">{text.slice(index, index + query.length)}</mark>{text.slice(index + query.length)}</>;
}

export function QueryPage() {
  const [config, setConfig] = useState<UiConfig | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [total, setTotal] = useState(0);
  const [selected, setSelected] = useState(0);
  const [page, setPage] = useState(0);
  const [composed, setComposed] = useState<SearchHit[]>([]);
  const [status, setStatus] = useState("");
  const [translation, setTranslation] = useState("");
  const [preferTranslation, setPreferTranslation] = useState(false);
  const [searched, setSearched] = useState<{ query: string; page: number } | null>(null);
  const [searchError, setSearchError] = useState("");
  const [languageMismatch, setLanguageMismatch] = useState(false);
  const [unmatchedEnter, setUnmatchedEnter] = useState(false);
  const [translating, setTranslating] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLElement>(null);
  const lastRequested = useRef(0);
  const translationRequest = useRef(0);
  const pageSize = config?.results.max_visible_results ?? 8;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const visible = useMemo(() => searched?.query === query.trim() && searched.page === page ? hits : [], [hits, page, query, searched]);
  const searching = !!query.trim() && (searched?.query !== query.trim() || searched.page !== page);
  const sameLanguage = !!config?.translation_language && config.source_language === config.translation_language;
  const noMatch = !!query.trim() && !searching && !searchError && !languageMismatch && total === 0 && !translation;
  const canTranslate = !!query.trim() && !sameLanguage && !!config?.ai_translation && !!config.translation_language;

  const reset = () => {
    ++lastRequested.current;
    ++translationRequest.current;
    setQuery(""); setHits([]); setTotal(0); setSelected(0); setPage(0);
    setComposed([]); setTranslation(""); setPreferTranslation(false); setStatus(""); setSearchError("");
    setSearched(null); setUnmatchedEnter(false); setTranslating(false); setLanguageMismatch(false);
  };

  useEffect(() => {
    const reload = () => { void api.getConfig().then(setConfig).catch(error => setStatus(String(error))); };
    reload(); input.current?.focus();
    const pending = listen("config-updated", reload);
    return () => { void pending.then(stop => stop()); };
  }, []);
  useEffect(() => {
    if (!content.current) return;
    const observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      if (rect) void api.resizeQuery(Math.ceil(rect.width) + 2, Math.ceil(rect.height) + 2);
    });
    observer.observe(content.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const pending = listen("clear-composed", () => { reset(); input.current?.focus(); });
    return () => { void pending.then(stop => stop()); };
  }, []);
  useEffect(() => {
    const id = ++lastRequested.current;
    if (!query.trim()) { setHits([]); setTotal(0); setSelected(0); setPage(0); setSearched(null); setLanguageMismatch(false); return; }
    setHits([]);
    setSearched(null);
    setSearchError("");
    api.search(query.trim(), page, pageSize).then(result => {
      if (id === lastRequested.current) {
        setHits(result.hits); setTotal(result.total); setSelected(0); setLanguageMismatch(result.language_mismatch);
        setSearched({ query: query.trim(), page });
      }
    }).catch(error => { if (id === lastRequested.current) setSearchError(String(error)); });
  }, [query, page, pageSize, config?.source_language, config?.translation_language]);
  useEffect(() => {
    const opened = listen("query-opened", () => {
      reset();
      void api.takePendingQuery().then(pending => { if (pending) setQuery(pending); });
      input.current?.focus();
    });
    const onFocus = () => {
      void api.takePendingQuery().then(pending => {
        if (pending) { setQuery(pending); setPage(0); }
        else if (config?.clear_on_focus_return) setQuery("");
        input.current?.focus();
      });
    };
    window.addEventListener("focus", onFocus);
    void api.takePendingQuery().then(pending => { if (pending) { setQuery(pending); setPage(0); } });
    return () => { window.removeEventListener("focus", onFocus); void opened.then(stop => stop()); };
  }, [config?.clear_on_focus_return]);

  const finish = async (next: SearchHit[]) => {
    try {
      await api.copy(
        next.map(item => item.key).join(" "),
        next.flatMap(item => item.library ? [{ library: item.library, key: item.key }] : []),
      );
      reset();
      await api.hideQuery();
    } catch (error) { setStatus(`复制失败：${String(error)}`); }
  };
  const append = (hit: SearchHit) => {
    ++translationRequest.current;
    setComposed(items => [...items, hit]);
    setQuery(""); setHits([]); setTotal(0); setSelected(0); setPage(0);
    setTranslation(""); setPreferTranslation(false); setStatus(""); setSearched(null); setUnmatchedEnter(false); setTranslating(false); setLanguageMismatch(false);
    input.current?.focus();
  };
  const translate = async () => {
    if (!query.trim() || translating) return;
    if (sameLanguage) {
      setStatus("原始语言和目标语言相同，无需 AI 翻译");
      return;
    }
    if (!config?.ai_translation || !config.translation_language) {
      setStatus("请先在设置中启用 AI 翻译并选择目标语言");
      return;
    }
    const request = ++translationRequest.current;
    setTranslating(true);
    setPreferTranslation(false);
    setStatus("");
    try {
      const result = await api.translate(query.trim());
      if (request === translationRequest.current) {
        setTranslation(result.translated);
        setPreferTranslation(true);
        setStatus(result.save_error ? `译文未加入词库：${result.save_error}` : "");
      }
    } catch (error) { if (request === translationRequest.current) setStatus(String(error)); }
    finally { if (request === translationRequest.current) setTranslating(false); }
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "ArrowDown" && visible.length) { event.preventDefault(); setSelected(preferTranslation ? 0 : Math.min(selected + 1, visible.length - 1)); setPreferTranslation(false); }
    else if (event.key === "ArrowUp" && visible.length) { event.preventDefault(); setSelected(preferTranslation ? 0 : Math.max(0, selected - 1)); setPreferTranslation(false); }
    else if (event.key === "ArrowRight" && page < pageCount - 1) { event.preventDefault(); setPreferTranslation(false); setPage(page + 1); setSelected(0); }
    else if (event.key === "ArrowLeft" && page > 0) { event.preventDefault(); setPreferTranslation(false); setPage(page - 1); setSelected(0); }
    else if (event.key === "Enter" && event.ctrlKey) { event.preventDefault(); void translate(); }
    else if (event.key === "Enter") {
      event.preventDefault();
      if (languageMismatch && query.trim()) return;
      if (translation && preferTranslation) void finish([...composed, { key: translation, value: query, rank: "exact" }]);
      else if (visible[selected]) void finish([...composed, visible[selected]]);
      else if (translation) void finish([...composed, { key: translation, value: query, rank: "exact" }]);
      else if (!query.trim() && composed.length) void finish(composed);
      else if (!query.trim()) void api.hideQuery();
      else if (!searching && !searchError && unmatchedEnter) { setQuery(""); setStatus(""); setUnmatchedEnter(false); }
      else if (!searching && !searchError) { setUnmatchedEnter(true); setStatus("没有匹配结果，再按一次 Enter 清空输入"); }
    } else if (event.key === "Backspace" && !query) {
      event.preventDefault();
      const next = event.ctrlKey ? [] : composed.slice(0, -1);
      setComposed(next);
    } else if (event.key === "Escape") { event.preventDefault(); void api.hideQuery(); }
    else if (event.key === " " && query.trim() && !event.ctrlKey && !event.altKey && !event.metaKey) {
      event.preventDefault();
      if (languageMismatch) return;
      if (translation && preferTranslation) append({ key: translation, value: query, rank: "exact" });
      else if (visible[selected]) append(visible[selected]);
      else if (translation) append({ key: translation, value: query, rank: "exact" });
      else if (!searching && !searchError) setStatus("没有匹配结果，当前文字未加入组合");
    }
  };

  const inputColor = config?.input.background_color ?? "#e8e8e8";
  const resultColor = config?.results.background_color ?? "#e8e8e8";
  return <main ref={content} className="inline-flex max-h-full flex-col overflow-hidden bg-transparent text-sm" style={{ color: config?.text_color ?? "#24272e", maxWidth: config?.results.max_width ?? 600 }}>
    <div className="flex shrink-0 items-center rounded-[var(--input-radius)] border border-slate-400/40 px-2 backdrop-blur-xl"
      style={{ height: config?.input.height ?? 30, width: config?.input.width ?? 200, "--input-radius": `${config?.input.corner_radius ?? 9}px`, backgroundColor: `${inputColor}${Math.round((config?.input.opacity ?? .2) * 255).toString(16).padStart(2, "0")}` } as React.CSSProperties}>
      <button type="button" aria-label="拖动查询窗口" className="mr-1 cursor-move select-none text-xs text-slate-500" onMouseDown={event => { event.preventDefault(); void api.dragQuery(); }}>⋮</button>
      <input ref={input} value={query} onChange={event => { ++translationRequest.current; setQuery(event.target.value); setPage(0); setTotal(0); setTranslation(""); setPreferTranslation(false); setTranslating(false); setStatus(""); setUnmatchedEnter(false); setLanguageMismatch(false); }} onKeyDown={onKeyDown}
        placeholder="输入文字…" className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-slate-500/70" style={{ fontSize: config?.input.font_size ?? 14 }} />
      <button type="button" aria-label="退出" className="ml-2 rounded-full px-1 text-lg text-slate-600 hover:bg-red-100 hover:text-red-700" onClick={() => void api.exit()}>×</button>
    </div>
    {(composed.length > 0 || visible.length > 0 || noMatch || canTranslate || languageMismatch || searchError || status || translation) && <div className="app-scrollbar w-max min-w-0 overflow-auto border border-slate-400/30 px-2 py-1 shadow-sm backdrop-blur-xl"
      style={{ marginTop: config?.gap ?? 4, borderRadius: config?.results.corner_radius ?? 7, minWidth: config?.results.min_width ?? 200, maxWidth: config?.results.max_width ?? 600, maxHeight: config?.results.max_height ?? 360, fontSize: config?.results.font_size ?? 12, backgroundColor: `${resultColor}${Math.round((config?.results.opacity ?? .2) * 255).toString(16).padStart(2, "0")}` }}>
      {composed.length > 0 && <p className="mb-1 border-b border-slate-500/20 py-1 text-xs text-slate-700">{composed.map(item => item.value ? `${item.key}（${item.value}）` : item.key).join(" · ")}</p>}
      {canTranslate && <button type="button" disabled={translating} onMouseDown={event => event.preventDefault()} onClick={() => void translate()}
        className="mx-2 my-1 rounded-md bg-white/75 px-2.5 py-1.5 text-left text-xs font-medium text-slate-800 hover:bg-white disabled:opacity-50">{translating ? "DeepSeek 翻译中…" : translation ? "重新使用 DeepSeek 翻译 · Ctrl+Enter" : "使用 DeepSeek 翻译 · Ctrl+Enter"}</button>}
      {translation && <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => void finish([...composed, { key: translation, value: query, rank: "exact" }])}
        className="mx-2 my-1 block max-w-full rounded-md px-2.5 py-1.5 text-left text-sm hover:bg-white/70"
        style={{ backgroundColor: preferTranslation ? `${config?.selected_color ?? "#A9CEFF"}${Math.round((config?.selected_opacity ?? .72) * 255).toString(16).padStart(2, "0")}` : undefined }}>
        <span className="mr-1 text-xs font-semibold">AI 译文</span>{translation}
      </button>}
      {visible.map((hit, index) => <button key={`${page}-${index}-${hit.key}`} type="button" onMouseDown={event => event.preventDefault()} onClick={() => void finish([...composed, hit])}
        className={`flex w-full items-baseline gap-2 rounded-md px-2 text-left ${index === selected && !preferTranslation ? "" : "hover:bg-white/50"}`}
        style={{ minHeight: config?.results.min_row_height ?? 28, paddingTop: 6, paddingBottom: 6, backgroundColor: index === selected && !preferTranslation ? `${config?.selected_color ?? "#A9CEFF"}${Math.round((config?.selected_opacity ?? .72) * 255).toString(16).padStart(2, "0")}` : undefined, color: index === selected && !preferTranslation ? config?.selected_text_color : undefined }}>
        <span className="min-w-0 shrink-0 font-semibold"><Highlight text={hit.key} query={query} /></span>
        <span className="min-w-0 truncate opacity-70"><Highlight text={hit.value} query={query} /></span>
      </button>)}
      {pageCount > 1 && <p className="px-2 py-1 text-right text-[11px] opacity-60">{page + 1} / {pageCount} · ← →</p>}
      {noMatch && <p className="px-2 py-1 text-xs text-slate-700">没有匹配结果</p>}
      {languageMismatch && <p className="px-2 py-1 text-xs text-amber-900">输入内容与所选语言不符，未加入结果</p>}
      {searchError && <p className="px-2 py-1.5 text-xs text-red-700">查询失败：{searchError}</p>}
      {status && <p className="px-2 py-1.5 text-xs text-slate-600">{status}</p>}
    </div>}
  </main>;
}
