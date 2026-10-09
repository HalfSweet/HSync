import { createI18n } from 'vue-i18n'

import enUS from './locales/en-US.json'
import zhCN from './locales/zh-CN.json'

/** zh-CN is the source language: its copy comes straight from the design. */
export type MessageSchema = typeof zhCN

export const SUPPORTED_LOCALES = ['zh-CN', 'en-US'] as const
export type AppLocale = (typeof SUPPORTED_LOCALES)[number]

declare module 'vue-i18n' {
  // eslint-disable-next-line @typescript-eslint/no-empty-object-type
  export interface DefineLocaleMessage extends MessageSchema {}
}

/** Chinese for any `zh*` system language, English otherwise (DESIGN.md §10.1). */
export function detectLocale(languages: readonly string[]): AppLocale {
  for (const language of languages) {
    const tag = language.toLowerCase()
    if (tag.startsWith('zh')) return 'zh-CN'
    if (tag.startsWith('en')) return 'en-US'
  }
  return 'en-US'
}

export const i18n = createI18n<[MessageSchema], AppLocale, false>({
  legacy: false,
  locale: detectLocale(typeof navigator === 'undefined' ? [] : navigator.languages),
  fallbackLocale: 'zh-CN',
  messages: {
    'zh-CN': zhCN,
    'en-US': enUS,
  },
  missingWarn: import.meta.env.DEV,
  fallbackWarn: import.meta.env.DEV,
})

export function setLocale(locale: AppLocale): void {
  i18n.global.locale.value = locale
  document.documentElement.lang = locale
}
