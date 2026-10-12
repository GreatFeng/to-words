import en from "../../assets/i18n/en.json";
import ja from "../../assets/i18n/ja.json";
import ko from "../../assets/i18n/ko.json";

type Catalog = Record<string, string>;
const catalogs: Record<string, Catalog> = { en, ja, ko };

export function t(key: string, language = "zh-CN"): string {
  return catalogs[language]?.[key] ?? key;
}
