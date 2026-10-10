<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, onBeforeUnmount, onMounted, shallowRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { wireOf, type ChannelRef } from '@/timeline/groups'

/**
 * A channel's context menu on the label column (DESIGN.md §2.1): its name, which saved files
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

const current = computed(() => capture.channelNames.get(props.channel.index) ?? '')
const name = shallowRef(current.value === String(props.channel.index) ? '' : current.value)
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
  const shown = current.value === String(props.channel.index) ? '' : current.value
  if (wanted !== shown) await capture.renameChannel(props.channel.index, wanted)
}
</script>

<template>
  <div
    ref="root"
    role="dialog"
    :aria-label="t('channelMenu.title', { channel: `D${channel.index}` })"
    class="fixed z-30 flex w-60 flex-col gap-2.5 rounded-6 border border-ink bg-raise p-3 text-small"
    :style="position"
  >
    <div class="flex items-center gap-2">
      <i class="size-[7px] flex-none rounded-full bg-wire" :class="`wire-${wire}`" />
      <span class="truncate font-medium">{{ channel.label }}</span>
      <span class="flex flex-none gap-1 font-mono text-caption text-ink-3">
        <span v-if="channel.roleLabel">{{ channel.roleLabel }}</span>
        <span>D{{ channel.index }}</span>
      </span>
    </div>
    <label class="flex flex-col gap-1">
      <span class="text-caption text-ink-3">{{ t('channelMenu.name') }}</span>
      <input
        ref="input"
        v-model="name"
        class="h-7 rounded-6 border border-line-2 bg-raise px-2 text-ink"
        :placeholder="`D${channel.index}`"
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
          class="flex size-5 items-center justify-center rounded-full aria-checked:outline-[1.5px] aria-checked:outline-offset-1 aria-checked:outline-ink aria-checked:outline-solid"
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
