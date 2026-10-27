import { SettingsPage } from "./pages/Settings";
import { QueryPage } from "./pages/Query";
import { OcrPage } from "./pages/Ocr";
import { VoicePage } from "./pages/Voice";
import { SystemAudioPage } from "./pages/SystemAudio";
import { SelectionMenuPage } from "./pages/SelectionMenu";

export default function App() {
  const view = new URLSearchParams(window.location.search).get("view") ?? "settings";
  if (view === "query") return <QueryPage />;
  if (view === "ocr") return <OcrPage />;
  if (view === "voice") return <VoicePage />;
  if (view === "system_audio") return <SystemAudioPage />;
  if (view === "selection_menu") return <SelectionMenuPage />;
  return <SettingsPage />;
}
