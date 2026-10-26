<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, onBeforeUnmount, onMounted, shallowRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import { analogNumber } from '@/analog/channels'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { wireOf, type ChannelRef } from '@/timeline/groups'

/**
 * A channel's context menu on the label column: its name, which saved files
 * keep, and its colour.
 */
const props = defineProps<{ channel: ChannelRef; x: number; y: number }>()
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const root = useTemplateRef<HTMLElement>('root')
const input = useTemplateRef<HTMLInputElement>('input')

const WIRES = [0, 1, 2, 3, 4, 5, 6, 7]

// The name is kept when the menu closes, by Enter or a click elsewhere; Escape drops it.
let cancelled = false
onClickOutside(root, () => emit('close'))
onKeyStroke('Escape', () => {
  cancelled = true
  emit('close')
})
onMounted(() => {
  input.value?.focus()
  input.value?.select()
})
onBeforeUnmount(() => {
  if (!cancelled) void rename()
})

/** The channel's number on the timeline, `D2` or `A0`. */
const number = props.channel.analog ? analogNumber(props.channel.index) : `D${props.channel.index}`
/** The name of a channel nobody named: its number, `2` as DSView names it, or `A0`. */
const unnamed = props.channel.analog ? number : String(props.channel.index)
const current = computed(() => capture.channelNames.get(props.channel.index) ?? '')
const name = shallowRef(current.value === unnamed ? '' : current.value)
const custom = computed(() => channels.colours[props.channel.index] !== undefined)
/** The colour in force, which changes while the menu is open. */
const wire = computed(() => channels.colours[props.channel.index] ?? wireOf(props.channel.index))

/** Keeps the menu inside the window. */
const position = computed(() => ({
  left: `${Math.min(props.x, window.innerWidth - 248)}px`,
  top: `${Math.min(props.y, window.innerHeight - 150)}px`,
}))

/** Gives the channel the name typed, if it changed; an empty name goes back to its number. */
async function rename(): Promise<void> {
  const wanted = name.value.trim()
  const shown = current.value === unnamed ? '' : current.value
  if (wanted !== shown) await capture.renameChannel(props.channel.index, wanted)
}
</script>

<template>
  <div
    ref="root"
    role="dialog"
    :aria-label="t('channelMenu.title', { channel: number })"
    class="fixed z-30 flex w-60 origin-top-left flex-col gap-2.5 rounded-6 bg-menu p-3 text-small"
    :style="position"
  >
    <div class="flex items-center gap-2">
      <i class="size-[7px] flex-none rounded-full bg-wire" :class="`wire-${wire}`" />
      <span class="truncate font-medium">{{ channel.label }}</span>
      <span class="flex flex-none gap-1 font-mono text-caption text-ink-3">
        <span v-if="channel.roleLabel">{{ channel.roleLabel }}</span>
        <span>{{ number }}</span>
      </span>
    </div>
    <label class="flex flex-col gap-1">
      <span class="text-caption text-ink-3">{{ t('channelMenu.name') }}</span>
      <input
        ref="input"
        v-model="name"
        class="h-7 rounded-6 bg-control px-2 text-ink"
        :placeholder="number"
        spellcheck="false"
        @keydown.enter.prevent="emit('close')"
      />
    </label>
    <div class="flex flex-col gap-1">
      <span class="text-caption text-ink-3">{{ t('channelMenu.colour') }}</span>
      <div class="flex items-center gap-1" role="radiogroup" :aria-label="t('channelMenu.colour')">
        <button
          v-for="choice in WIRES"
          :key="choice"
          role="radio"
          :aria-checked="choice === wire"
          :aria-label="t(`channelMenu.wires.${choice}`)"
          :title="t(`channelMenu.wires.${choice}`)"
          class="flex size-5 items-center justify-center rounded-full hover:bg-hover aria-checked:bg-sel"
          :class="`wire-${choice}`"
          @click="channels.setColour(channel.index, choice)"
        >
          <i class="size-3 rounded-full bg-wire" />
        </button>
        <button
          class="ml-auto text-caption whitespace-nowrap text-ink-3 hover:text-ink disabled:opacity-45"
          :disabled="!custom"
          @click="channels.setColour(channel.index, null)"
        >
          {{ t('channelMenu.auto') }}
        </button>
      </div>
    </div>
  </div>
</template>
