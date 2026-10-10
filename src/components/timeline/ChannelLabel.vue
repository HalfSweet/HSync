<script setup lang="ts">
import { useI18n } from 'vue-i18n'

import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import type { ChannelRef } from '@/timeline/groups'

import TriggerSlot from './TriggerSlot.vue'

/**
 * A channel row's label: its colour, name or decoder role, number and trigger slot. The dot,
 * a double-click on the name or a right-click opens the channel's name and colour.
 */
defineProps<{ channel: ChannelRef; condition: TriggerCondition; menuOpen: boolean }>()
const emit = defineEmits<{
  trigger: [slot: HTMLElement]
  menu: [at: { x: number; y: number }]
}>()

const { t } = useI18n()

/** Opens the menu under the element, or at the pointer for a right-click. */
function open(event: MouseEvent): void {
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const pointer = event.detail > 0 || event.type === 'contextmenu'
  emit('menu', pointer ? { x: event.clientX, y: event.clientY } : { x: box.left, y: box.bottom })
}
</script>

<template>
  <div
    class="flex min-w-0 flex-1 items-center gap-2 pl-6 text-small text-ink-2"
    :class="`wire-${channel.wire}`"
    :title="t('channelMenu.tip')"
    @contextmenu.prevent="open"
  >
    <button
      class="-m-1 flex flex-none rounded-full p-1 hover:bg-hover"
      :aria-label="t('channelMenu.title', { channel: channel.label })"
      @click.stop="open"
    >
      <i class="size-[7px] rounded-full bg-wire" />
    </button>
    <span class="truncate" :class="channel.numeric && 'font-mono'" @dblclick.prevent="open">{{
      channel.label
    }}</span>
    <span
      v-if="!channel.numeric"
      class="ml-auto flex flex-none gap-1 font-mono text-caption leading-none whitespace-nowrap text-ink-3 max-[640px]:hidden"
    >
      <span v-if="channel.roleLabel">{{ channel.roleLabel }}</span>
      <span>D{{ channel.index }}</span>
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
