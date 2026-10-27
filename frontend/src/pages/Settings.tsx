import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type AudioOutputDevice, type LanguageOption, type LocalTtsVoice, type UiConfig } from "../api";
import { t } from "../i18n";
import { availableVoices, findConfiguredVoice, findLocalVoice, speechLanguage } from "../localSpeech";
import { LibraryPage } from "./Library";

type Section = "library" | "ai" | "voice" | "shortcuts" | "appearance";
const sections: { id: Section; title: string }[] = [
  { id: "library", title: "词库" },
  { id: "ai", title: "AI 翻译 · DeepSeek" },
  { id: "voice", title: "语音识别" },
  { id: "shortcuts", title: "快捷键" },
  { id: "appearance", title: "通用外观" },
];

function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (value: boolean) => void }) {
  return <label className="flex min-h-10 items-center justify-between gap-4 text-sm text-slate-700">
    <span>{label}</span>
    <button type="button" role="switch" aria-checked={checked} onClick={() => onChange(!checked)}
      className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${checked ? "bg-violet-500" : "bg-slate-400"}`}>
      <span className={`absolute top-[3px] left-[3px] h-[18px] w-[18px] rounded-full border-[4px] border-white transition-transform ${checked ? "translate-x-5 bg-white" : "bg-transparent"}`} />
    </button>
  </label>;
}

function Slider({ label, value, min, max, step, unit, onChange }: { label: string; value: number; min: number; max: number; step: number; unit?: string; onChange: (value: number) => void }) {
  return <label className="grid gap-2 text-sm text-slate-700">
    <span className="flex justify-between gap-4"><span>{label}</span><strong className="tabular-nums text-slate-900">{value}{unit}</strong></span>
    <input type="range" value={value} min={min} max={max} step={step} onChange={event => onChange(Number(event.target.value))}
      className="setting-range w-full cursor-pointer" style={{ "--range-progress": `${((value - min) / (max - min)) * 100}%` } as React.CSSProperties} />
  </label>;
}

function ColorField({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return <label className="flex items-center justify-between gap-3 text-sm text-slate-700">
    {label}<span className="flex items-center gap-2 rounded-xl border border-slate-200 bg-white px-2 py-1">
      <input type="color" value={value} onChange={event => onChange(event.target.value)} className="h-7 w-8 cursor-pointer border-0 bg-transparent" />
      <span className="w-16 font-mono text-xs uppercase text-slate-500">{value}</span>
    </span>
  </label>;
}

function ShortcutField({ label, value, onChange, recording, onRecordingChange }: { label: string; value: string; onChange: (value: string) => void; recording: boolean; onRecordingChange: (active: boolean) => void }) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  useEffect(() => {
    if (!recording) return;
    const pending = listen<string>("mouse-shortcut", event => {
      onChangeRef.current(event.payload);
      onRecordingChange(false);
      void api.setShortcutRecording(false);
    });
    return () => { void pending.then(stop => stop()); };
  }, [recording]);
  return <div className="flex min-h-12 items-center justify-between gap-3 text-sm text-slate-700">
    <span>{label}</span>
    <button type="button" className={`button min-w-40 font-mono text-xs ${recording ? "border-blue-500 ring-2 ring-blue-100" : ""}`}
      onClick={() => { onRecordingChange(true); void api.setShortcutRecording(true); }}
      onKeyDown={event => {
        if (!recording) return;
        event.preventDefault(); event.stopPropagation();
        if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) return;
        if (event.key === "Escape") { onRecordingChange(false); void api.setShortcutRecording(false); return; }
        const modifiers = [event.ctrlKey && "ctrl", event.altKey && "alt", event.shiftKey && "shift", event.metaKey && "super"].filter(Boolean);
        const key = event.key === " " ? "space" : event.key.toLowerCase();
        onChange([...modifiers, key].join("+")); onRecordingChange(false); void api.setShortcutRecording(false);
      }}>
      {recording ? "按下快捷键…" : value || "点击录制"}
    </button>
  </div>;
}

export function SettingsPage() {
  const [config, setConfig] = useState<UiConfig | null>(null);
  const uiLanguage = useRef("zh-CN");
  uiLanguage.current = config?.ui_language ?? "zh-CN";
  const [languages, setLanguages] = useState<LanguageOption[]>([]);
  const [active, setActive] = useState<Section>("library");
  const [editingWords, setEditingWords] = useState(false);
  const [notice, setNoticeState] = useState("");
  const [noticeRevision, setNoticeRevision] = useState(0);
  const [noticeDuration, setNoticeDuration] = useState(2400);
  const setNotice = (value: string, duration = 2400) => { setNoticeState(value); setNoticeDuration(duration); setNoticeRevision(current => current + 1); };
  const [busy, setBusy] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const [hasApiKey, setHasApiKey] = useState(false);
  const [voiceRegistered, setVoiceRegistered] = useState(false);
  const [voiceEnrolling, setVoiceEnrolling] = useState(false);
  const [voiceEnrollmentStatus, setVoiceEnrollmentStatus] = useState("");
  const [recordingShortcut, setRecordingShortcut] = useState<keyof UiConfig["hotkeys"] | null>(null);
  const scrollArea = useRef<HTMLDivElement>(null);
  const [showScrollTop, setShowScrollTop] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState("");
  const [playbackVoices, setPlaybackVoices] = useState<SpeechSynthesisVoice[] | null>(null);
  const [localTtsVoices, setLocalTtsVoices] = useState<LocalTtsVoice[]>([]);
  const [windowsTtsVoices, setWindowsTtsVoices] = useState<LocalTtsVoice[]>([]);
  const [audioDevices, setAudioDevices] = useState<AudioOutputDevice[]>([]);
  const [playbackError, setPlaybackError] = useState("");
  const previewAudio = useRef<HTMLAudioElement | null>(null);
  const micTogglePending = useRef(false);

  useEffect(() => {
    api.getConfig().then(setConfig).catch(error => setNotice(String(error)));
    api.languages().then(setLanguages).catch(() => {});
    api.hasApiKey().then(setHasApiKey).catch(error => setNotice(String(error)));
    api.voiceRegistered().then(setVoiceRegistered).catch(() => {});
  }, []);
  useEffect(() => {
    const cancelRecording = () => { setRecordingShortcut(null); void api.setShortcutRecording(false); };
    window.addEventListener("blur", cancelRecording);
    return () => { window.removeEventListener("blur", cancelRecording); void api.setShortcutRecording(false); };
  }, []);
  useEffect(() => {
    const pending = listen<string>("voice-enrollment", event => setVoiceEnrollmentStatus(event.payload));
    return () => { void pending.then(stop => stop()); };
  }, []);
  useEffect(() => {
    const state = listen<boolean>("mic-tts-toggle", event =>
      setConfig(current => current && { ...current, mic_tts_enabled: event.payload }));
    const status = listen<string>("mic-tts-status", event => setNotice(event.payload));
    const subtitleState = listen<boolean>("system-audio-toggle", event =>
      setConfig(current => current && { ...current, system_audio_monitor: event.payload, system_audio_translate: event.payload }));
    const subtitleStatus = listen<string>("system-audio-status", event => setNotice(t(event.payload, uiLanguage.current)));
    const voiceCommand = listen<string[]>("voice-command-applied", event => {
      void api.getConfig().then(saved => setConfig(current => {
        if (!current) return saved;
        const next = { ...current, input: { ...current.input }, results: { ...current.results }, hotkeys: { ...current.hotkeys } };
        for (const key of event.payload) {
          if (key === "max_visible_results") next.results.max_visible_results = saved.results.max_visible_results;
          else if (key.startsWith("input.")) {
            const field = key.slice("input.".length);
            (next.input as unknown as Record<string, unknown>)[field] = (saved.input as unknown as Record<string, unknown>)[field];
          } else if (key.startsWith("results.")) {
            const field = key.slice("results.".length);
            (next.results as unknown as Record<string, unknown>)[field] = (saved.results as unknown as Record<string, unknown>)[field];
          } else if (key.startsWith("hotkeys.")) {
            const field = key.slice("hotkeys.".length);
            (next.hotkeys as unknown as Record<string, unknown>)[field] = (saved.hotkeys as unknown as Record<string, unknown>)[field];
          }
          else if (key in saved) (next as unknown as Record<string, unknown>)[key] = (saved as unknown as Record<string, unknown>)[key];
        }
        return next;
      })).catch(() => {});
    });
    const refresh = () => void api.getConfig().then(saved =>
      setConfig(current => current && { ...current, mic_tts_enabled: saved.mic_tts_enabled,
        system_audio_monitor: saved.system_audio_monitor, system_audio_translate: saved.system_audio_translate })).catch(() => {});
    window.addEventListener("focus", refresh);
    return () => { void state.then(stop => stop()); void status.then(stop => stop());
      void subtitleState.then(stop => stop()); void subtitleStatus.then(stop => stop());
      void voiceCommand.then(stop => stop()); window.removeEventListener("focus", refresh); };
  }, []);
  useEffect(() => { if (!notice) return; const timer = window.setTimeout(() => setNotice(""), noticeDuration); return () => clearTimeout(timer); }, [notice, noticeRevision, noticeDuration]);
  useEffect(() => { scrollArea.current?.scrollTo({ top: 0, behavior: "auto" }); setShowScrollTop(false); }, [active]);
  useEffect(() => {
    if (active === "voice" && config) void api.voiceEngineStatus(config.voice_backend, config.source_language, config.translation_language).then(setVoiceStatus).catch(error => setVoiceStatus(String(error)));
  }, [active, config?.voice_backend, config?.source_language, config?.translation_language]);
  useEffect(() => {
    if (active !== "voice") return;
    void api.localTtsVoices().then(setLocalTtsVoices).catch(error => setPlaybackError(String(error)));
    void api.windowsTtsVoices().then(setWindowsTtsVoices).catch(error => setPlaybackError(String(error)));
    void api.audioOutputDevices().then(setAudioDevices).catch(error => setPlaybackError(String(error)));
    const synthesis = window.speechSynthesis;
    if (!synthesis) { setPlaybackVoices([]); return; }
    let cancelled = false;
    void availableVoices(synthesis).then(voices => {
      if (!cancelled) { setPlaybackVoices(voices.filter(voice => voice.localService)); setPlaybackError(""); }
    }).catch(error => { if (!cancelled) { setPlaybackVoices([]); setPlaybackError(String(error)); } });
    return () => { cancelled = true; };
  }, [active]);
  useEffect(() => () => { previewAudio.current?.pause(); window.speechSynthesis?.cancel(); }, []);

  if (!config) return <div className="grid h-full place-items-center bg-[#eeedea] text-sm text-slate-600">{notice || "正在加载设置…"}</div>;
  const label = (key: string) => t(key, config.ui_language);
  const update = <K extends keyof UiConfig>(key: K, value: UiConfig[K]) => setConfig(current => current && { ...current, [key]: value });
  const changeMicTts = async (enabled: boolean) => {
    if (micTogglePending.current) return;
    micTogglePending.current = true;
    try {
      const applied = await api.setMicTtsEnabled(enabled, config.mic_output_device);
      setConfig(current => current && { ...current, mic_tts_enabled: applied });
    } catch (error) { setNotice(String(error)); }
    finally { micTogglePending.current = false; }
  };
  const updatePart = <K extends "input" | "results" | "hotkeys">(part: K, key: keyof UiConfig[K], value: string | number) =>
    setConfig(current => current && { ...current, [part]: { ...current[part], [key]: value } });
  const playbackLanguage = config.translation_language ? speechLanguage(config.translation_language, "") : "";
  const playbackKey = playbackLanguage.toLocaleLowerCase();
  const selectedPlaybackUri = config.speech_voices?.[playbackKey] ?? "";
  const voicesForLanguage = playbackVoices?.filter(voice => !playbackLanguage
    || voice.lang.toLocaleLowerCase().split("-")[0] === playbackKey.split("-")[0]) ?? [];
  const localVoicesForLanguage = [...localTtsVoices, ...windowsTtsVoices].filter(voice => !playbackLanguage
    || voice.lang.toLocaleLowerCase().split("-")[0] === playbackKey.split("-")[0]
    || (playbackKey.startsWith("zh-") && voice.lang.toLocaleLowerCase().startsWith("yue-")));
  const choosePlaybackVoice = (uri: string) => setConfig(current => {
    if (!current) return current;
    const speech_voices = { ...current.speech_voices };
    if (uri) speech_voices[playbackKey] = uri;
    else delete speech_voices[playbackKey];
    return { ...current, speech_voices };
  });
  const previewPlaybackVoice = async () => {
    previewAudio.current?.pause();
    previewAudio.current = null;
    window.speechSynthesis?.cancel();
    if (!playbackLanguage) return;
    const selectedVoice = [...localTtsVoices, ...windowsTtsVoices].find(voice => voice.id === selectedPlaybackUri);
    const sample = selectedVoice?.lang.toLocaleLowerCase().startsWith("yue-") ? "你好，今日天氣好好。"
      : playbackKey.startsWith("ko") ? "안녕하세요. 반갑습니다."
      : playbackKey.startsWith("ja") ? "こんにちは。よろしくお願いします。"
        : playbackKey.startsWith("zh") ? "你好，很高兴认识你。"
          : "Hello, nice to meet you.";
    if (selectedPlaybackUri.startsWith("piper:") || selectedPlaybackUri.startsWith("supertonic:")) {
      try {
        const source = await api.synthesizePiper(selectedPlaybackUri, sample);
        const audio = new Audio(source);
        previewAudio.current = audio;
        await audio.play();
      } catch (error) { setNotice(`本地发音失败：${String(error)}`); }
      return;
    }
    if (selectedPlaybackUri.startsWith("winrt:")) {
      try {
        const source = await api.synthesizeWindows(selectedPlaybackUri, sample);
        const audio = new Audio(source);
        previewAudio.current = audio;
        await audio.play();
      } catch (error) { setNotice(`Windows 发音失败：${String(error)}`); }
      return;
    }
    const synthesis = window.speechSynthesis;
    if (!synthesis) return;
    const voice = findConfiguredVoice(voicesForLanguage, selectedPlaybackUri, playbackLanguage)
      ?? findLocalVoice(voicesForLanguage, playbackLanguage);
    if (!voice) return;
    const utterance = new SpeechSynthesisUtterance(sample);
    utterance.voice = voice;
    utterance.lang = voice.lang;
    synthesis.speak(utterance);
  };

  const save = async () => {
    setBusy(true);
    try { await api.saveConfig(config); setNotice(label("配置已保存并立即生效。"), 1200); }
    catch (error) { setNotice(String(error)); }
    finally { setBusy(false); }
  };

  if (editingWords) return <LibraryPage language={config.ui_language} source={config.source_language === "auto" ? "cn" : config.source_language} target={config.translation_language} onBack={() => { setEditingWords(false); setShowScrollTop(false); }} />;

  return <main className="flex h-full min-w-[650px] flex-col bg-[#eeedea] text-slate-800">
    <header className="flex shrink-0 items-center justify-end border-b border-slate-300/70 px-8 py-4">
      <label className="flex items-center gap-3 text-sm text-slate-600">
        <span className="flex shrink-0 flex-col items-start leading-tight">
          <span className="whitespace-nowrap">{label("界面语言")}</span>
          {config.ui_language === "zh-CN" && <span className="whitespace-nowrap text-xs">(language)</span>}
        </span>
        <select className="field w-38" value={config.ui_language} onChange={event => update("ui_language", event.target.value)}>
          <option value="zh-CN">简体中文</option><option value="en">English</option><option value="ko">한국어</option><option value="ja">日本語</option>
        </select>
      </label>
    </header>
    <div className="flex min-h-0 flex-1">
      <nav className="w-44 shrink-0 border-r border-slate-300/70 px-3 py-6">
        {sections.map(section => <button key={section.id} type="button" onClick={() => setActive(section.id)}
          className={`mb-1 w-full rounded-xl px-3 py-2.5 text-left text-sm transition-colors ${active === section.id ? "bg-white font-semibold text-slate-900 shadow-sm" : "text-slate-600 hover:bg-white/60"}`}>
          {label(section.title)}
        </button>)}
      </nav>
      <div ref={scrollArea} onScroll={event => setShowScrollTop(event.currentTarget.scrollTop > 30)} className="app-scrollbar min-w-0 flex-1 overflow-y-auto px-8 py-6">
        <section className="surface-card mx-auto max-w-3xl p-6">
          <h1 className="mb-6 text-xl font-semibold tracking-tight">{label(sections.find(section => section.id === active)?.title ?? "设置")}</h1>
          {active === "library" && <div className="grid gap-5">
            <label className="grid gap-2 text-sm">{label("原始语言")}
              <select className="field" value={config.source_language} onChange={event => update("source_language", event.target.value)}>
                <option value="auto">{label("自动识别")}</option>{languages.map(item => <option key={item.code} value={item.code}>{item.name}</option>)}
              </select>
            </label>
            <label className="grid gap-2 text-sm">{label("目标语言")}
              <select className="field" value={config.translation_language} onChange={event => update("translation_language", event.target.value)}>
                <option value="">{label("未选择（默认词库）")}</option>
                {languages.map(item => <option key={item.code} value={item.code}>{item.name}</option>)}
              </select>
            </label>
            <button type="button" className="button w-fit" onClick={() => setEditingWords(true)}>{label("编辑词库")}</button>
          </div>}
          {active === "ai" && <div className="grid gap-3">
            <Switch label={label("启用 AI 翻译")} checked={config.ai_translation} onChange={value => update("ai_translation", value)} />
            <Switch label={label("翻译后自动加入词库")} checked={config.ai_auto_save} onChange={value => update("ai_auto_save", value)} />
            <div className="grid gap-2 text-sm text-slate-700">
              <span>{label("敬语模式")}</span>
              <div role="group" aria-label={label("敬语模式")} className="grid grid-cols-3 gap-1 rounded-xl border border-slate-300 bg-slate-100 p-1">
                {(["informal", "conversational", "formal"] as const).map((level, index) =>
                  <button key={level} type="button" aria-pressed={config.ai_politeness === level}
                    onClick={() => update("ai_politeness", level)}
                    className={`min-h-9 rounded-lg px-2 py-1.5 text-xs font-medium transition-colors ${config.ai_politeness === level ? "bg-white text-slate-900 shadow-sm" : "text-slate-600 hover:bg-white/60"}`}>
                    {label(["非敬语", "口语化敬语", "官方化敬语"][index])}
                  </button>)}
              </div>
            </div>
            <Switch label={label("OCR识别后自动翻译成原始语言")} checked={config.ocr_auto_translate} onChange={value => update("ocr_auto_translate", value)} />
            <Switch label={label("截图翻译同时显示原文")} checked={config.ocr_show_original} onChange={value => update("ocr_show_original", value)} />
            <div className="grid gap-2 border-t border-slate-200 pt-4">
              <label className="text-sm text-slate-700" htmlFor="deepseek-key">{label("Base64 编码的 DeepSeek API Key")} {hasApiKey ? `· ${label("已保存")}` : ""}</label>
              <div className="flex gap-2">
                <input id="deepseek-key" className="field min-w-0 flex-1" type="password" autoComplete="off" value={apiKey}
                  onChange={event => setApiKey(event.target.value)} placeholder={hasApiKey ? "••••••••••••" : label("粘贴 Base64 字符串")} />
                <button type="button" className="button" disabled={!apiKey.trim()} onClick={async () => {
                  try { await api.setApiKey(apiKey); setApiKey(""); setHasApiKey(true); setNotice(label("API Key 已保存")); }
                  catch (error) { setNotice(String(error)); }
                }}>{label("保存密钥")}</button>
                {hasApiKey && <button type="button" className="button" onClick={async () => {
                  try { await api.deleteApiKey(); setHasApiKey(false); setNotice(label("密钥已删除")); }
                  catch (error) { setNotice(String(error)); }
                }}>{label("删除密钥")}</button>}
              </div>
            </div>
          </div>}
          {active === "voice" && <div className="grid gap-4">
            {voiceStatus && <p className="whitespace-pre-wrap rounded-xl border border-slate-200 bg-slate-50 p-3 text-xs text-slate-600">{voiceStatus}</p>}
            <p className="text-xs text-slate-600">{label("语音指令模式会持续监听；可连续下达多条设置指令。再次按快捷键或说“关闭指令模式”即可停止。需要 DeepSeek API Key。")}</p>
            <label className="grid gap-2 text-sm">{label("识别引擎")}
              <select className="field" value={config.voice_backend} onChange={event => update("voice_backend", event.target.value)}>
                <option value="sensevoice">SenseVoice</option><option value="whisper">whisper.cpp</option>
              </select>
            </label>
            <label className="grid gap-2 text-sm">{label("语音识别语言")}
              <select className="field" value={config.voice_language} onChange={event => update("voice_language", event.target.value)}>
                <option value="auto">{label("自动识别")}</option>
                {languages.map(item => <option key={item.code} value={item.code}>{item.name}</option>)}
              </select>
            </label>
            <Switch label={label("是否保持语音输入")} checked={config.voice_keep_input} onChange={value => update("voice_keep_input", value)} />
            <Switch label={label("按住语音快捷键说话，松开后停止")} checked={config.voice_hold_to_talk} onChange={value => update("voice_hold_to_talk", value)} />
            <Switch label={label("多条词库匹配时自动输入第一条")} checked={config.voice_auto_copy_first} onChange={value => update("voice_auto_copy_first", value)} />
            <Switch label={label("Aion2 兼容模式：复制语音结果，手动粘贴")} checked={config.voice_aion2_manual_paste} onChange={value => update("voice_aion2_manual_paste", value)} />
            <Switch label={label("仅识别已注册的本人声音")} checked={config.voice_owner_filter} onChange={value => update("voice_owner_filter", value)} />
            <details className="group rounded-xl border border-slate-200 bg-slate-50 text-sm">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-xl px-4 py-3 font-medium text-slate-800 hover:bg-slate-100 [&::-webkit-details-marker]:hidden">
                {label("本人声纹")}
                <svg aria-hidden="true" viewBox="0 0 20 20" fill="none" className="h-4 w-4 shrink-0 text-slate-500 transition-transform group-open:rotate-180">
                  <path d="m5 7.5 5 5 5-5" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              </summary>
              <div className="border-t border-slate-200 px-4 pb-4 pt-3">
              <p className="whitespace-pre-line text-xs leading-5 text-slate-600">{label("1. 今天我想试试语音输入，看看识别是否准确。\n2. 请把这句话记下来，等一下我还会继续说话。\n3. 现在我会用平时的声音，完成最后一段录音。")}</p>
              <p className="mt-2 text-xs leading-5 text-slate-600">{label("点击“注册本人声纹”后先等约 1 秒，再逐句朗读。每句说约 3–5 秒，每句后（包括最后一句）停顿约 1 秒，看界面是否依次显示 1/3、2/3、3/3。")}</p>
              <div className="mt-4 flex items-center gap-2">
                <button type="button" className="button" disabled={voiceEnrolling} onClick={async () => {
                  setVoiceEnrolling(true); setVoiceEnrollmentStatus(label("请开始朗读第一段"));
                  try { await api.registerVoiceProfile(); setVoiceRegistered(true); }
                  catch (error) { setVoiceEnrollmentStatus(String(error)); }
                  finally { setVoiceEnrolling(false); }
                }}>{label("注册本人声纹")}</button>
                {voiceRegistered && <button type="button" className="button button-danger" disabled={voiceEnrolling} onClick={async () => {
                  try { await api.deleteVoiceProfile(); setVoiceRegistered(false); update("voice_owner_filter", false); setVoiceEnrollmentStatus(label("声纹已删除")); }
                  catch (error) { setVoiceEnrollmentStatus(String(error)); }
                }}>{label("删除声纹")}</button>}
              </div>
              <p role="status" className="mt-3 text-xs text-slate-600">{label("本人声纹")} · {voiceRegistered ? label("已注册本人声纹") : label("尚未注册本人声纹")}{voiceEnrollmentStatus && ` · ${voiceEnrollmentStatus}`}</p>
              </div>
            </details>
            <Switch label={label("过滤电脑播放声")} checked={config.voice_playback_filter} onChange={value => update("voice_playback_filter", value)} />
            <div className="grid gap-2 border-t border-slate-200 pt-4">
              <Switch label={label("监听系统播放声音并显示识别字幕")} checked={config.system_audio_monitor} onChange={value => update("system_audio_monitor", value)} />
              <Switch label={label("将系统声音字幕翻译成原始语言")} checked={config.system_audio_translate} onChange={value => update("system_audio_translate", value)} />
              <label className="grid gap-2 text-sm text-slate-700">{label("系统声音识别语言")}
                <select className="field" value={config.system_audio_language} onChange={event => update("system_audio_language", event.target.value)}>
                  <option value="target">{label("跟随目标语言")}</option>
                  <option value="auto">{label("自动识别")}</option>
                  <option value="cn">{label("简体中文")}</option>
                  <option value="en">{label("英语")}</option>
                  <option value="ja">{label("日语")}</option>
                  <option value="ko">{label("韩语")}</option>
                </select>
              </label>
              <label className="grid gap-2 text-sm text-slate-700">{label("系统声音播放设备")}
                <select className="field" value={config.system_audio_device} onChange={event => update("system_audio_device", event.target.value)}>
                  <option value="">{label("Windows 默认播放设备")}</option>
                  {config.system_audio_device && !audioDevices.some(device => device.id === config.system_audio_device)
                    && <option value={config.system_audio_device} disabled>{label("已保存的设备不可用")}</option>}
                  {audioDevices.map(device => <option key={device.id} value={device.id}>{device.name}</option>)}
                </select>
              </label>
              <p className="text-xs text-slate-600">{label("选择网页视频实际使用的播放设备；原始语言为自动识别时，字幕译为简体中文。翻译需要 DeepSeek API Key。")}</p>
            </div>
            <Slider label={label("语音停顿时间（秒）")} value={config.voice_silence_seconds} min={0.2} max={3} step={0.1} unit="s" onChange={value => update("voice_silence_seconds", value)} />
            <Slider label={label("指令模式停顿时间（秒）")} value={config.command_silence_seconds} min={0.3} max={3} step={0.1} unit="s" onChange={value => update("command_silence_seconds", value)} />
            <div className="grid gap-4 border-t border-slate-200 pt-5 text-sm text-slate-700">
              <h2 className="font-semibold text-slate-900">{label("查询结果发音")}</h2>
              <p>{label("选择查询结果与自动麦克风发音使用的本地声音。")}</p>
              {!playbackLanguage ? <p className="rounded-xl bg-slate-100 p-3">{label("请先在词库页面选择目标语言。")}</p> : <>
                <label className="grid gap-2">{label("发音语音")} · {playbackLanguage}
                  <select className="field" value={selectedPlaybackUri} onChange={event => choosePlaybackVoice(event.target.value)}>
                    <option value="">{label("自动选择")}</option>
                    {selectedPlaybackUri && !voicesForLanguage.some(voice => voice.voiceURI === selectedPlaybackUri)
                      && !localVoicesForLanguage.some(voice => voice.id === selectedPlaybackUri) &&
                      <option value={selectedPlaybackUri} disabled>{label("已保存的声音不可用")}</option>}
                    {voicesForLanguage.map(voice => <option key={voice.voiceURI} value={voice.voiceURI}>{voice.name} · {voice.lang} · {label("仅本机播放")}</option>)}
                    {localVoicesForLanguage.map(voice => <option key={voice.id} value={voice.id}>{voice.name} · {voice.lang}</option>)}
                  </select>
                </label>
                <button type="button" className="button w-fit" disabled={!voicesForLanguage.length && !localVoicesForLanguage.length} onClick={() => void previewPlaybackVoice()}>{label("试听")}</button>
                {playbackVoices === null && <p className="text-xs text-slate-500">{label("正在读取本地语音…")}</p>}
                {playbackVoices !== null && !voicesForLanguage.length && !localVoicesForLanguage.length && <p className="text-xs text-amber-800">{label("未找到这个语言可用的本地声音。请在 Windows 语言设置中安装文字转语音组件后重启 to_words。")}</p>}
              </>}
              {playbackError && <p role="status" className="text-xs text-rose-700">{playbackError}</p>}
              <div className="grid gap-3 border-t border-slate-200 pt-5">
                <Switch label={label("自动向虚拟麦克风播放查询和语音结果")} checked={config.mic_tts_enabled} onChange={value => void changeMicTts(value)} />
                <label className="grid gap-2">{label("虚拟麦克风的播放端")}
                  <select className="field" value={config.mic_output_device} onChange={event => update("mic_output_device", event.target.value)}>
                    <option value="">{label("未选择")}</option>
                    {config.mic_output_device && !audioDevices.some(device => device.id === config.mic_output_device)
                      && <option value={config.mic_output_device} disabled>{label("已保存的设备不可用")}</option>}
                    {audioDevices.map(device => <option key={device.id} value={device.id}>{device.name}</option>)}
                  </select>
                </label>
                <p className="text-xs text-slate-600">{label("请选虚拟音频设备的播放端（例如 CABLE Input），并在聊天软件中选择对应的录音端（例如 CABLE Output）。")}</p>
                <p className="text-xs text-slate-600">{label("使用 WebView 声音时，自动麦克风发音会选择同名 Windows 声音或同语言本地模型。")}</p>
              </div>
            </div>
          </div>}
          {active === "shortcuts" && <div className="grid gap-1">
            {(["popup", "settings", "clear_composed", "ocr", "voice", "command", "mic_tts", "system_audio"] as const).map((key, index) =>
              <ShortcutField key={key} label={label(["打开查询", "设置", "清空所有组合内容", "OCR 识别", "语音识别", "语音指令模式", "自动麦克风发音开关", "系统声音字幕与翻译开关"][index])}
                value={config.hotkeys[key]} onChange={value => updatePart("hotkeys", key, value)}
                recording={recordingShortcut === key} onRecordingChange={recording => setRecordingShortcut(recording ? key : null)} />)}
          </div>}
          {active === "appearance" && <div className="grid gap-7">
            <div className="grid gap-5">
              <h2 className="font-semibold">{label("输入框")}</h2>
              <ColorField label={label("背景颜色")} value={config.input.background_color} onChange={value => updatePart("input", "background_color", value)} />
              <Slider label={label("透明度")} value={config.input.opacity} min={0} max={1} step={0.05} onChange={value => updatePart("input", "opacity", value)} />
              <Slider label={label("宽度")} value={config.input.width} min={80} max={1200} step={10} unit="px" onChange={value => updatePart("input", "width", value)} />
              <Slider label={label("高度")} value={config.input.height} min={18} max={200} step={2} unit="px" onChange={value => updatePart("input", "height", value)} />
              <Slider label={label("字号")} value={config.input.font_size} min={8} max={72} step={1} unit="px" onChange={value => updatePart("input", "font_size", value)} />
              <Slider label={label("圆角")} value={config.input.corner_radius} min={0} max={100} step={1} unit="px" onChange={value => updatePart("input", "corner_radius", value)} />
            </div>
            <div className="grid gap-5 border-t border-slate-200 pt-5">
              <h2 className="font-semibold">{label("查询结果")}</h2>
              <ColorField label={label("背景颜色")} value={config.results.background_color} onChange={value => updatePart("results", "background_color", value)} />
              <Slider label={label("最多展示匹配数")} value={config.results.max_visible_results} min={1} max={100} step={1} onChange={value => updatePart("results", "max_visible_results", value)} />
              <Slider label={label("透明度")} value={config.results.opacity} min={0} max={1} step={0.05} onChange={value => updatePart("results", "opacity", value)} />
              <Slider label={label("最小宽度")} value={config.results.min_width} min={80} max={1200} step={10} unit="px" onChange={value => updatePart("results", "min_width", value)} />
              <Slider label={label("最大宽度")} value={config.results.max_width} min={200} max={1600} step={10} unit="px" onChange={value => updatePart("results", "max_width", value)} />
              <Slider label={label("最小行高")} value={config.results.min_row_height} min={18} max={200} step={2} unit="px" onChange={value => updatePart("results", "min_row_height", value)} />
              <Slider label={label("最大高度")} value={config.results.max_height} min={40} max={1200} step={10} unit="px" onChange={value => updatePart("results", "max_height", value)} />
              <Slider label={label("字号")} value={config.results.font_size} min={8} max={72} step={1} unit="px" onChange={value => updatePart("results", "font_size", value)} />
              <Slider label={label("圆角")} value={config.results.corner_radius} min={0} max={100} step={1} unit="px" onChange={value => updatePart("results", "corner_radius", value)} />
            </div>
            <div className="grid gap-5 border-t border-slate-200 pt-5">
              <h2 className="font-semibold">{label("通用")}</h2>
              <Slider label={label("字幕字号")} value={config.subtitle_font_size} min={12} max={48} step={1} unit="px" onChange={value => update("subtitle_font_size", value)} />
              <ColorField label={label("文字颜色")} value={config.text_color} onChange={value => update("text_color", value)} />
              <ColorField label={label("选中背景颜色")} value={config.selected_color} onChange={value => update("selected_color", value)} />
              <ColorField label={label("选中文字颜色")} value={config.selected_text_color} onChange={value => update("selected_text_color", value)} />
              <Slider label={label("选中背景透明度")} value={config.selected_opacity} min={0} max={1} step={0.05} onChange={value => update("selected_opacity", value)} />
              <Slider label={label("间距")} value={config.gap} min={0} max={40} step={1} unit="px" onChange={value => update("gap", value)} />
            </div>
          </div>}
        </section>
      </div>
    </div>
    <footer className="flex shrink-0 items-center justify-end gap-3 border-t border-white/80 bg-white/75 px-8 py-4 backdrop-blur-xl">
      {notice && <span role="status" className="mr-auto max-w-lg truncate text-sm text-slate-600">{notice}</span>}
      <button type="button" className="button" onClick={() => api.getDefaultConfig().then(setConfig).catch(error => setNotice(String(error)))}>{label("恢复默认值")}</button>
      <button type="button" className="button button-primary" disabled={busy} onClick={save}>{label("保存并应用")}</button>
      {showScrollTop && <button type="button" className="button w-9 px-0" aria-label={label("返回顶部")}
        onClick={() => scrollArea.current?.scrollTo({ top: 0, behavior: "smooth" })}>
        <svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m5.5 11.5 4.5-4 4.5 4" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </button>}
    </footer>
  </main>;
}
