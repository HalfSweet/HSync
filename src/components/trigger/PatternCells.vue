<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { conditionForKey, nextCondition, type Pattern } from '@/trigger/advanced'

/**
 * One trigger pattern: a cell per channel showing its condition. Clicking a cell steps through
 * the conditions (right-click backwards); typing X, 0, 1, R, F or C sets one, as in DSView.
 */
const props = defineProps<{ modelValue: Pattern; channels: number; label: string }>()
const emit = defineEmits<{ 'update:modelValue': [pattern: Pattern] }>()

const { t } = useI18n()

/** Rows of 16 channels. */
const rows = computed(() => {
  const all = Array.from({ length: props.channels }, (_, index) => index)
  const out: number[][] = []
  for (let at = 0; at < all.length; at += 16) out.push(all.slice(at, at + 16))
  return out
})

function conditionOf(channel: number): TriggerCondition {
  return props.modelValue[channel] ?? 'none'
}

function set(channel: number, condition: TriggerCondition): void {
  const next = { ...props.modelValue }
  if (condition === 'none') delete next[channel]
  else next[channel] = condition
  emit('update:modelValue', next)
}

function tip(channel: number): string {
  return t('triggerPanel.cellTip', {
    channel: `D${channel}`,
    condition: t(`trigger.condition.${conditionOf(channel)}`),
  })
}

function onKeydown(event: KeyboardEvent, channel: number): void {
  const condition = conditionForKey(event.key)
  if (condition) {
    event.preventDefault()
    set(channel, condition)
    return
  }
  const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
  if (step === 0) return
  event.preventDefault()
  const cells = (event.currentTarget as HTMLElement)
    .closest('[role="group"]')
    ?.querySelectorAll<HTMLElement>('button')
  cells?.[channel + step]?.focus()
}
</script>

<template>
  <div role="group" :aria-label="label" class="flex flex-col gap-1">
    <div v-for="(row, at) in rows" :key="at" class="flex gap-0.5">
      <button
        v-for="channel in row"
        :key="channel"
        class="flex w-[21px] flex-none flex-col items-center gap-px rounded-3 pt-0.5 pb-px hover:bg-hover"
        :aria-label="tip(channel)"
        :title="tip(channel)"
        @click="set(channel, nextCondition(conditionOf(channel)))"
        @contextmenu.prevent="set(channel, nextCondition(conditionOf(channel), true))"
        @keydown="onKeydown($event, channel)"
      >
        <span class="font-mono text-caption leading-none text-ink-3 tabular-nums">{{
          channel
        }}</span>
        <span
          class="inline-flex h-4 w-5 items-center justify-center rounded-3"
          :class="conditionOf(channel) === 'none' ? 'text-trace-2' : 'bg-ink text-canvas'"
        >
          <TriggerGlyph :condition="conditionOf(channel)" />
        </span>
      </button>
    </div>
  </div>
</template>
