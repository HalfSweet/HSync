<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import type { OptionState } from '@/ipc/bindings/OptionState'
import { useBackendText } from '@/composables/useBackendText'
import { useDeviceStore } from '@/stores/device'
import { formatSamplerate } from '@/utils/units'

/**
 * A form for options of the open device (DESIGN.md §4), following the option kinds the
 * driver reports.
 */
defineProps<{ options: OptionState[]; locked: boolean }>()
const { t } = useI18n()
const text = useBackendText()
const device = useDeviceStore()

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
  <div class="flex flex-col gap-2.5">
    <template v-for="option in options" :key="option.key">
      <label v-if="option.kind.type === 'bool'" class="flex items-start gap-2">
        <input
          type="checkbox"
          class="mt-0.5 size-4 flex-none accent-ink"
          :checked="option.value.value === true"
          :disabled="locked || !option.enabled"
          @change="
            device.setOption(option.key, {
              type: 'bool',
              value: ($event.target as HTMLInputElement).checked,
            })
          "
        />
        <span :class="!option.enabled && 'text-ink-3'">{{ t(`device.option.${option.key}`) }}</span>
      </label>
      <label v-else class="flex flex-col gap-1">
        <span class="text-caption text-ink-3">{{ t(`device.option.${option.key}`) }}</span>
        <select
          v-if="option.kind.type === 'choice' || option.kind.type === 'integers'"
          class="h-7 min-w-0 rounded-6 bg-control px-2 text-ink disabled:opacity-55"
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
            class="h-7 w-24 rounded-6 bg-control px-2 text-ink tabular-nums disabled:opacity-55"
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
  </div>
</template>
