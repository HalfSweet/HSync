import { mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { reactive } from 'vue'
import { i18n } from '@/i18n'
import { buildEntry, DEFAULT_CATALOG, typeOf, type DecoderEntry } from '@/decode/catalog'
import DecoderDialog from './DecoderDialog.vue'

let state: {
  catalog: typeof DEFAULT_CATALOG
  entries: DecoderEntry[]
  editor: { index: number | null }
  setEntries: ReturnType<typeof vi.fn>
  closeEditor: ReturnType<typeof vi.fn>
}
vi.mock('@/stores/decode', () => ({ useDecodeStore: () => state }))
vi.mock('@/stores/capture', () => ({
  useCaptureStore: () => ({
    summary: {
      channels: Array.from({ length: 8 }, (_, index) => ({ index, name: String(index) })),
    },
  }),
}))

function dialog(entry?: DecoderEntry) {
  if (entry) {
    state.entries = [entry]
    state.editor = { index: 0 }
  }
  return mount(DecoderDialog, { global: { plugins: [i18n] }, attachTo: document.body })
}
function save(wrapper: VueWrapper) {
  return wrapper.findAll('footer button').at(-1)!.trigger('click')
}
async function append(wrapper: VueWrapper, type: string) {
  const select = wrapper
    .findAll('select')
    .find((select) =>
      select
        .findAll('option')
        .some((option) => option.attributes('value')?.startsWith(`["${type}",`)),
    )!
  const option = select
    .findAll('option')
    .find((option) => option.attributes('value')?.startsWith(`["${type}",`))!
  await select.setValue(option.attributes('value'))
}

beforeEach(() => {
  state = reactive({
    catalog: DEFAULT_CATALOG,
    entries: [],
    editor: { index: null },
    setEntries: vi.fn().mockResolvedValue(undefined),
    closeEditor: vi.fn(),
  })
})
describe('decoder graph editor', () => {
  it('adds and saves a three-layer 1-Wire temperature pipeline', async () => {
    const wrapper = dialog()
    const name = i18n.global.t('decoder.onewire-link.name')
    await wrapper
      .findAll('[role="option"]')
      .find((button) => button.text().includes(name))!
      .trigger('click')
    await append(wrapper, 'onewire')
    await append(wrapper, 'ds18b20')
    await save(wrapper)
    const entry = state.setEntries.mock.calls[0]![0][0] as DecoderEntry
    expect(Object.values(entry.nodes).map((node) => node.type)).toEqual([
      'onewire-link',
      'onewire',
      'ds18b20',
    ])
    expect(entry.connections).toHaveLength(3)
    expect(entry.connections?.at(-1)).toEqual({ from: 'onewire.events', to: 'ds18b20.bus' })
    wrapper.unmount()
  })
  it('edits and saves all three existing layers without dropping metadata or changing IDs', async () => {
    const entry = buildEntry(typeOf('onewire-link')!, { data: 2 }, [
      { type: typeOf('onewire')!, config: {} },
      { type: typeOf('ds18b20')!, config: {} },
    ])
    entry.nodes['onewire-link']!.extra = { keep: true }
    entry.future = { keep: true }
    entry.expose = { custom: 'ds18b20.events' }
    const wrapper = dialog(entry)
    await save(wrapper)
    expect(state.setEntries.mock.calls[0]![0]).toEqual([entry])
    wrapper.unmount()
  })
  it('saves branched and unavailable nodes losslessly while editing the label', async () => {
    const entry: DecoderEntry = {
      nodes: {
        u7: { type: 'uart', config: { line: 1, baud: 115200, unknown: 9 } },
        x: { type: 'unavailable', config: { value: 8 } },
        m: { type: 'modbus-rtu' },
      },
      connections: [
        { from: 'source', to: 'u7.logic' },
        { from: 'u7.events', to: 'x.bus' },
        { from: 'u7.events', to: 'm.bus' },
      ],
      expose: { raw: 'x.events' },
      future: { test: 1 },
      import: { format: 'dsview', raw: [1, 2], runnable: true, diagnostics: [] },
    }
    const wrapper = dialog(entry)
    await wrapper.find('input[type="text"]').setValue('Retained graph')
    await save(wrapper)
    expect(state.setEntries.mock.calls[0]![0]).toEqual([{ ...entry, label: 'Retained graph' }])
    wrapper.unmount()
  })
})

it('keeps repeated layer instances separate and uses the selected declared ports', async () => {
  const type = typeOf('uart')!
  const base = {
    ...type,
    name: 'root',
    config: [],
    inputs: [{ port: 'logic', schema: 'logic.samples' }],
    outputs: [
      { port: 'wrong', schema: 'unrelated' },
      { port: 'chosen', schema: 'repeat.events' },
    ],
  }
  const repeat = {
    ...type,
    name: 'repeat',
    config: [],
    inputs: [{ port: 'bus', schema: 'repeat.events' }],
    outputs: [{ port: 'again', schema: 'repeat.events' }],
  }
  state.catalog = [base, repeat]
  const wrapper = dialog()
  await append(wrapper, 'repeat')
  await append(wrapper, 'repeat')
  await save(wrapper)
  const entry = state.setEntries.mock.calls[0]![0][0] as DecoderEntry
  expect(Object.keys(entry.nodes)).toEqual(['root', 'repeat', 'repeat-2'])
  expect(entry.connections).toEqual([
    { from: 'source', to: 'root.logic' },
    { from: 'root.chosen', to: 'repeat.bus' },
    { from: 'repeat.again', to: 'repeat-2.bus' },
  ])
  wrapper.unmount()
})
