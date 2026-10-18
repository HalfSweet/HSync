import { describe, expect, it } from 'vitest'

import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'

import { typeOf } from './catalog'
import { moduleLocale } from './copy'

const block = (name: string) => ({ name, fields: {}, values: {}, events: {}, eventFields: {} })
const sensor: CatalogEntry = {
  ...(typeOf('pwm') as CatalogEntry),
  name: 'my-sensor',
  module: '/decoders/my-sensor.json',
  locales: { 'en-US': block('My sensor'), 'zh-Hans': block('我的传感器') },
}

describe('moduleLocale', () => {
  it('picks the copy under the tag itself, else of the same language, else the first', () => {
    expect(moduleLocale(sensor, 'en-US')?.name).toBe('My sensor')
    expect(moduleLocale(sensor, 'zh-CN')?.name).toBe('我的传感器')
    expect(moduleLocale(sensor, 'de-DE')?.name).toBe('My sensor')
  })

  it('has nothing for a type without copy', () => {
    expect(moduleLocale(typeOf('pwm'), 'en-US')).toBeUndefined()
    expect(moduleLocale(undefined, 'en-US')).toBeUndefined()
  })
})
