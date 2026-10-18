import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { defineComponent } from 'vue'

import { activeCatalog, DEFAULT_CATALOG, typeOf } from '@/decode/catalog'
import { i18n } from '@/i18n'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'

import { useDecoderCopy } from './useDecoderCopy'

/** The composable as a component sees it. */
function words(): ReturnType<typeof useDecoderCopy> {
  let result: ReturnType<typeof useDecoderCopy> | undefined
  const Probe = defineComponent({
    setup() {
      result = useDecoderCopy()
      return () => null
    },
  })
  mount(Probe, { global: { plugins: [i18n] } })
  if (!result) throw new Error('the composable ran')
  return result
}

const pwm = typeOf('pwm') as CatalogEntry
const block = (name: string, fields: Record<string, string> = {}) => ({
  name,
  fields,
  values: {},
  events: {},
  eventFields: {},
})

describe('useDecoderCopy', () => {
  afterEach(() => {
    activeCatalog.value = DEFAULT_CATALOG
    i18n.global.locale.value = 'en-US'
  })

  it("uses the interface's copy for the decoders built in, and the identifiers otherwise", () => {
    i18n.global.locale.value = 'en-US'
    const copy = words()
    expect(copy.typeName('pwm')).toBe('PWM')
    expect(copy.typeSummary('pwm')).toBe('Each cycle of a PWM line: its period and its active time')
    expect(copy.fieldLabel('pwm', 'channel')).toBe('Signal line')
    expect(copy.valueLabel('pwm', 'polarity', 'active-low')).toBe('Active low')
    expect(copy.eventLabel('pwm', 'cycle')).toBe('Cycle')
    expect(copy.eventFieldLabel('pwm', 'periodTicks')).toBe('Period (ticks)')
    expect(copy.categoryName('line')).toBe('Signal line')
    expect(copy.typeName('my-sensor')).toBe('my-sensor')
    expect(copy.typeSummary('my-sensor')).toBe('')
    expect(copy.eventLabel('my-sensor', 'reading')).toBe('reading')
  })

  it("prefers the copy a loaded module carries, in the interface's language", () => {
    activeCatalog.value = [
      ...DEFAULT_CATALOG.filter((type) => type.name !== 'pwm'),
      {
        ...pwm,
        module: '/decoders/pwm.json',
        locales: { 'en-US': block('Pulse width', { channel: 'Line' }) },
      },
      {
        ...pwm,
        name: 'my-sensor',
        module: '/decoders/my-sensor.json',
        locales: { 'en-US': block('My sensor'), 'zh-CN': block('我的传感器') },
      },
    ]
    i18n.global.locale.value = 'zh-CN'
    const copy = words()
    expect(copy.typeName('my-sensor')).toBe('我的传感器')
    // A module with copy in another language alone shows that rather than nothing.
    expect(copy.typeName('pwm')).toBe('Pulse width')
    expect(copy.fieldLabel('pwm', 'channel')).toBe('Line')
    // What the module's copy leaves out comes from the interface's copy for the name.
    expect(copy.fieldLabel('pwm', 'polarity')).toBe('极性')
    expect(copy.typeSummary('pwm')).toBe('测量 PWM 信号线的每个周期：周期与有效时长')
  })
})
