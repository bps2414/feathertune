import { getLocales } from 'expo-localization';

import { en } from './en';
import { ptBR } from './pt-BR';

export type Locale = 'en' | 'pt-BR';
export type TextKey = keyof typeof en;

const tables: Record<Locale, Record<TextKey, string>> = { en, 'pt-BR': ptBR };

/** Portuguese (any region) gets pt-BR, everything else English, like the desktop app. */
export function pickLocale(languageCode: string | null | undefined): Locale {
  return languageCode?.toLowerCase() === 'pt' ? 'pt-BR' : 'en';
}

/** The text for `key`; `{name}` in it is replaced by `vars.name`. */
export function t(key: TextKey, vars?: Record<string, string | number>): string {
  const text = tables[pickLocale(getLocales()[0]?.languageCode)][key];
  return vars ? text.replace(/\{(\w+)\}/g, (all, name: string) => (name in vars ? String(vars[name]) : all)) : text;
}
