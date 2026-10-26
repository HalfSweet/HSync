<script setup lang="ts">
import { Minus, Plus } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { analogNumber } from '@/analog/channels'
import type { VerticalScale } from '@/analog/scale'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'
import { APPLE } from '@/shortcuts/chords'
import type { ChannelRef } from '@/timeline/groups'
import { formatVoltsPerDiv } from '@/utils/units'

import TriggerSlot from './TriggerSlot.vue'

/**
 * A channel row's label: its colour, name or decoder role, number and trigger slot. The dot,
 * a double-click on the name or a right-click opens the channel's name and colour. An analog
 * channel shows its volts per division instead of a trigger slot, with buttons to change them.
 */
defineProps<{
  channel: ChannelRef
  condition: TriggerCondition
  menuOpen: boolean
  /** An analog channel's scale. */
  scale?: VerticalScale | null
  /** The user set the scale rather than letting it fit the samples. */
  manual?: boolean
}>()
const emit = defineEmits<{
  trigger: [slot: HTMLElement]
  menu: [at: { x: number; y: number }]
  /** Steps the volts per division along the 1-2-5 sequence. */
  step: [steps: number]
  /** Fits the scale to the samples again. */
  fit: []
}>()

const { t } = useI18n()
const alt = APPLE ? '⌥' : 'Alt'

/** Opens the menu under the element, or at the pointer for a right-click. */
function open(event: MouseEvent): void {
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
  const pointer = event.detail > 0 || event.type === 'contextmenu'
  emit('menu', pointer ? { x: event.clientX, y: event.clientY } : { x: box.left, y: box.bottom })
}
</script>

<template>
  <div
    class="group/channel flex min-w-0 flex-1 flex-col justify-center gap-1 self-stretch pl-6 text-small text-ink-2"
    :class="`wire-${channel.wire}`"
    :title="t('channelMenu.tip')"
    @contextmenu.prevent="open"
  >
    <div class="flex min-w-0 items-center gap-2">
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
        <span>{{ channel.analog ? analogNumber(channel.index) : `D${channel.index}` }}</span>
      </span>
      <TriggerSlot
        v-if="!channel.analog"
        :class="channel.numeric && 'ml-auto'"
        :channel="channel.label"
        :condition="condition"
        :open="menuOpen"
        @toggle="(slot) => emit('trigger', slot)"
      />
    </div>
    <div
      v-if="channel.analog && scale"
      class="flex min-w-0 items-center gap-1 pl-[15px] text-caption text-ink-3"
      :title="t('analog.scaleTip', { key: alt })"
    >
      <span class="font-mono whitespace-nowrap tabular-nums" :class="manual && 'text-ink-2'">{{
        formatVoltsPerDiv(scale.perDiv)
      }}</span>
      <button
        v-if="manual"
        class="rounded-4 px-1 leading-[18px] hover:bg-hover hover:text-ink"
        :title="t('analog.fitTip')"
        @click.stop="emit('fit')"
      >
        {{ t('analog.fit') }}
      </button>
      <span
        class="ml-auto flex flex-none gap-px opacity-0 group-hover/channel:opacity-100 focus-within:opacity-100"
      >
        <button
          class="inline-flex size-[18px] items-center justify-center rounded-4 hover:bg-hover hover:text-ink"
          :aria-label="t('analog.finer', { channel: channel.label })"
          :title="t('analog.finer', { channel: channel.label })"
          @click.stop="emit('step', -1)"
        >
          <Plus :size="12" />
        </button>
        <button
          class="inline-flex size-[18px] items-center justify-center rounded-4 hover:bg-hover hover:text-ink"
          :aria-label="t('analog.coarser', { channel: channel.label })"
          :title="t('analog.coarser', { channel: channel.label })"
          @click.stop="emit('step', 1)"
        >
          <Minus :size="12" />
        </button>
      </span>
    </div>
  </div>
</template>
