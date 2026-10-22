import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

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

/** The copy a decoder module declares for one language (`uiharu/docs/dsl.md` §2.4). */
interface ModuleCopy {
  name?: string
  summary?: string
  fields?: Record<string, string>
  values?: Record<string, Record<string, string>>
  events?: Record<string, string>
  eventFields?: Record<string, string>
}
interface ModuleWithCopy {
  name: string
  decoder: { locales: Record<string, ModuleCopy> }
}

/** A configuration field as the catalog spells it: serde's snake case. */
const snake = (name: string) => name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`)

/** A module's copy for one language, under the keys the app's locale files use. */
function keysOf(module: ModuleWithCopy, tag: string): Map<string, string> {
  const copy = module.decoder.locales[tag] ?? {}
  const prefix = `decoder.${module.name}`
  const out = new Map<string, string>()
  if (copy.name !== undefined) out.set(`${prefix}.name`, copy.name)
  if (copy.summary !== undefined) out.set(`${prefix}.summary`, copy.summary)
  for (const [field, label] of Object.entries(copy.fields ?? {}))
    out.set(`${prefix}.field.${snake(field)}`, label)
  for (const [field, values] of Object.entries(copy.values ?? {}))
    for (const [value, label] of Object.entries(values))
      out.set(`${prefix}.value.${snake(field)}.${value}`, label)
  for (const [kind, label] of Object.entries(copy.events ?? {}))
    out.set(`${prefix}.event.${kind}`, label)
  for (const [field, label] of Object.entries(copy.eventFields ?? {}))
    out.set(`${prefix}.eventField.${field}`, label)
  return out
}

describe('the decoders written in the DSL', () => {
  // Tests prepare IR before running; type-checking and normal builds do not need it.
  const ir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../uiharu/dsl/ir')
  const modules = ['pwm', 'spi', 'spi-nor', 'uart'].map(
    (name) => JSON.parse(fs.readFileSync(path.join(ir, `${name}.json`), 'utf8')) as ModuleWithCopy,
  )
  it.each(modules.map((module) => [module.name, module] as const))(
    '%s carries the copy the app shows for it',
    (_name, module) => {
      for (const [tag, messages] of [
        ['en-US', en],
        ['zh-CN', zh],
      ] as const) {
        const own = new Map(
          [...messages].filter(([key]) => key.startsWith(`decoder.${module.name}.`)),
        )
        expect(keysOf(module, tag), tag).toEqual(own)
      }
    },
  )
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
