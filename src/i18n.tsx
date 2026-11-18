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
  binNoun = "trash",
  children,
}: {
  lang: Language;
  setLang: (language: Language) => void;
  binNoun?: "trash" | "recycle-bin";
  children: ReactNode;
}) {
  // Windows calls it 回收站; macOS and freedesktop keep 废纸篓.
  const zhBin = binNoun === "recycle-bin" ? "回收站" : "废纸篓";
  const enBin = binNoun === "recycle-bin" ? "Recycle Bin" : "Trash";
  const value: I18nValue = {
    lang,
    setLang,
    t: (en, zh) => (lang === "zh" ? zh : en),
    bin: lang === "zh" ? zhBin : enBin,
    binLower: lang === "zh" ? zhBin : enBin.toLowerCase(),
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
