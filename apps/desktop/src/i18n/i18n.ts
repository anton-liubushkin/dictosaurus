import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "../locales/en/common.json";
import ru from "../locales/ru/common.json";

export type UiLanguage = "auto" | "en" | "ru";

/** Cache of the last resolved language to avoid an English flash before Rust settings load. */
const STORAGE_KEY = "dictosaurus.uiLanguageResolved";

/** Emitted (via Tauri) when the user changes the UI language, so all windows stay in sync. */
export const UI_LANGUAGE_EVENT = "ui-language-changed";

function systemLanguage(): "en" | "ru" {
  const primary = navigator.language?.split("-")[0]?.toLowerCase();
  return primary === "ru" ? "ru" : "en";
}

function readInitialLanguage(): "en" | "ru" {
  try {
    const cached = localStorage.getItem(STORAGE_KEY);
    if (cached === "en" || cached === "ru") return cached;
  } catch {
    /* no storage */
  }
  return systemLanguage();
}

/** Applies a language preference from settings ("auto" resolves to the system language). */
export function applyUiLanguage(pref: string): void {
  const resolved = pref === "en" || pref === "ru" ? pref : systemLanguage();
  if (i18n.language !== resolved) void i18n.changeLanguage(resolved);
  try {
    localStorage.setItem(STORAGE_KEY, resolved);
  } catch {
    /* no storage */
  }
}

void i18n.use(initReactI18next).init({
  lng: readInitialLanguage(),
  fallbackLng: "en",
  supportedLngs: ["en", "ru"],
  resources: {
    en: { common: en },
    ru: { common: ru },
  },
  defaultNS: "common",
  ns: ["common"],
  interpolation: { escapeValue: false },
});

export default i18n;
