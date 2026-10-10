<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import type { OptionKey } from '@/ipc/bindings/OptionKey'
import type { OptionState } from '@/ipc/bindings/OptionState'
import { useBackendText } from '@/composables/useBackendText'
import { useCaptureStore } from '@/stores/capture'
import { DEMO_DRIVER, useDeviceStore } from '@/stores/device'
import { formatSamplerate } from '@/utils/units'

/**
 * Every option of the open device but rate and depth, which the acquisition menu holds
 * (DESIGN.md §11). The form follows the option kinds the driver reports.
 */
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const text = useBackendText()
const device = useDeviceStore()
const capture = useCaptureStore()
const dialog = useTemplateRef<HTMLElement>('dialog')

onKeyStroke('Escape', () => emit('close'))
onMounted(() => dialog.value?.focus())

const HIDDEN: OptionKey[] = ['samplerate', 'sampleLimit']
const options = computed(() =>
  (device.state?.options ?? []).filter((option) => !HIDDEN.includes(option.key)),
)
const locked = computed(() => capture.running)
const title = computed(() => {
  const info = device.state?.info
  const name = info?.driver === DEMO_DRIVER ? t('device.demo') : (info?.model ?? '')
  return t('device.dialog.title', { device: name })
})

function choose(option: OptionState, raw: string): void {
  if (option.kind.type === 'choice')
    void device.setOption(option.key, { type: 'choice', value: raw })
  else if (option.kind.type === 'integers')
    void device.setOption(option.key, { type: 'integer', value: Number(raw) })
}

function setNumber(option: OptionState, raw: string): void {
  const value = Number(raw)
  if (Number.isFinite(value)) void device.setOption(option.key, { type: 'number', value })
}

function current(option: OptionState): string {
  return String(option.value.value)
}
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center modal-veil"
    @click.self="emit('close')"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="title"
      tabindex="-1"
      class="flex max-h-[80vh] w-[min(440px,calc(100vw-32px))] flex-col rounded-6 bg-raise text-small outline-none"
    >
      <header class="flex h-10 items-center justify-between border-b border-line pr-2 pl-4">
        <h2 class="font-medium">{{ title }}</h2>
        <button class="btn-icon" :aria-label="t('device.dialog.close')" @click="emit('close')">
          <AppIcon name="close" />
        </button>
      </header>
      <div class="flex flex-col gap-3 overflow-y-auto p-4">
        <p v-if="locked" class="text-ink-3">{{ t('device.dialog.locked') }}</p>
        <template v-for="option in options" :key="option.key">
          <label v-if="option.kind.type === 'bool'" class="flex items-center gap-2">
            <input
              type="checkbox"
              class="size-4 accent-ink"
              :checked="option.value.value === true"
              :disabled="locked || !option.enabled"
              @change="
                device.setOption(option.key, {
                  type: 'bool',
                  value: ($event.target as HTMLInputElement).checked,
                })
              "
            />
            <span :class="!option.enabled && 'text-ink-3'">{{
              t(`device.option.${option.key}`)
            }}</span>
          </label>
          <label v-else class="flex flex-col gap-1">
            <span class="text-caption text-ink-3">{{ t(`device.option.${option.key}`) }}</span>
            <select
              v-if="option.kind.type === 'choice' || option.kind.type === 'integers'"
              class="h-8 rounded-6 border border-line-2 bg-raise px-2 text-ink disabled:opacity-55"
              :value="current(option)"
              :disabled="locked || !option.enabled"
              @change="choose(option, ($event.target as HTMLSelectElement).value)"
            >
              <template v-if="option.kind.type === 'choice'">
                <option v-for="choice in option.kind.choices" :key="choice.id" :value="choice.id">
                  {{ text.label(choice.label) }}
                </option>
              </template>
              <template v-else>
                <option v-for="value in option.kind.values" :key="value" :value="String(value)">
                  {{ formatSamplerate(value) }}
                </option>
              </template>
            </select>
            <span v-else-if="option.kind.type === 'range'" class="flex items-center gap-2">
              <input
                type="number"
                class="h-8 w-28 rounded-6 border border-line-2 bg-raise px-2 text-ink tabular-nums disabled:opacity-55"
                :min="option.kind.min"
                :max="option.kind.max"
                :step="option.kind.step"
                :value="option.value.value"
                :disabled="locked || !option.enabled"
                @change="setNumber(option, ($event.target as HTMLInputElement).value)"
              />
              <span v-if="option.key === 'vth'" class="text-ink-3">V</span>
            </span>
          </label>
        </template>
        <p
          v-if="device.error"
          role="alert"
          class="rise-in rounded-4 bg-alert-wash px-2 py-1.5 text-alert"
        >
          {{ text.error(device.error) }}
        </p>
      </div>
    </div>
  </div>
</template>
