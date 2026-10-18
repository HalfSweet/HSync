<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, reactive, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import { useDecoderCopy } from '@/composables/useDecoderCopy'
import {
  baseTypes,
  buildEntry,
  channelFields,
  entryChannels,
  entryLabel,
  nodeConfig,
  stackableOn,
  typeOf,
  type FieldValue,
  type NodeEntry,
} from '@/decode/catalog'
import { entryFormat, NUMBER_FORMATS, type NumberFormat } from '@/decode/numberFormat'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { CatalogField } from '@/ipc/bindings/CatalogField'
import { toUiError } from '@/ipc/errors'
import { vIndicator } from '@/motion/indicator'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'

/**
 * Adds a decoder or changes or removes one (DESIGN.md §11): a protocol reading samples, found
 * by searching the catalog, a device decoder stacked on what it puts out, a name, a channel for
 * each line the protocol reads, each layer's options and the number format, stored as the
 * pipeline the capture keeps.
 */
const i18n = useI18n()
const { t } = i18n
const text = useBackendText()
const capture = useCaptureStore()
const decode = useDecodeStore()
const dialog = useTemplateRef<HTMLElement>('dialog')

onKeyStroke('Escape', () => decode.closeEditor())
onMounted(() => dialog.value?.focus())

const editing = computed(() => decode.editor?.index ?? null)
const existing = computed(() =>
  editing.value === null ? undefined : decode.entries[editing.value],
)
/** The stored nodes when editing: the protocol first, then the decoder stacked on it. */
const storedNodes = computed<NodeEntry[]>(() => Object.values(existing.value?.nodes ?? {}))

const copy = useDecoderCopy()
const { typeName, categoryName } = copy
/** The decoder's one-line summary; for a loaded module without one, the file it came from. */
function typeSummary(type: string): string {
  const summary = copy.typeSummary(type)
  if (summary) return summary
  const file = typeOf(type, decode.catalog)?.module?.split(/[\\/]/).pop()
  return file ? t('decoderDialog.loadedFrom', { file }) : ''
}
const fieldLabel = (type: CatalogEntry, field: CatalogField) =>
  copy.fieldLabel(type.name, field.name)
const valueLabel = (type: CatalogEntry, field: CatalogField, value: string) =>
  copy.valueLabel(type.name, field.name, value)

const CATEGORY_ORDER = ['bus', 'line', 'device']
function categoryRank(category: string): number {
  const at = CATEGORY_ORDER.indexOf(category)
  return at < 0 ? CATEGORY_ORDER.length : at
}
const byName = (a: CatalogEntry, b: CatalogEntry) =>
  typeName(a.name).localeCompare(typeName(b.name), i18n.locale.value)

/** The protocols that read samples: buses, then signal lines, each by name. */
const bases = computed(() =>
  baseTypes(decode.catalog).sort(
    (a, b) => categoryRank(a.category) - categoryRank(b.category) || byName(a, b),
  ),
)

/** The protocol, by type name: the stored entry's first node when editing, which stays. */
const picked = shallowRef(storedNodes.value[0]?.type ?? bases.value[0]?.name ?? '')
const base = computed(() => typeOf(picked.value, decode.catalog))
/** The device decoders that read what the protocol puts out. */
const stackable = computed(() =>
  base.value ? stackableOn(base.value, decode.catalog).sort(byName) : [],
)
/** The decoder stacked on the protocol, by type name; empty for none. */
const stacked = shallowRef(storedNodes.value[1]?.type ?? '')
const stackType = computed(() => stackable.value.find((type) => type.name === stacked.value))

const label = shallowRef(existing.value?.label ?? '')
const numberFormat = shallowRef<NumberFormat>(entryFormat(existing.value))
const error = shallowRef<string | null>(null)

const captureChannels = computed(() => capture.summary?.channels ?? [])
const hasChannel = (index: number) =>
  captureChannels.value.some((channel) => channel.index === index)

/** Channels other decoders read, so a new one starts on free channels. */
const taken = computed(() => {
  const used = new Set<number>()
  decode.entries.forEach((entry, index) => {
    if (index !== editing.value)
      for (const use of entryChannels(entry, decode.catalog)) used.add(use.index)
  })
  return used
})

function channelName(index: number): string {
  const name = captureChannels.value.find((channel) => channel.index === index)?.name
  return name && name !== String(index) ? `D${index} · ${name}` : `D${index}`
}

/**
 * A field as the form holds it: a channel or `null` for none, the text of a channel list or
 * of a number (so that what is typed can be checked), a choice, a switch or text.
 */
type FormValue = number | string | boolean | null
type Form = Record<string, FormValue>

const baseForm = reactive<Form>({})
const stackForm = reactive<Form>({})

/** The channels of a list as typed, `0, 1, 2`, also as `D0` or with Chinese commas. */
function parseChannels(list: string): number[] | null {
  const channels: number[] = []
  for (const word of list.split(/[\s,，、]+/).filter(Boolean)) {
    const digits = /^[dD]?(\d+)$/.exec(word)
    if (!digits) return null
    channels.push(Number(digits[1]))
  }
  return channels
}

/** A whole number from a number field's text: `undefined` when empty, `null` when not one. */
function parseInteger(value: FormValue | undefined): number | null | undefined {
  const trimmed = String(value ?? '').trim()
  if (trimmed === '') return undefined
  const number = Number(trimmed)
  return Number.isInteger(number) ? number : null
}

const numbersOf = (value: unknown): number[] =>
  Array.isArray(value)
    ? (value as unknown[]).filter((item): item is number => typeof item === 'number')
    : []

/**
 * A field's form value from `value`, its stored or default value; for a new decoder, channel
 * fields take the next of the `free` channels instead, as many as their defaults name.
 */
function formValue(field: CatalogField, value: unknown, free: number[] | null): FormValue {
  const { kind } = field
  switch (kind.kind) {
    case 'channel':
      if (free) return typeof field.default === 'number' ? (free.shift() ?? null) : null
      return typeof value === 'number' ? value : null
    case 'channels':
      return (free ? free.splice(0, numbersOf(field.default).length) : numbersOf(value)).join(', ')
    case 'integer':
      return typeof value === 'number' ? String(value) : ''
    case 'enum':
      return typeof value === 'string' ? value : (kind.values[0] ?? '')
    case 'bool':
      return value === true
    case 'text':
      return typeof value === 'string' ? value : ''
  }
}

/** Fills `form` for `type`: from `stored` when it is a node of that type, else its defaults. */
function fill(form: Form, type: CatalogEntry | undefined, stored: NodeEntry | undefined): void {
  for (const name of Object.keys(form)) delete form[name]
  if (!type) return
  const config = stored && stored.type === type.name ? nodeConfig(stored, decode.catalog) : null
  const free = config
    ? null
    : captureChannels.value
        .map((channel) => channel.index)
        .filter((index) => !taken.value.has(index))
  for (const field of type.config)
    form[field.name] = formValue(field, config ? config[field.name] : field.default, free)
}

watch(
  picked,
  () => {
    fill(baseForm, base.value, storedNodes.value[0])
    if (!stackable.value.some((type) => type.name === stacked.value)) stacked.value = ''
    error.value = null
  },
  { immediate: true },
)
watch(stacked, () => fill(stackForm, stackType.value, storedNodes.value[1]), {
  immediate: true,
})

/** The protocol's fields naming channels. */
const channelSlots = computed(() => (base.value ? channelFields(base.value) : []))

const optionFields = (type: CatalogEntry) =>
  type.config.filter((field) => field.kind.kind !== 'channel' && field.kind.kind !== 'channels')

/** The layers with options: the protocol, then the decoder stacked on it. */
const layers = computed(() => {
  const all: { type: CatalogEntry; form: Form; fields: CatalogField[] }[] = []
  if (base.value) all.push({ type: base.value, form: baseForm, fields: optionFields(base.value) })
  if (stackType.value)
    all.push({ type: stackType.value, form: stackForm, fields: optionFields(stackType.value) })
  return all.filter((layer) => layer.fields.length > 0)
})

const title = computed(() =>
  existing.value === undefined
    ? t('decoderDialog.addTitle')
    : t('decoderDialog.editTitle', { name: entryLabel(existing.value, typeName) }),
)

// Finding a protocol: by name, summary or type name, listed by category.
const query = shallowRef('')
const found = computed(() => {
  const words = query.value.trim().toLowerCase().split(/\s+/).filter(Boolean)
  const groups: { category: string; types: CatalogEntry[] }[] = []
  for (const type of bases.value) {
    const haystack = [typeName(type.name), typeSummary(type.name), type.name]
      .join(' ')
      .toLowerCase()
    if (!words.every((word) => haystack.includes(word))) continue
    const group = groups.find((one) => one.category === type.category)
    if (group) group.types.push(type)
    else groups.push({ category: type.category, types: [type] })
  }
  return groups
})

function validate(): string | null {
  const type = base.value
  if (!type) return null
  const used: number[] = []
  for (const field of channelSlots.value) {
    const role = fieldLabel(type, field)
    const value = baseForm[field.name]
    if (field.kind.kind === 'channel') {
      if (typeof value === 'number') used.push(value)
      else if (!field.optional) return t('decoderDialog.needChannel', { role })
    } else if (field.kind.kind === 'channels') {
      const list = parseChannels(String(value ?? ''))
      const max = field.kind.max
      if (!list || !list.every(hasChannel) || (max !== null && list.length > max))
        return t('decoderDialog.badChannelList', { field: role })
      if (list.length === 0 && !field.optional) return t('decoderDialog.needChannel', { role })
      used.push(...list)
    }
  }
  const twice = used.find((channel, at) => used.indexOf(channel) !== at)
  if (twice !== undefined) return t('decoderDialog.duplicateChannel', { channel: twice })
  for (const layer of layers.value) {
    for (const field of layer.fields) {
      if (field.kind.kind !== 'integer') continue
      const number = parseInteger(layer.form[field.name])
      if (number === null || (number === undefined && !field.optional))
        return t('decoderDialog.badNumber', { field: fieldLabel(layer.type, field) })
    }
  }
  return null
}

/** A layer's configuration as the form holds it; an optional number left empty stays out. */
function configOf(type: CatalogEntry, form: Form): Record<string, FieldValue> {
  const config: Record<string, FieldValue> = {}
  for (const field of type.config) {
    const value = form[field.name]
    switch (field.kind.kind) {
      case 'channel':
        // An optional line left unused is stored as `null`: left out, it would take its default.
        config[field.name] = typeof value === 'number' ? value : null
        break
      case 'channels':
        config[field.name] = parseChannels(String(value ?? '')) ?? []
        break
      case 'integer':
        config[field.name] = parseInteger(value) ?? undefined
        break
      case 'enum':
      case 'text':
        config[field.name] = typeof value === 'string' ? value : ''
        break
      case 'bool':
        config[field.name] = value === true
        break
    }
  }
  return config
}

const saving = shallowRef(false)

async function commit(next: unknown[]): Promise<void> {
  saving.value = true
  try {
    await decode.setEntries(next)
    decode.closeEditor()
  } catch (reason) {
    error.value = text.error(toUiError(reason))
  } finally {
    saving.value = false
  }
}

async function save(): Promise<void> {
  const type = base.value
  if (!type) return
  error.value = validate()
  if (error.value) return
  const entry = buildEntry(
    type,
    configOf(type, baseForm),
    stackType.value
      ? [{ type: stackType.value, config: configOf(stackType.value, stackForm) }]
      : [],
    label.value,
    numberFormat.value === 'hex' ? undefined : { numbers: numberFormat.value },
  )
  const next: unknown[] = [...decode.entries]
  if (editing.value === null) next.push(entry)
  else next[editing.value] = entry
  await commit(next)
}

async function remove(): Promise<void> {
  if (editing.value === null) return
  await commit(decode.entries.filter((_, index) => index !== editing.value))
}
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center modal-veil"
    @click.self="decode.closeEditor()"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
      tabindex="-1"
      class="flex max-h-[88vh] w-[min(520px,calc(100vw-32px))] flex-col rounded-6 bg-raise text-small outline-none"
    >
      <header
        class="flex h-10 flex-none items-center justify-between border-b border-line pr-2 pl-4"
      >
        <h2 class="font-medium">{{ title }}</h2>
        <button
          class="btn-icon"
          :aria-label="t('decoderDialog.cancel')"
          @click="decode.closeEditor()"
        >
          <AppIcon name="close" />
        </button>
      </header>
      <div class="flex min-h-0 flex-col gap-4 overflow-y-auto p-4">
        <div v-if="editing === null" class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('decoderDialog.protocol') }}</span>
          <label
            class="flex h-8 items-center gap-2 rounded-6 bg-control px-2 text-ink-2 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ink"
          >
            <AppIcon name="search" />
            <input
              v-model="query"
              class="min-w-0 flex-1 border-0 bg-transparent p-0 text-ink outline-none"
              :placeholder="t('decoderDialog.search', { count: bases.length })"
              :aria-label="t('decoderDialog.protocol')"
            />
          </label>
          <div
            role="listbox"
            :aria-label="t('decoderDialog.protocol')"
            class="flex max-h-60 flex-col overflow-y-auto rounded-6 bg-control"
          >
            <div
              v-for="group in found"
              :key="group.category"
              role="group"
              :aria-label="categoryName(group.category)"
              class="flex flex-col"
            >
              <div class="sticky top-0 z-10 bg-control px-2 pt-1.5 pb-0.5 text-caption text-ink-3">
                {{ categoryName(group.category) }}
              </div>
              <button
                v-for="one in group.types"
                :key="one.name"
                role="option"
                :aria-selected="picked === one.name"
                class="flex flex-col items-start gap-0.5 px-2 py-1.5 text-left hover:bg-hover aria-selected:bg-sel"
                @click="picked = one.name"
              >
                <span class="font-medium">{{ typeName(one.name) }}</span>
                <span v-if="typeSummary(one.name)" class="w-full truncate text-caption text-ink-3">
                  {{ typeSummary(one.name) }}
                </span>
              </button>
            </div>
            <p v-if="found.length === 0" class="px-2 py-1.5 text-ink-3">
              {{ t('decoderDialog.noMatch') }}
            </p>
          </div>
        </div>
        <div v-else-if="base" class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('decoderDialog.protocol') }}</span>
          <div class="flex flex-col gap-0.5 rounded-6 bg-control px-2 py-1.5">
            <span class="font-medium">{{ typeName(base.name) }}</span>
            <span v-if="typeSummary(base.name)" class="truncate text-caption text-ink-3">
              {{ typeSummary(base.name) }}
            </span>
          </div>
        </div>

        <div v-if="base && stackable.length > 0" class="flex flex-col gap-1.5">
          <label class="flex flex-col gap-1.5">
            <span class="text-caption text-ink-3">{{ t('decoderDialog.stack') }}</span>
            <select v-model="stacked" class="h-8 w-full rounded-6 bg-control px-2 text-ink">
              <option value="">{{ t('decoderDialog.stackNone') }}</option>
              <option v-for="one in stackable" :key="one.name" :value="one.name">
                {{ typeName(one.name) }}
              </option>
            </select>
          </label>
          <span class="text-ink-3">
            {{ t('decoderDialog.stackHint', { name: typeName(base.name) }) }}
          </span>
        </div>

        <label class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('decoderDialog.label') }}</span>
          <input
            v-model="label"
            type="text"
            class="h-8 w-full rounded-6 bg-control px-2 text-ink"
            :placeholder="t('decoderDialog.labelHint')"
          />
        </label>

        <fieldset v-if="base && channelSlots.length > 0" class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">{{ t('decoderDialog.channels') }}</legend>
          <label v-for="field in channelSlots" :key="field.name" class="flex items-center gap-3">
            <span class="w-40 flex-none truncate text-ink-2" :title="fieldLabel(base, field)">
              {{ fieldLabel(base, field) }}
            </span>
            <select
              v-if="field.kind.kind === 'channel'"
              v-model="baseForm[field.name]"
              class="h-8 min-w-0 flex-1 rounded-6 bg-control px-2 text-ink"
            >
              <option v-if="field.optional" :value="null">{{ t('decoderDialog.unused') }}</option>
              <option
                v-for="channel in captureChannels"
                :key="channel.index"
                :value="channel.index"
              >
                {{ channelName(channel.index) }}
              </option>
            </select>
            <input
              v-else
              v-model="baseForm[field.name]"
              type="text"
              class="h-8 min-w-0 flex-1 rounded-6 bg-control px-2 text-ink tabular-nums"
              :placeholder="t('decoderDialog.channelList')"
              :title="t('decoderDialog.channelList')"
            />
          </label>
        </fieldset>

        <fieldset v-for="layer in layers" :key="layer.type.name" class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">
            {{
              stackType
                ? t('decoderDialog.optionsOf', { name: typeName(layer.type.name) })
                : t('decoderDialog.options')
            }}
          </legend>
          <label v-for="field in layer.fields" :key="field.name" class="flex items-center gap-3">
            <span class="w-40 flex-none truncate text-ink-2" :title="fieldLabel(layer.type, field)">
              {{ fieldLabel(layer.type, field) }}
            </span>
            <select
              v-if="field.kind.kind === 'enum'"
              v-model="layer.form[field.name]"
              class="h-8 min-w-0 flex-1 rounded-6 bg-control px-2 text-ink"
            >
              <option v-for="value in field.kind.values" :key="value" :value="value">
                {{ valueLabel(layer.type, field, value) }}
              </option>
            </select>
            <input
              v-else-if="field.kind.kind === 'bool'"
              v-model="layer.form[field.name]"
              type="checkbox"
              class="accent-ink"
            />
            <input
              v-else-if="field.kind.kind === 'integer'"
              v-model="layer.form[field.name]"
              type="number"
              step="1"
              :min="field.kind.min ?? undefined"
              :max="field.kind.max ?? undefined"
              :placeholder="field.optional ? t('decoderDialog.unset') : undefined"
              class="h-8 min-w-0 flex-1 rounded-6 bg-control px-2 text-ink tabular-nums"
            />
            <input
              v-else
              v-model="layer.form[field.name]"
              type="text"
              class="h-8 min-w-0 flex-1 rounded-6 bg-control px-2 text-ink"
            />
          </label>
        </fieldset>

        <fieldset class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">{{ t('decoderDialog.display') }}</legend>
          <div class="flex items-center gap-3">
            <span class="w-40 flex-none text-ink-2">{{ t('decoderDialog.numberFormat') }}</span>
            <div v-indicator role="group" class="flex gap-0.5 rounded-6 bg-hover p-0.5">
              <button
                v-for="one in NUMBER_FORMATS"
                :key="one"
                class="seg"
                :aria-pressed="numberFormat === one"
                @click="numberFormat = one"
              >
                {{ t(`decoderDialog.formats.${one}`) }}
              </button>
            </div>
          </div>
        </fieldset>

        <p v-if="error" role="alert" class="rise-in rounded-4 bg-alert-wash px-2 py-1.5 text-alert">
          {{ error }}
        </p>
      </div>
      <footer class="flex h-12 flex-none items-center gap-2 border-t border-line px-4">
        <button
          v-if="editing !== null"
          class="btn-ghost text-alert"
          :disabled="saving"
          @click="remove"
        >
          {{ t('decoderDialog.remove') }}
        </button>
        <div class="flex-1" />
        <button class="btn-ghost" @click="decode.closeEditor()">
          {{ t('decoderDialog.cancel') }}
        </button>
        <button
          class="inline-flex h-7 items-center rounded-6 bg-ink px-3 font-medium text-canvas hover:opacity-86 disabled:opacity-55"
          :disabled="saving || !base"
          @click="save"
        >
          {{ editing === null ? t('decoderDialog.add') : t('decoderDialog.save') }}
        </button>
      </footer>
    </div>
  </div>
</template>
