import { invoke } from "@tauri-apps/api/core";

export type WordRow = {
  key: string;
  value: string;
  pronunciation: string;
  created_at: number;
  created_at_label: string;
};

export type SearchHit = {
  key: string;
  value: string;
  pronunciation?: string;
  library?: string;
  rank: "exact" | "prefix" | "contains";
};
export type SearchPage = { hits: SearchHit[]; total: number; language_mismatch: boolean };

export type UiConfig = {
  input: { background_color: string; opacity: number; width: number; height: number; font_size: number; corner_radius: number };
  results: { background_color: string; opacity: number; min_width: number; max_width: number; min_row_height: number; max_height: number; font_size: number; corner_radius: number; max_visible_results: number };
  hotkeys: { popup: string; settings: string; clear_composed: string; ocr: string; voice: string; command: string; mic_tts: string; system_audio: string; selected_translate: string };
  gap: number;
  continuous_input: boolean;
  ai_translation: boolean;
  ai_auto_save: boolean;
  selected_text_context_menu: boolean;
  ai_politeness: "informal" | "conversational" | "formal";
  ocr_auto_translate: boolean;
  ocr_show_original: boolean;
  voice_auto_copy_first: boolean;
  voice_aion2_manual_paste: boolean;
  voice_keep_input: boolean;
  voice_hold_to_talk: boolean;
  voice_owner_filter: boolean;
  voice_playback_filter: boolean;
  system_audio_monitor: boolean;
  system_audio_translate: boolean;
  system_audio_device: string;
  system_audio_language: string;
  subtitle_font_size: number;
  subtitle_bounds: SubtitleBounds | null;
  voice_silence_seconds: number;
  command_silence_seconds: number;
  voice_backend: string;
  voice_language: string;
  speech_voices: Record<string, string>;
  mic_tts_enabled: boolean;
  mic_output_device: string;
  translation_language: string;
  source_language: string;
  ui_language: string;
  text_color: string;
  selected_color: string;
  selected_text_color: string;
  selected_opacity: number;
};

export type SubtitleBounds = { x: number; y: number; width: number; height: number };

export type LanguageOption = { code: string; name: string };
export type LocalTtsVoice = { id: string; name: string; lang: string };
export type AudioOutputDevice = { id: string; name: string };
export type OcrResult = { original: string; translation: string; status: string; show_original: boolean };
export type OcrSnapshot = { image: string; generation: number };
export type OcrRectangle = { x: number; y: number; width: number; height: number };

export const api = {
  getConfig: () => invoke<UiConfig>("get_config"),
  narratorVoice: () => invoke<string | null>("get_narrator_voice"),
  localTtsVoices: () => invoke<LocalTtsVoice[]>("list_local_tts_voices"),
  windowsTtsVoices: () => invoke<LocalTtsVoice[]>("list_windows_tts_voices"),
  audioOutputDevices: () => invoke<AudioOutputDevice[]>("list_audio_output_devices"),
  synthesizePiper: (voiceId: string, text: string) => invoke<string>("synthesize_piper", { voiceId, text }),
  synthesizeWindows: (voiceId: string, text: string) => invoke<string>("synthesize_windows", { voiceId, text }),
  saveConfig: (config: UiConfig) => invoke<void>("save_config", { config }),
  setMicTtsEnabled: (enabled: boolean, outputDevice: string) => invoke<boolean>("set_mic_tts_enabled", { enabled, outputDevice }),
  setShortcutRecording: (active: boolean) => invoke<void>("set_shortcut_recording", { active }),
  voiceEngineStatus: (backend: string, source: string, target: string) => invoke<string>("voice_engine_status", { backend, source, target }),
  hasApiKey: () => invoke<boolean>("has_api_key"),
  setApiKey: (key: string) => invoke<void>("set_api_key", { key }),
  deleteApiKey: () => invoke<void>("delete_api_key"),
  getDefaultConfig: () => invoke<UiConfig>("get_default_config"),
  languages: () => invoke<LanguageOption[]>("list_languages"),
  search: (query: string, page: number, pageSize: number) => invoke<SearchPage>("search_words", { query, page, pageSize }),
  copy: (text: string, usedEntries: { library: string; key: string }[]) => invoke<void>("copy_text", { text, usedEntries }),
  listWords: (source: string, target: string) => invoke<WordRow[]>("list_words", { source, target }),
  reviewVoiceCache: () => invoke<void>("review_voice_cache_now"),
  saveWords: (source: string, target: string, rows: WordRow[]) => invoke<void>("save_words", { source, target, rows }),
  exportWords: (name: string, rows: WordRow[]) => invoke<string>("export_words", { name, rows }),
  translate: (text: string) => invoke<{ translated: string; pronunciation: string; save_error: string | null }>("translate_text", { text }),
  translateSelection: (text: string) => invoke<{ translated: string; pronunciation: string; save_error: string | null }>("translate_selected_text", { text }),
  hideQuery: () => invoke<void>("hide_query"),
  resizeQuery: (width: number, height: number) => invoke<void>("resize_query", { width, height }),
  dragQuery: () => invoke<void>("drag_query"),
  ocrSnapshot: () => invoke<OcrSnapshot>("get_ocr_snapshot"),
  showOcr: (generation: number) => invoke<boolean>("show_ocr", { generation }),
  recognizeOcr: (rect: OcrRectangle, viewportWidth: number, viewportHeight: number) => invoke<OcrResult>("recognize_ocr", { rect, viewportWidth, viewportHeight }),
  confirmOcr: () => invoke<void>("confirm_ocr"),
  cancelOcr: () => invoke<void>("cancel_ocr"),
  voiceDisplay: () => invoke<{ original: string; result: string; status: string }>("get_voice_display"),
  systemAudioDisplay: () => invoke<{ original: string; result: string; status: string }>("get_system_audio_display"),
  saveSubtitleBounds: (bounds: SubtitleBounds) => invoke<void>("save_subtitle_bounds", { bounds }),
  toggleVoice: () => invoke<void>("toggle_voice"),
  takePendingQuery: () => invoke<string | null>("take_pending_query"),
  takePendingSelectedTranslation: () => invoke<string | null>("take_pending_selected_translation"),
  translateMenuSelection: () => invoke<void>("translate_menu_selection"),
  hideSelectionMenu: () => invoke<void>("hide_selection_menu"),
  voiceRegistered: () => invoke<boolean>("voice_registered"),
  registerVoiceProfile: () => invoke<void>("register_voice_profile"),
  deleteVoiceProfile: () => invoke<void>("delete_voice_profile"),
  exit: () => invoke<void>("exit_app"),
};
