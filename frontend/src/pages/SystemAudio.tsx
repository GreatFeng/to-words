/** Movable, resizable subtitles for system playback and its translation. */
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "../api";
import { t } from "../i18n";

type Subtitle = { original: string; result: string; status: string };
const overlay = getCurrentWindow();

export function SystemAudioPage() {
  const [subtitle, setSubtitle] = useState<Subtitle>({ original: "", result: "", status: "" });
  const [fontSize, setFontSize] = useState(20);
  const [language, setLanguage] = useState("zh-CN");
  const interacted = useRef(false);
  const saveTimer = useRef<number | undefined>(undefined);
  useEffect(() => {
    let mounted = true;
    const refresh = () => void api.systemAudioDisplay()
      .then(display => { if (mounted) setSubtitle(display); })
      .catch(() => {});
    const refreshConfig = () => void api.getConfig()
      .then(config => { if (mounted) { setFontSize(config.subtitle_font_size); setLanguage(config.ui_language); } })
      .catch(() => {});
    const saveGeometry = () => {
      if (!interacted.current) return;
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(async () => {
        try {
          const [position, size] = await Promise.all([overlay.outerPosition(), overlay.innerSize()]);
          await api.saveSubtitleBounds({ x: position.x, y: position.y, width: size.width, height: size.height });
        } catch (error) { console.error("Cannot save subtitle position:", error); }
      }, 350);
    };
    refresh();
    refreshConfig();
    // Hidden WebView2 windows can miss events while suspended. Polling after
    // the overlay becomes visible restores the latest Rust-side subtitle.
    const timer = window.setInterval(refresh, 250);
    const pending = listen<Subtitle>("system-audio-display", event => setSubtitle(event.payload));
    const configListener = listen("config-updated", refreshConfig);
    const moveListener = overlay.onMoved(saveGeometry);
    const resizeListener = overlay.onResized(saveGeometry);
    return () => {
      mounted = false;
      window.clearInterval(timer);
      window.clearTimeout(saveTimer.current);
      void pending.then(stop => stop());
      void configListener.then(stop => stop());
      void moveListener.then(stop => stop());
      void resizeListener.then(stop => stop());
    };
  }, []);
  const drag = (event: React.MouseEvent) => {
    if (event.button !== 0) return;
    interacted.current = true;
    void overlay.startDragging().catch(error => console.error("Cannot drag subtitle:", error));
  };
  const resize = (event: React.MouseEvent) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    interacted.current = true;
    void overlay.startResizeDragging("SouthEast").catch(error => console.error("Cannot resize subtitle:", error));
  };
  return <main onMouseDown={drag}
    className="group relative flex h-full w-full cursor-move select-none flex-col items-center justify-end overflow-hidden bg-transparent px-6 pb-3 text-center text-white [text-shadow:0_2px_7px_rgba(0,0,0,.95),0_0_2px_rgba(0,0,0,1)]">
    <div className="absolute right-2 top-1 text-xs text-white/70 opacity-0 transition-opacity group-hover:opacity-100" aria-hidden="true">⋮⋮</div>
    {subtitle.original && <p className="max-h-[45%] max-w-full overflow-hidden break-words font-semibold" style={{ fontSize, lineHeight: 1.3 }}>{subtitle.original}</p>}
    {subtitle.result && <p className="mt-1 max-h-[45%] max-w-full overflow-hidden break-words font-bold" style={{ fontSize: fontSize + 3, lineHeight: 1.3 }}>{subtitle.result}</p>}
    {subtitle.status && <p className="mt-1 max-w-full truncate px-3 text-xs">{subtitle.status}</p>}
    <button type="button" onMouseDown={resize} title={t("拖动调整字幕区域大小", language)} aria-label={t("调整字幕区域大小", language)}
      className="absolute bottom-0 right-0 flex h-6 w-6 cursor-nwse-resize items-end justify-end p-1 text-white/70 opacity-0 transition-opacity group-hover:opacity-100">
      <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true"><path d="M5 12h7V5M9 12l3-3" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" /></svg>
    </button>
  </main>;
}
