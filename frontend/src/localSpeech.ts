/** 仅选择 WebView2/Windows 已安装的本地语音，避免意外调用在线 TTS。 */
const LANGUAGE_TAGS: Record<string, string> = {
  cn: "zh-CN", zh_tw: "zh-TW", ko: "ko-KR", ja: "ja-JP",
  en: "en-US", pt: "pt-PT", no: "nb-NO", fil: "fil-PH",
};

export function speechLanguage(code: string, text: string): string {
  if (code) return LANGUAGE_TAGS[code] ?? code;
  if (/[가-힣]/u.test(text)) return "ko-KR";
  if (/[ぁ-ゟ゠-ヿ]/u.test(text)) return "ja-JP";
  if (/[\p{Script=Han}]/u.test(text)) return "zh-CN";
  return "en-US";
}

export function findLocalVoice(voices: SpeechSynthesisVoice[], language: string): SpeechSynthesisVoice | undefined {
  const local = voices.filter(voice => voice.localService);
  const wanted = language.toLocaleLowerCase();
  const primary = wanted.split("-")[0];
  return local.find(voice => voice.lang.toLocaleLowerCase() === wanted)
    ?? local.find(voice => voice.lang.toLocaleLowerCase().split("-")[0] === primary);
}

export function findConfiguredVoice(voices: SpeechSynthesisVoice[], uri: string | undefined, language: string): SpeechSynthesisVoice | undefined {
  if (!uri) return undefined;
  const primary = language.toLocaleLowerCase().split("-")[0];
  return voices.find(voice => voice.localService
    && voice.voiceURI === uri
    && voice.lang.toLocaleLowerCase().split("-")[0] === primary);
}

function legacyVoiceName(name: string): string {
  return name.trim().toLocaleLowerCase()
    .replace(/\s+-\s+[^-]+$/, "")
    .replace(/\s+desktop$/, "")
    .trim();
}

/** 只接受同语种、同名称的本地语音；不会把自然语音匹配成同名普通语音。 */
export function findNarratorVoice(voices: SpeechSynthesisVoice[], selected: string | null, language: string): SpeechSynthesisVoice | undefined {
  if (!selected) return undefined;
  const wanted = language.toLocaleLowerCase();
  const primary = wanted.split("-")[0];
  const name = legacyVoiceName(selected);
  return voices.find(voice => voice.localService
    && voice.lang.toLocaleLowerCase().split("-")[0] === primary
    && legacyVoiceName(voice.name) === name);
}

export function availableVoices(synthesis: SpeechSynthesis): Promise<SpeechSynthesisVoice[]> {
  const voices = synthesis.getVoices();
  if (voices.length) return Promise.resolve(voices);
  return new Promise(resolve => {
    const done = () => {
      clearTimeout(timeout);
      synthesis.removeEventListener("voiceschanged", done);
      resolve(synthesis.getVoices());
    };
    const timeout = window.setTimeout(done, 1200);
    synthesis.addEventListener("voiceschanged", done, { once: true });
  });
}
