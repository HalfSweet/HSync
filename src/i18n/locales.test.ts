import { describe, expect, it } from 'vitest'

import { detectLocale } from '.'
import enUS from './locales/en-US.json'
import zhCN from './locales/zh-CN.json'

type Messages = { [key: string]: string | Messages }

function flatten(messages: Messages, prefix = ''): Map<string, string> {
  const out = new Map<string, string>()
  for (const [key, value] of Object.entries(messages)) {
    const path = prefix ? `${prefix}.${key}` : key
    if (typeof value === 'string') out.set(path, value)
    else for (const [k, v] of flatten(value, path)) out.set(k, v)
  }
  return out
}

const placeholders = (message: string) =>
  [...message.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort()

const zh = flatten(zhCN)
const en = flatten(enUS)

describe('locale files', () => {
  it('define the same keys', () => {
    expect([...en.keys()].sort()).toEqual([...zh.keys()].sort())
  })

  it('use the same placeholders for every key', () => {
    for (const [key, message] of zh) {
      expect(placeholders(en.get(key) ?? ''), key).toEqual(placeholders(message))
    }
  })

  it('have no empty messages', () => {
    for (const [key, message] of [...zh, ...en]) {
      expect(message.trim(), key).not.toBe('')
    }
  })

  it('avoid characters reserved by the message syntax', () => {
    for (const [key, message] of [...zh, ...en]) {
      expect(message, key).not.toMatch(/[@|$]/)
    }
  })
})

describe('detectLocale', () => {
  it('picks Chinese for any zh tag', () => {
    expect(detectLocale(['zh-TW'])).toBe('zh-CN')
    expect(detectLocale(['zh-Hans-CN', 'en-US'])).toBe('zh-CN')
  })

  it('picks English for English and other languages', () => {
    expect(detectLocale(['en-GB'])).toBe('en-US')
    expect(detectLocale(['de-DE'])).toBe('en-US')
    expect(detectLocale([])).toBe('en-US')
  })

  it('honours the order of preference', () => {
    expect(detectLocale(['en-US', 'zh-CN'])).toBe('en-US')
  })
})
