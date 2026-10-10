<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'

const props = defineProps<{ channel: string; condition: TriggerCondition; open: boolean }>()
const emit = defineEmits<{ toggle: [slot: HTMLElement] }>()

const { t } = useI18n()
const set = computed(() => props.condition !== 'none')
const tip = computed(() =>
  t('trigger.slotTip', {
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
      open && 'outline-[1.5px] outline-offset-1 outline-ink outline-solid',
      open && !set && 'bg-hover text-ink',
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
