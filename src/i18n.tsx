import { createContext, useContext, type ReactNode } from "react";
import type { Language } from "./types";

interface I18nValue {
  lang: Language;
  setLang: (language: Language) => void;
  t: (en: string, zh: string) => string;
  bin: string;
  binLower: string;
  currency: string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({
  lang,
  setLang,
  children,
}: {
  lang: Language;
  setLang: (language: Language) => void;
  children: ReactNode;
}) {
  const value: I18nValue = {
    lang,
    setLang,
    t: (en, zh) => (lang === "zh" ? zh : en),
    bin: lang === "zh" ? "废纸篓" : "Trash",
    binLower: lang === "zh" ? "废纸篓" : "trash",
    currency: lang === "zh" ? "zh-CN" : "en-US",
  };
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n must be used inside I18nProvider");
  return value;
}

export function detectLanguage(): Language {
  const candidate = navigator.language || "en";
  return candidate.toLowerCase().startsWith("zh") ? "zh" : "en";
}
