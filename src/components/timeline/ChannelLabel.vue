<script setup lang="ts">
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import type { ChannelRef } from '@/timeline/groups'

import TriggerSlot from './TriggerSlot.vue'

defineProps<{ channel: ChannelRef; condition: TriggerCondition; menuOpen: boolean }>()
const emit = defineEmits<{ trigger: [slot: HTMLElement] }>()
</script>

<template>
  <div
    class="flex min-w-0 flex-1 items-center gap-2 pl-6 text-small text-ink-2"
    :class="`wire-${channel.wire}`"
  >
    <i class="size-[7px] flex-none rounded-full bg-wire" />
    <span class="truncate" :class="channel.numeric && 'font-mono'">{{ channel.label }}</span>
    <span
      v-if="!channel.numeric"
      class="ml-auto font-mono text-caption leading-none text-ink-3 max-[640px]:hidden"
    >
      D{{ channel.index }}
    </span>
    <TriggerSlot
      :class="channel.numeric && 'ml-auto'"
      :channel="channel.label"
      :condition="condition"
      :open="menuOpen"
      @toggle="(slot) => emit('trigger', slot)"
    />
  </div>
</template>
