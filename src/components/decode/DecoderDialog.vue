<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, reactive, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import {
  buildEntry,
  decoderSpec,
  entryChannels,
  entryStack,
  stackFor,
  type DecoderSpec,
  type OptionSpec,
} from '@/decode/catalog'
import { toUiError } from '@/ipc/errors'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { baseDecoderId } from '@/timeline/groups'

/**
 * Adds a decoder or changes or removes one (DESIGN.md §9.11): the protocol, found by searching
 * the catalog, the decoders it stacks on, a channel for each role of the bottom one and every
 * layer's options, stored as DSView stores decoders so `.dsl` files keep them.
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
  editing.value === null
    ? undefined
    : (decode.entries[editing.value] as Record<string, unknown> | undefined),
)

const lookup = (id: string) => decoderSpec(id, decode.catalog) ?? decoderSpec(id)

/** The decoder picked (the top of its stack); editing keeps the stored stack. */
const picked = shallowRef<string>(
  (() => {
    const entry = existing.value
    if (!entry || typeof entry.id !== 'string') return decode.catalog[0]?.id ?? ''
    return entryStack(entry).at(-1)?.id ?? entry.id
  })(),
)

/** The stack, bottom first: the stored one when editing, else the one the pick needs. */
const stack = computed<DecoderSpec[]>(() => {
  const entry = existing.value
  if (entry && typeof entry.id === 'string') {
    return [entry.id, ...entryStack(entry).map((layer) => layer.id)].flatMap((id) => {
      const spec = lookup(id)
      return spec ? [spec] : []
    })
  }
  const spec = lookup(picked.value)
  return spec ? (stackFor(spec, decode.catalog) ?? []) : []
})
const bottom = computed(() => stack.value[0])

const channels = reactive<Record<string, number | null>>({})
/** Options by layer position, then by option id. */
const options = reactive<Record<number, Record<string, string | number>>>({})
const error = shallowRef<string | null>(null)

const captureChannels = computed(() => capture.summary?.channels ?? [])

/** Channels other decoders use, so new ones start on free channels. */
const taken = computed(() => {
  const used = new Set<number>()
  decode.entries.forEach((entry, index) => {
    if (index !== editing.value)
      for (const channel of Object.values(entryChannels(entry))) used.add(channel)
  })
  return used
})

function reset(): void {
  const layers = stack.value
  const base = layers[0]
  if (!base) return
  const stored = existing.value && existing.value.id === base.id ? existing.value : undefined
  const storedChannels = stored ? entryChannels(stored) : {}
  const storedStack = stored ? entryStack(stored) : []
  const free = captureChannels.value
    .map((channel) => channel.index)
    .filter((index) => !taken.value.has(index))
  for (const key of Object.keys(channels)) delete channels[key]
  for (const key of Object.keys(options)) delete options[Number(key)]
  for (const role of base.roles) {
    channels[role.id] = storedChannels[role.id] ?? (role.required ? (free.shift() ?? null) : null)
  }
  layers.forEach((layer, position) => {
    const saved =
      position === 0
        ? ((stored?.options ?? {}) as Record<string, unknown>)
        : (storedStack.find((one) => one.id === layer.id)?.options ?? {})
    options[position] = Object.fromEntries(
      layer.options.map((option) => {
        const value = saved[option.id]
        return [
          option.id,
          typeof value === 'string' || typeof value === 'number' ? value : option.default,
        ]
      }),
    )
  })
  error.value = null
}
watch(stack, reset, { immediate: true })

const title = computed(() =>
  editing.value === null
    ? t('decoderDialog.addTitle')
    : t('decoderDialog.editTitle', {
        name: baseDecoderId(stack.value.at(-1)?.label ?? picked.value),
      }),
)

// Finding a decoder: by name, long name, description, id or tag.
const query = shallowRef('')
const found = computed(() => {
  const words = query.value.trim().toLowerCase().split(/\s+/).filter(Boolean)
  return decode.catalog.filter((spec) => {
    const haystack = [spec.label, spec.longname, spec.desc, spec.id, ...(spec.tags ?? [])]
      .join(' ')
      .toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
})

function optionLabel(option: OptionSpec): string {
  const key = `decoderDialog.option.${option.id}`
  return i18n.te(key) ? t(key as never) : option.desc || option.id
}

function valueLabel(value: string | number): string {
  const key = `decoderDialog.value.${value}`
  return typeof value === 'string' && i18n.te(key) ? t(key as never) : String(value)
}

function channelName(index: number): string {
  const name = captureChannels.value.find((channel) => channel.index === index)?.name
  return name && name !== String(index) ? `D${index} · ${name}` : `D${index}`
}

function validate(): string | null {
  const base = bottom.value
  if (!base) return t('decoderDialog.noStack')
  for (const role of base.roles) {
    if (role.required && channels[role.id] == null)
      return t('decoderDialog.needChannel', { role: role.name })
  }
  if (base.bus === 'spi' && channels.miso == null && channels.mosi == null)
    return t('decoderDialog.needData')
  return null
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
  error.value = validate()
  const [base, ...stacked] = stack.value
  if (error.value || !base) return
  const entry = buildEntry(
    base,
    { ...channels },
    { ...options[0] },
    stacked.map((spec, position) => ({ spec, options: { ...options[position + 1] } })),
  )
  const next = [...decode.entries]
  if (editing.value === null) next.push(entry)
  else next[editing.value] = { ...(existing.value ?? {}), ...entry }
  await commit(next)
}

async function remove(): Promise<void> {
  if (editing.value === null) return
  await commit(decode.entries.filter((_, index) => index !== editing.value))
}

const locked = computed(() => capture.capturing)
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center bg-veil"
    @click.self="decode.closeEditor()"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
      tabindex="-1"
      class="flex max-h-[88vh] w-[min(520px,calc(100vw-32px))] flex-col rounded-6 border border-ink bg-raise text-small outline-none"
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
        <p v-if="locked" class="text-ink-3">{{ t('decoderDialog.busy') }}</p>

        <div v-if="editing === null" class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('decoderDialog.protocol') }}</span>
          <label
            class="flex h-8 items-center gap-2 rounded-6 border border-line-2 bg-raise px-2 text-ink-2 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ink"
          >
            <AppIcon name="search" />
            <input
              v-model="query"
              class="min-w-0 flex-1 border-0 bg-transparent p-0 text-ink outline-none"
              :placeholder="t('decoderDialog.search', { count: decode.catalog.length })"
              :aria-label="t('decoderDialog.protocol')"
              :disabled="locked"
            />
          </label>
          <div
            role="listbox"
            :aria-label="t('decoderDialog.protocol')"
            class="flex max-h-48 flex-col overflow-y-auto rounded-6 border border-line"
          >
            <button
              v-for="one in found"
              :key="one.id"
              role="option"
              :aria-selected="picked === one.id"
              class="flex flex-col items-start gap-0.5 px-2 py-1.5 text-left hover:bg-hover aria-selected:bg-sel"
              :disabled="locked"
              @click="picked = one.id"
            >
              <span class="flex w-full items-baseline gap-2">
                <span class="font-medium">{{ baseDecoderId(one.label) }}</span>
                <span v-if="one.longname" class="min-w-0 truncate text-caption text-ink-3">{{
                  one.longname
                }}</span>
              </span>
            </button>
            <p v-if="found.length === 0" class="px-2 py-1.5 text-ink-3">
              {{ t('decoderDialog.noMatch') }}
            </p>
          </div>
          <p v-if="stack.length > 1" class="text-ink-3">
            {{
              t('decoderDialog.stacksOn', {
                chain: stack.map((spec) => baseDecoderId(spec.label)).join(' → '),
              })
            }}
          </p>
          <p v-else-if="stack.length === 0" role="alert" class="text-alert">
            {{ t('decoderDialog.noStack') }}
          </p>
        </div>

        <fieldset v-if="bottom" class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">{{ t('decoderDialog.channels') }}</legend>
          <label
            v-for="role in bottom.roles"
            :key="role.id"
            class="flex items-center gap-3"
            :title="role.desc"
          >
            <span class="w-20 flex-none truncate font-medium">{{ role.name }}</span>
            <select
              v-model="channels[role.id]"
              class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink"
              :disabled="locked"
            >
              <option v-if="!role.required" :value="null">{{ t('decoderDialog.unused') }}</option>
              <option
                v-for="channel in captureChannels"
                :key="channel.index"
                :value="channel.index"
              >
                {{ channelName(channel.index) }}
              </option>
            </select>
          </label>
        </fieldset>

        <template v-for="(layer, position) in stack" :key="layer.id">
          <fieldset
            v-if="layer.options.length > 0 && options[position]"
            class="flex flex-col gap-2"
          >
            <legend class="mb-1 text-caption text-ink-3">
              {{
                stack.length > 1
                  ? t('decoderDialog.optionsOf', { name: baseDecoderId(layer.label) })
                  : t('decoderDialog.options')
              }}
            </legend>
            <label v-for="option in layer.options" :key="option.id" class="flex items-center gap-3">
              <span class="w-40 flex-none text-ink-2" :title="option.desc">{{
                optionLabel(option)
              }}</span>
              <select
                v-if="option.kind === 'choice'"
                v-model="options[position]![option.id]"
                class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink"
                :disabled="locked"
              >
                <option v-for="value in option.values" :key="value" :value="value">
                  {{ valueLabel(value) }}
                </option>
              </select>
              <input
                v-else-if="option.kind === 'text'"
                v-model="options[position]![option.id]"
                class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink"
                :disabled="locked"
              />
              <input
                v-else
                v-model.number="options[position]![option.id]"
                type="number"
                class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink tabular-nums"
                :min="option.min"
                :max="option.max"
                :step="option.step"
                :disabled="locked"
              />
            </label>
          </fieldset>
        </template>

        <p v-if="error" role="alert" class="rounded-4 bg-alert-wash px-2 py-1.5 text-alert">
          {{ error }}
        </p>
      </div>
      <footer class="flex h-12 flex-none items-center gap-2 border-t border-line px-4">
        <button
          v-if="editing !== null"
          class="btn-ghost text-alert"
          :disabled="locked || saving"
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
          :disabled="locked || saving || !bottom"
          @click="save"
        >
          {{ editing === null ? t('decoderDialog.add') : t('decoderDialog.save') }}
        </button>
      </footer>
    </div>
  </div>
</template>
