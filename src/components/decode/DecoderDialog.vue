<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, reactive, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import {
  buildEntry,
  DECODERS,
  decoderSpec,
  entryChannels,
  type DecoderSpec,
  type OptionSpec,
} from '@/decode/catalog'
import { toUiError } from '@/ipc/errors'
import { useBackendText } from '@/composables/useBackendText'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { baseDecoderId } from '@/timeline/groups'

/**
 * Adds a decoder or changes or removes one (DESIGN.md §9.11): protocol, channel for each role
 * and options, stored as DSView stores decoders so `.dsl` files keep them.
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

const specId = shallowRef(
  (typeof existing.value?.id === 'string' ? existing.value.id : null) ?? DECODERS[0]?.id ?? '',
)
const spec = computed<DecoderSpec | undefined>(() => decoderSpec(specId.value))
const channels = reactive<Record<string, number | null>>({})
const options = reactive<Record<string, string | number>>({})
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
  const current = spec.value
  if (!current) return
  const stored = existing.value && existing.value.id === current.id ? existing.value : undefined
  const storedChannels = stored ? entryChannels(stored) : {}
  const storedOptions = (stored?.options ?? {}) as Record<string, unknown>
  const free = captureChannels.value
    .map((channel) => channel.index)
    .filter((index) => !taken.value.has(index))
  for (const key of Object.keys(channels)) delete channels[key]
  for (const key of Object.keys(options)) delete options[key]
  for (const role of current.roles) {
    channels[role.id] = storedChannels[role.id] ?? (role.required ? (free.shift() ?? null) : null)
  }
  for (const option of current.options) {
    const value = storedOptions[option.id]
    options[option.id] =
      typeof value === 'string' || typeof value === 'number' ? value : option.default
  }
  error.value = null
}
watch(specId, reset, { immediate: true })

const title = computed(() =>
  editing.value === null
    ? t('decoderDialog.addTitle')
    : t('decoderDialog.editTitle', { name: baseDecoderId(spec.value?.label ?? specId.value) }),
)

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
  const current = spec.value
  if (!current) return null
  for (const role of current.roles) {
    if (role.required && channels[role.id] == null)
      return t('decoderDialog.needChannel', { role: role.name })
  }
  if (current.bus === 'spi' && channels.miso == null && channels.mosi == null)
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
  const current = spec.value
  if (!current) return
  error.value = validate()
  if (error.value) return
  const entry = buildEntry(current, { ...channels }, { ...options })
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
      class="flex max-h-[85vh] w-[min(460px,calc(100vw-32px))] flex-col rounded-6 border border-ink bg-raise text-small outline-none"
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
        <label v-if="editing === null" class="flex flex-col gap-1">
          <span class="text-caption text-ink-3">{{ t('decoderDialog.protocol') }}</span>
          <select
            v-model="specId"
            class="h-8 rounded-6 border border-line-2 bg-raise px-2 text-ink"
            :disabled="locked"
          >
            <option v-for="one in DECODERS" :key="one.id" :value="one.id">
              {{ baseDecoderId(one.label) }}
            </option>
          </select>
        </label>
        <fieldset v-if="spec" class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">{{ t('decoderDialog.channels') }}</legend>
          <label v-for="role in spec.roles" :key="role.id" class="flex items-center gap-3">
            <span class="w-14 flex-none font-medium">{{ role.name }}</span>
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
        <fieldset v-if="spec && spec.options.length > 0" class="flex flex-col gap-2">
          <legend class="mb-1 text-caption text-ink-3">{{ t('decoderDialog.options') }}</legend>
          <label v-for="option in spec.options" :key="option.id" class="flex items-center gap-3">
            <span class="w-36 flex-none text-ink-2">{{ optionLabel(option) }}</span>
            <select
              v-if="option.kind === 'choice'"
              v-model="options[option.id]"
              class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink"
              :disabled="locked"
            >
              <option v-for="value in option.values" :key="value" :value="value">
                {{ valueLabel(value) }}
              </option>
            </select>
            <input
              v-else-if="option.kind === 'text'"
              v-model="options[option.id]"
              class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink"
              :disabled="locked"
            />
            <input
              v-else
              v-model.number="options[option.id]"
              type="number"
              class="h-8 min-w-0 flex-1 rounded-6 border border-line-2 bg-raise px-2 text-ink tabular-nums"
              :min="option.min"
              :max="option.max"
              :step="option.step"
              :disabled="locked"
            />
          </label>
        </fieldset>
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
          :disabled="locked || saving"
          @click="save"
        >
          {{ editing === null ? t('decoderDialog.add') : t('decoderDialog.save') }}
        </button>
      </footer>
    </div>
  </div>
</template>
