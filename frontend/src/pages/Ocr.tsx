import { useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { api, type OcrRectangle, type OcrResult } from "../api";

type Gesture = { mode: "draw" | "move" | "resize"; x: number; y: number; initial: OcrRectangle };
const empty: OcrResult = { original: "", translation: "", status: "", show_original: true };

function actionPosition(rect: OcrRectangle, viewportWidth: number, viewportHeight: number) {
  const width = 68, height = 28, margin = 10, edge = 6;
  const right = rect.x + rect.width + margin;
  const x = right + width <= viewportWidth - edge
    ? right
    : Math.min(Math.max(0, viewportWidth - width - edge), Math.max(edge, rect.x + rect.width - width));
  const y = Math.max(edge, Math.min(viewportHeight - height - edge, rect.y + rect.height - height));
  return { left: x - rect.x, top: y - rect.y };
}

export function OcrPage() {
  const [screenshot, setScreenshot] = useState("");
  const [selection, setSelection] = useState<OcrRectangle | null>(null);
  const selectionRef = useRef<OcrRectangle | null>(null);
  const [result, setResult] = useState<OcrResult>(empty);
  const gesture = useRef<Gesture | null>(null);
  const requestId = useRef(0);

  const prepare = async (requestedGeneration?: number) => {
    const id = ++requestId.current;
    try {
      const snapshot = await api.ocrSnapshot();
      if (id !== requestId.current) return;
      selectionRef.current = null;
      flushSync(() => {
        setSelection(null);
        setResult(empty);
        setScreenshot(snapshot.image);
      });
      // 页面仍隐藏；等真正用于绘制的图片解码完再显示窗口。
      const image = document.getElementById("ocr-screenshot") as HTMLImageElement | null;
      if (!image) throw new Error("OCR 截图图片未准备好");
      await image.decode();
      if (id !== requestId.current) return;
      await api.showOcr(snapshot.generation);
    } catch (error) {
      if (id !== requestId.current || requestedGeneration === undefined) return;
      setResult({ ...empty, status: String(error) });
      // 截图异常时仍显示可取消的窗口，避免快捷键看起来完全没有反应。
      await api.showOcr(requestedGeneration).catch(console.error);
    }
  };
  useEffect(() => {
    let active = true;
    const pending = listen<number>("ocr-reset", event => { void prepare(event.payload); });
    // 先订阅事件，再检查快照，避免首次窗口尚未加载时漏掉快捷键。
    void pending.then(() => { if (active) void prepare(); });
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") void api.cancelOcr(); };
    window.addEventListener("keydown", onKey);
    return () => { active = false; window.removeEventListener("keydown", onKey); void pending.then(stop => stop()); };
  }, []);

  const recognize = async (rect: OcrRectangle) => {
    if (rect.width < 8 || rect.height < 8) return;
    const id = ++requestId.current;
    setResult({ ...empty, status: "正在识别…" });
    try {
      const recognized = await api.recognizeOcr(rect, window.innerWidth, window.innerHeight);
      if (id === requestId.current) setResult(recognized);
    } catch (error) {
      if (id === requestId.current) setResult({ ...empty, status: String(error) });
    }
  };

  const begin = (event: React.PointerEvent, mode: Gesture["mode"], initial: OcrRectangle) => {
    event.stopPropagation();
    event.preventDefault();
    gesture.current = { mode, x: event.clientX, y: event.clientY, initial };
    event.currentTarget.setPointerCapture(event.pointerId);
    if (mode === "draw") { selectionRef.current = initial; setSelection(initial); setResult(empty); }
  };
  const move = (event: React.PointerEvent) => {
    const active = gesture.current;
    if (!active) return;
    const dx = event.clientX - active.x, dy = event.clientY - active.y;
    const r = active.initial;
    let next: OcrRectangle;
    if (active.mode === "draw") {
      next = { x: Math.min(r.x, event.clientX), y: Math.min(r.y, event.clientY), width: Math.abs(dx), height: Math.abs(dy) };
    } else if (active.mode === "move") {
      next = { ...r, x: Math.max(0, Math.min(window.innerWidth - r.width, r.x + dx)), y: Math.max(0, Math.min(window.innerHeight - r.height, r.y + dy)) };
    } else {
      next = { ...r, width: Math.max(8, Math.min(window.innerWidth - r.x, r.width + dx)), height: Math.max(8, Math.min(window.innerHeight - r.y, r.height + dy)) };
    }
    selectionRef.current = next;
    setSelection(next);
  };
  const end = () => {
    if (!gesture.current) return;
    gesture.current = null;
    if (selectionRef.current) void recognize(selectionRef.current);
  };

  return <main className="relative h-full w-full overflow-hidden"
    onPointerDown={event => begin(event, "draw", { x: event.clientX, y: event.clientY, width: 0, height: 0 })}
    onPointerMove={move} onPointerUp={end}>
    {screenshot && <img id="ocr-screenshot" src={screenshot} alt="屏幕截图" draggable={false} className="pointer-events-none absolute inset-0 h-full w-full object-fill" />}
    {selection && <div className="absolute cursor-move border-2 border-blue-400 bg-blue-200/10 shadow-[0_0_0_9999px_rgba(0,0,0,.14)]"
      style={{ left: selection.x, top: selection.y, width: selection.width, height: selection.height }}
      onPointerDown={event => begin(event, "move", selection)} onPointerMove={move} onPointerUp={end}>
      <span className="absolute -right-1 -bottom-1 size-3 cursor-nwse-resize rounded-sm bg-blue-500 ring-2 ring-white"
        onPointerDown={event => begin(event, "resize", selection)} onPointerMove={move} onPointerUp={end} />
      <div className="absolute left-0 top-full mt-2 max-w-[min(600px,90vw)] whitespace-pre-wrap text-sm font-semibold text-white [text-shadow:0_2px_7px_rgba(0,0,0,.9)]"
        onPointerDown={event => event.stopPropagation()}>
        {result.original && (result.show_original || !result.translation) && <p>{result.original}</p>}
        {result.translation && <p className="mt-1">{result.translation}</p>}
      </div>
      <div className="absolute flex h-7 w-[68px] items-center gap-1" style={actionPosition(selection, window.innerWidth, window.innerHeight)} onPointerDown={event => event.stopPropagation()}>
        <button type="button" disabled={!result.original} className="h-7 flex-1 rounded-md bg-slate-800/95 text-sm text-white disabled:opacity-50"
          onClick={event => { event.stopPropagation(); void api.confirmOcr().catch(error => setResult(current => ({ ...current, status: String(error) }))); }}>√</button>
        <button type="button" className="h-7 flex-1 rounded-md bg-slate-800/95 text-sm text-white"
          onClick={event => { event.stopPropagation(); void api.cancelOcr(); }}>×</button>
      </div>
    </div>}
    {result.status && <span className="absolute bottom-4 left-4 max-w-[70vw] rounded-lg bg-slate-900/90 px-3 py-1.5 text-xs text-white">{result.status}</span>}
  </main>;
}
