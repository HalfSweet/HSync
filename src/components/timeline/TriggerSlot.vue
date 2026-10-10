<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { useTriggerStore } from '@/stores/trigger'

const props = defineProps<{ channel: string; condition: TriggerCondition; open: boolean }>()
const emit = defineEmits<{ toggle: [slot: HTMLElement] }>()

const { t } = useI18n()
const trigger = useTriggerStore()
const set = computed(() => props.condition !== 'none')
/** A multi-stage or serial trigger is in use: the row's condition waits for the simple one. */
const dormant = computed(() => trigger.activeMode !== 'simple')
const tip = computed(() =>
  dormant.value
    ? t('triggerPanel.dormant', {
        channel: props.channel,
        mode: t(`triggerPanel.modes.${trigger.activeMode}`),
      })
    : t('trigger.slotTip', {
        channel: props.channel,
        condition: t(`trigger.condition.${props.condition}`),
        action: t(set.value ? 'trigger.slotChange' : 'trigger.slotSet'),
      }),
)
</script>

<template>
  <button
    class="inline-flex h-[min(16px,calc(var(--h-trc)-2px))] w-5 flex-none items-center justify-center rounded-3"
    :class="[
      set ? 'bg-ink text-canvas hover:opacity-86' : 'text-trace-2 hover:bg-hover hover:text-ink',
      open && !set && 'bg-hover text-ink',
      dormant && 'opacity-40',
    ]"
    aria-haspopup="menu"
    :aria-expanded="open"
    :aria-label="tip"
    :title="tip"
    @click="emit('toggle', $event.currentTarget as HTMLElement)"
  >
    <TriggerGlyph :condition="condition" />
  </button>
</template>
