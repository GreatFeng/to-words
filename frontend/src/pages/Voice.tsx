import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";

type VoiceDisplay = { original: string; result: string; status: string };

export function VoicePage() {
  const [display, setDisplay] = useState<VoiceDisplay>({ original: "", result: "", status: "" });
  useEffect(() => {
    void api.voiceDisplay().then(setDisplay);
    const unlisten = listen<VoiceDisplay>("voice-display", event => setDisplay(event.payload));
    return () => { void unlisten.then(stop => stop()); };
  }, []);
  return <main className="flex h-full w-full flex-col items-center justify-end bg-transparent pb-4 text-center text-white [text-shadow:0_2px_8px_rgba(0,0,0,.8)]">
    {display.original && <p className="text-lg font-semibold">{display.original}</p>}
    {display.result && <p className="mt-2 text-xl font-bold">{display.result}</p>}
    {!display.result && display.status && <p className="mt-1 text-xs opacity-80">{display.status}</p>}
  </main>;
}
