import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { i18n } from '@/i18n'
import { decoderImportWarnings } from '@/session/decoderImports'
import DecoderImportWarnings from './DecoderImportWarnings.vue'

let state: {
  entries: unknown[]
  readonly importWarnings: ReturnType<typeof decoderImportWarnings>
  setEntries: ReturnType<typeof vi.fn>
}
vi.mock('@/stores/decode', () => ({ useDecodeStore: () => state }))

function retained(label: string) {
  return {
    label,
    nodes: {},
    import: {
      format: 'dsview',
      raw: { id: label, options: { opaque: [0, 255] } },
      runnable: false,
      diagnostics: [{ code: 'decode.import.unsupportedDecoder', params: { decoder: label } }],
    },
  }
}

function button(wrapper: VueWrapper, key: 'next' | 'remove') {
  const label = i18n.global.t(`decode.import.${key}`)
  return wrapper.findAll('button').find((button) => button.text() === label)!
}

beforeEach(() => {
  state = reactive({
    entries: [{ nodes: {} }, retained('first'), retained('second')],
    get importWarnings() {
      return decoderImportWarnings(this.entries)
    },
    setEntries: vi.fn().mockResolvedValue(undefined),
  })
})

describe('retained decoder import notes', () => {
  it('pages notes and removes the original entry index without changing other imports', async () => {
    const wrapper = mount(DecoderImportWarnings, { global: { plugins: [i18n] } })
    expect(wrapper.text()).toContain('first')
    expect(wrapper.text()).not.toContain('second')
    await button(wrapper, 'next').trigger('click')
    expect(wrapper.text()).toContain('second')
    expect(wrapper.text()).not.toContain('first')
    await button(wrapper, 'remove').trigger('click')
    await flushPromises()
    expect(state.setEntries).toHaveBeenCalledWith(state.entries.slice(0, 2))
    expect(state.entries).toHaveLength(3)
    wrapper.unmount()
  })

  it('keeps the raw import and displays a localized backend error when removal fails', async () => {
    state.setEntries.mockRejectedValue({
      code: 'error.app.unexpected',
      params: { detail: 'removal failed' },
    })
    const original = JSON.stringify(state.entries)
    const wrapper = mount(DecoderImportWarnings, { global: { plugins: [i18n] } })
    await button(wrapper, 'remove').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('removal failed')
    expect(JSON.stringify(state.entries)).toBe(original)
    expect(button(wrapper, 'remove').attributes('disabled')).toBeUndefined()
    wrapper.unmount()
  })
})
