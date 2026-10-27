import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type SearchHit, type UiConfig } from "../api";
import { availableVoices, findConfiguredVoice, findLocalVoice, findNarratorVoice, speechLanguage } from "../localSpeech";

function Highlight({ text, query }: { text: string; query: string }) {
  if (!query) return <>{text}</>;
  const index = text.toLocaleLowerCase().indexOf(query.toLocaleLowerCase());
  if (index < 0) return <>{text}</>;
  return <>{text.slice(0, index)}<mark className="rounded-sm bg-amber-200/80 text-inherit">{text.slice(index, index + query.length)}</mark>{text.slice(index + query.length)}</>;
}

function SpeakerIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" className="h-3.5 w-3.5">
    <path d="M11 5 6.5 9H3v6h3.5L11 19V5Z" />
    <path d="M15 9a4 4 0 0 1 0 6M18 6a8 8 0 0 1 0 12" />
  </svg>;
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
  const [translationPronunciation, setTranslationPronunciation] = useState("");
  const [speakingId, setSpeakingId] = useState<string | null>(null);
  const [preferTranslation, setPreferTranslation] = useState(false);
  const [searched, setSearched] = useState<{ query: string; page: number } | null>(null);
  const [searchError, setSearchError] = useState("");
  const [languageMismatch, setLanguageMismatch] = useState(false);
  const [unmatchedEnter, setUnmatchedEnter] = useState(false);
  const [translating, setTranslating] = useState(false);
  const [selectedTranslationPending, setSelectedTranslationPending] = useState(false);
  const selectionPendingRef = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLElement>(null);
  const lastWindowSize = useRef({ width: 0, height: 0 });
  const lastRequested = useRef(0);
  const translationRequest = useRef(0);
  const speechRequest = useRef(0);
  const localAudio = useRef<HTMLAudioElement | null>(null);
  const pageSize = config?.results.max_visible_results ?? 8;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const visible = useMemo(() => searched?.query === query.trim() && searched.page === page ? hits : [], [hits, page, query, searched]);
  const searching = !!query.trim() && (searched?.query !== query.trim() || searched.page !== page);
  const sameLanguage = !!config?.translation_language && config.source_language === config.translation_language;
  const noMatch = !!query.trim() && !searching && !searchError && !languageMismatch && total === 0 && !translation;
  const canTranslate = !!query.trim() && !!config?.translation_language
    && (selectedTranslationPending || (!sameLanguage && !!config.ai_translation));

  const stopSpeech = () => {
    ++speechRequest.current;
    window.speechSynthesis?.cancel();
    localAudio.current?.pause();
    localAudio.current = null;
    setSpeakingId(null);
  };

  const playSpeech = async (id: string, text: string) => {
    if (speakingId === id) { stopSpeech(); return; }
    stopSpeech();
    const request = speechRequest.current;
    const language = speechLanguage(config?.translation_language ?? "", text);
    const configuredUri = config?.speech_voices?.[language.toLocaleLowerCase()] ?? config?.speech_voices?.[language.toLocaleLowerCase().split("-")[0]];
    if (configuredUri?.startsWith("piper:") || configuredUri?.startsWith("supertonic:") || configuredUri?.startsWith("winrt:")) {
      setSpeakingId(id);
      setStatus("正在生成本地发音…");
      try {
        const source = configuredUri.startsWith("piper:") || configuredUri.startsWith("supertonic:")
          ? await api.synthesizePiper(configuredUri, text)
          : await api.synthesizeWindows(configuredUri, text);
        if (request !== speechRequest.current) return;
        const audio = new Audio(source);
        localAudio.current = audio;
        audio.onended = () => { if (request === speechRequest.current) { localAudio.current = null; setSpeakingId(null); } };
        audio.onerror = () => { if (request === speechRequest.current) { localAudio.current = null; setSpeakingId(null); setStatus("本地发音播放失败"); } };
        await audio.play();
        if (request === speechRequest.current) setStatus("");
        input.current?.focus();
      } catch (error) {
        if (request === speechRequest.current) { localAudio.current = null; setSpeakingId(null); setStatus(`本地发音失败：${String(error)}`); }
      }
      return;
    }
    const synthesis = window.speechSynthesis;
    if (!synthesis || typeof SpeechSynthesisUtterance === "undefined") {
      setStatus("当前 WebView2 不支持语音播放");
      return;
    }
    let voice: SpeechSynthesisVoice | undefined;
    let selectedNarratorVoice: string | null = null;
    try {
      const [voices, narratorVoice] = await Promise.all([availableVoices(synthesis), configuredUri ? Promise.resolve(null) : api.narratorVoice().catch(() => null)]);
      selectedNarratorVoice = narratorVoice;
      voice = findConfiguredVoice(voices, configuredUri, language)
        ?? findNarratorVoice(voices, narratorVoice, language)
        ?? findLocalVoice(voices, language);
    } catch {
      if (request === speechRequest.current) setStatus("无法读取 Windows 语音设置");
      return;
    }
    if (request !== speechRequest.current) return;
    if (!voice) {
      setStatus(`未找到 ${language} 的本地语音，请在 Windows 语言设置中安装对应的文字转语音组件`);
      return;
    }
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.voice = voice;
    utterance.lang = voice.lang;
    utterance.onend = () => { if (request === speechRequest.current) setSpeakingId(null); };
    utterance.onerror = () => { if (request === speechRequest.current) { setSpeakingId(null); setStatus("读音播放失败，请检查 Windows 语音组件"); } };
    setStatus(configuredUri && !findConfiguredVoice([voice], configuredUri, language)
      ? "所选发音语音已不可用，已使用其他本地声音"
      : selectedNarratorVoice && !findNarratorVoice([voice], selectedNarratorVoice, language)
        ? "讲述人所选声音不可用于此语言的应用播放，已使用可用的本地声音"
        : "");
    setSpeakingId(id);
    try {
      synthesis.speak(utterance);
      input.current?.focus();
    } catch {
      if (request === speechRequest.current) { setSpeakingId(null); setStatus("读音播放失败，请检查 Windows 语音组件"); }
    }
  };

  const audioButton = (id: string, text: string) => <button type="button" aria-label={`${speakingId === id ? "停止" : "播放"} ${text} 的读音`}
    aria-pressed={speakingId === id} title={speakingId === id ? "停止播放" : "播放读音"}
    onMouseDown={event => event.preventDefault()} onClick={() => void playSpeech(id, text)}
    className={`inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full transition-colors ${speakingId === id ? "bg-blue-100 text-blue-700" : "text-slate-600 hover:bg-white/75 hover:text-blue-700"}`}>
    <SpeakerIcon />
  </button>;

  const reset = () => {
    selectionPendingRef.current = false;
    setSelectedTranslationPending(false);
    stopSpeech();
    ++lastRequested.current;
    ++translationRequest.current;
    setQuery(""); setHits([]); setTotal(0); setSelected(0); setPage(0);
    setComposed([]); setTranslation(""); setTranslationPronunciation(""); setPreferTranslation(false); setStatus(""); setSearchError("");
    setSearched(null); setUnmatchedEnter(false); setTranslating(false); setLanguageMismatch(false);
  };

  const fitWindow = (force = false) => {
    const element = content.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    const width = Math.ceil(Math.max(rect.width, element.scrollWidth) * scale);
    const height = Math.ceil(Math.max(rect.height, element.scrollHeight) * scale);
    if (width <= 0 || height <= 0) return;
    if (!force && width === lastWindowSize.current.width && height === lastWindowSize.current.height) return;
    lastWindowSize.current = { width, height };
    void api.resizeQuery(width, height);
  };

  useLayoutEffect(() => { fitWindow(); });

  useEffect(() => {
    const reload = () => { void api.getConfig().then(setConfig).catch(error => setStatus(String(error))); };
    reload(); input.current?.focus();
    const pending = listen("config-updated", reload);
    const toggled = listen<boolean>("mic-tts-toggle", event => setConfig(current => current && { ...current, mic_tts_enabled: event.payload }));
    const micStatus = listen<string>("mic-tts-status", event => setStatus(event.payload));
    return () => { window.speechSynthesis?.cancel(); localAudio.current?.pause(); void pending.then(stop => stop()); void toggled.then(stop => stop()); void micStatus.then(stop => stop()); };
  }, []);
  useEffect(() => {
    if (!content.current) return;
    const observer = new ResizeObserver(() => fitWindow());
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
  const beginSelectedTranslation = (text: string) => {
    selectionPendingRef.current = true;
    setSelectedTranslationPending(true);
    setQuery(text);
    setPage(0);
    setTranslation("");
    setTranslationPronunciation("");
    setPreferTranslation(false);
    setTranslating(true);
    setStatus("正在翻译选中文字…");
    const request = ++translationRequest.current;
    void api.translateSelection(text).then(result => {
      if (request !== translationRequest.current) return;
      setTranslation(result.translated);
      setTranslationPronunciation(result.pronunciation);
      setPreferTranslation(true);
      setStatus("");
    }).catch(error => {
      if (request === translationRequest.current) setStatus(`选中文字翻译失败：${String(error)}`);
    }).finally(() => {
      if (request === translationRequest.current) {
        setTranslating(false);
      }
    });
  };
  useEffect(() => {
    const opened = listen("query-opened", () => {
      reset();
      void api.takePendingSelectedTranslation().then(selected => {
        if (selected) beginSelectedTranslation(selected);
        else {
          void api.takePendingQuery().then(pending => { if (pending) setQuery(pending); });
        }
      });
      input.current?.focus();
      requestAnimationFrame(() => fitWindow(true));
    });
    const onFocus = () => {
      if (selectionPendingRef.current) return;
      void api.takePendingQuery().then(pending => {
        if (selectionPendingRef.current) return;
        if (pending) { setQuery(pending); setPage(0); }
        input.current?.focus();
      });
    };
    window.addEventListener("focus", onFocus);
    const selectedError = listen<string>("selected-text-error", event => setStatus(event.payload));
    void api.takePendingQuery().then(pending => { if (pending) { setQuery(pending); setPage(0); } });
    return () => { window.removeEventListener("focus", onFocus); void opened.then(stop => stop()); void selectedError.then(stop => stop()); };
  }, []);

  const finish = async (next: SearchHit[]) => {
    stopSpeech();
    try {
      await api.copy(
        next.map(item => item.key).join(" "),
        next.flatMap(item => item.library ? [{ library: item.library, key: item.key }] : []),
      );
      reset();
      await api.hideQuery();
    } catch (error) { setStatus(`复制失败：${String(error)}`); }
  };
  const closeQuery = () => { stopSpeech(); void api.hideQuery(); };
  const append = (hit: SearchHit) => {
    stopSpeech();
    ++translationRequest.current;
    selectionPendingRef.current = false;
    setSelectedTranslationPending(false);
    setComposed(items => [...items, hit]);
    setQuery(""); setHits([]); setTotal(0); setSelected(0); setPage(0);
    setTranslation(""); setTranslationPronunciation(""); setPreferTranslation(false); setStatus(""); setSearched(null); setUnmatchedEnter(false); setTranslating(false); setLanguageMismatch(false);
    input.current?.focus();
  };
  const translate = async () => {
    if (!query.trim() || translating) return;
    if (sameLanguage && !selectedTranslationPending) {
      setStatus("原始语言和目标语言相同，无需 AI 翻译");
      return;
    }
    if ((!config?.ai_translation && !selectedTranslationPending) || !config?.translation_language) {
      setStatus("请先在设置中启用 AI 翻译并选择目标语言");
      return;
    }
    const request = ++translationRequest.current;
    setTranslating(true);
    setPreferTranslation(false);
    setStatus("");
    try {
      const result = selectedTranslationPending
        ? await api.translateSelection(query.trim())
        : await api.translate(query.trim());
      if (request === translationRequest.current) {
        setTranslation(result.translated);
        setTranslationPronunciation(result.pronunciation);
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
      if (languageMismatch && query.trim() && !(translation && preferTranslation)) return;
      if (translation && preferTranslation) void finish([...composed, { key: translation, value: query, rank: "exact" }]);
      else if (visible[selected]) void finish([...composed, visible[selected]]);
      else if (translation) void finish([...composed, { key: translation, value: query, rank: "exact" }]);
      else if (!query.trim() && composed.length) void finish(composed);
      else if (!query.trim()) closeQuery();
      else if (!searching && !searchError && unmatchedEnter) { setQuery(""); setStatus(""); setUnmatchedEnter(false); }
      else if (!searching && !searchError) { setUnmatchedEnter(true); setStatus("没有匹配结果，再按一次 Enter 清空输入"); }
    } else if (event.key === "Backspace" && !query) {
      event.preventDefault();
      const next = event.ctrlKey ? [] : composed.slice(0, -1);
      setComposed(next);
    } else if (event.key === "Escape") { event.preventDefault(); closeQuery(); }
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
  return <main ref={content} className="query-popup flex w-max flex-col bg-transparent p-[2px] text-sm" style={{ color: config?.text_color ?? "#24272e", maxWidth: (config?.results.max_width ?? 600) + 4 }}>
    <div className="flex shrink-0 items-center rounded-[var(--input-radius)] border border-slate-400/40 px-2"
      style={{ height: config?.input.height ?? 30, width: config?.input.width ?? 200, "--input-radius": `${config?.input.corner_radius ?? 9}px`, backgroundColor: `${inputColor}${Math.round((config?.input.opacity ?? .2) * 255).toString(16).padStart(2, "0")}` } as React.CSSProperties}>
      <button type="button" aria-label="拖动查询窗口" className="mr-1 cursor-move select-none text-xs text-slate-500" onMouseDown={event => { event.preventDefault(); void api.dragQuery(); }}>⋮</button>
      <input ref={input} value={query} onChange={event => { stopSpeech(); ++translationRequest.current; selectionPendingRef.current = false; setSelectedTranslationPending(false); setQuery(event.target.value); setPage(0); setTotal(0); setTranslation(""); setTranslationPronunciation(""); setPreferTranslation(false); setTranslating(false); setStatus(""); setUnmatchedEnter(false); setLanguageMismatch(false); }} onKeyDown={onKeyDown}
        placeholder="输入文字…" className="h-full min-w-0 flex-1 bg-transparent px-0 outline-none placeholder:text-slate-500/70" style={{ fontSize: config?.input.font_size ?? 14 }} />
      <button type="button" aria-label="退出" className="ml-2 rounded-full px-1 text-lg text-slate-600 hover:bg-red-100 hover:text-red-700" onClick={() => void api.exit()}>×</button>
    </div>
    {(composed.length > 0 || visible.length > 0 || noMatch || canTranslate || languageMismatch || searchError || status || translation) && <div className="app-scrollbar w-max min-w-0 overflow-auto border border-slate-400/30 px-2 py-1"
      style={{ marginTop: config?.gap ?? 4, borderRadius: config?.results.corner_radius ?? 7, minWidth: config?.results.min_width ?? 200, maxWidth: config?.results.max_width ?? 600, maxHeight: config?.results.max_height ?? 360, fontSize: config?.results.font_size ?? 12, backgroundColor: `${resultColor}${Math.round((config?.results.opacity ?? .2) * 255).toString(16).padStart(2, "0")}` }}>
      {composed.length > 0 && <p className="mb-1 border-b border-slate-500/20 py-1 text-xs text-slate-700">{composed.map(item => item.value ? `${item.key}（${item.value}）` : item.key).join(" · ")}</p>}
      {canTranslate && <button type="button" disabled={translating} onMouseDown={event => event.preventDefault()} onClick={() => void translate()}
        className="mx-2 my-1 rounded-md bg-white/75 px-2.5 py-1.5 text-left text-xs font-medium text-slate-800 hover:bg-white disabled:opacity-50">{translating ? "DeepSeek 翻译中…" : selectedTranslationPending ? "重新翻译选中文字 · Ctrl+Enter" : translation ? "重新使用 DeepSeek 翻译 · Ctrl+Enter" : "使用 DeepSeek 翻译 · Ctrl+Enter"}</button>}
      {translation && <div className="mx-2 my-1 max-w-full min-w-0 rounded-md px-2.5 py-1.5 hover:bg-white/70"
        style={{ backgroundColor: preferTranslation ? `${config?.selected_color ?? "#A9CEFF"}${Math.round((config?.selected_opacity ?? .72) * 255).toString(16).padStart(2, "0")}` : undefined }}>
        <div className="flex min-w-0 items-center gap-1.5">
          <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => void finish([...composed, { key: translation, value: query, pronunciation: translationPronunciation, rank: "exact" }])}
            className="min-w-0 flex-1 text-left text-sm"><span className="mr-1 text-xs font-semibold">AI 译文</span>{translation}</button>
          {!translationPronunciation && audioButton("ai", translation)}
        </div>
        {translationPronunciation && <div className="flex min-w-0 items-center gap-1 pl-1 text-[11px] opacity-75">
          <span className="min-w-0 truncate" title={translationPronunciation}>{translationPronunciation}</span>{audioButton("ai", translation)}
        </div>}
      </div>}
      {visible.map((hit, index) => <div key={`${page}-${index}-${hit.key}`} className={`w-full min-w-0 rounded-md px-2 text-left ${index === selected && !preferTranslation ? "" : "hover:bg-white/50"}`}
        style={{ minHeight: config?.results.min_row_height ?? 28, paddingTop: 6, paddingBottom: 6, backgroundColor: index === selected && !preferTranslation ? `${config?.selected_color ?? "#A9CEFF"}${Math.round((config?.selected_opacity ?? .72) * 255).toString(16).padStart(2, "0")}` : undefined, color: index === selected && !preferTranslation ? config?.selected_text_color : undefined }}>
        <div className="flex min-w-0 items-center gap-2">
          <button type="button" onMouseDown={event => event.preventDefault()} onClick={() => void finish([...composed, hit])} className="flex min-w-0 flex-1 items-baseline gap-2 text-left">
            <span className="max-w-[48%] min-w-0 truncate font-semibold" title={hit.key}><Highlight text={hit.key} query={query} /></span>
            <span className="min-w-0 flex-1 truncate opacity-70" title={hit.value}><Highlight text={hit.value} query={query} /></span>
          </button>
          {!hit.pronunciation && audioButton(`${page}-${index}-${hit.key}`, hit.key)}
        </div>
        {hit.pronunciation && <div className="flex min-w-0 items-center gap-1 pl-1 text-[11px] opacity-75">
          <span className="min-w-0 truncate" title={hit.pronunciation}>{hit.pronunciation}</span>
          {audioButton(`${page}-${index}-${hit.key}`, hit.key)}
        </div>}
      </div>)}
      {pageCount > 1 && <p className="px-2 py-1 text-right text-[11px] opacity-60">{page + 1} / {pageCount} · ← →</p>}
      {noMatch && <p className="px-2 py-1 text-xs text-slate-700">没有匹配结果</p>}
      {languageMismatch && <p className="px-2 py-1 text-xs text-amber-900">输入内容与所选语言不符，未加入结果</p>}
      {searchError && <p className="px-2 py-1.5 text-xs text-red-700">查询失败：{searchError}</p>}
      {status && <p className="px-2 py-1.5 text-xs text-slate-600">{status}</p>}
    </div>}
  </main>;
}
