<script setup lang="ts">
import { shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'

/**
 * The bottom edge of a channel row in the label column (DESIGN.md §6.1): dragging it asks for
 * the row's height to follow the pointer, then says when the drag ends; a double-click asks for
 * the row's default height.
 */
const props = defineProps<{ channel: string; height: number }>()
const emit = defineEmits<{ resize: [height: number]; end: []; reset: [] }>()

const { t } = useI18n()

/** Where the drag started, and the row's height then. */
const grip = shallowRef<{ y: number; height: number } | null>(null)

function onDown(event: PointerEvent): void {
  if (event.button !== 0) return
  // No text selection in the label column while dragging.
  event.preventDefault()
  grip.value = { y: event.clientY, height: props.height }
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
}

function onMove(event: PointerEvent): void {
  const from = grip.value
  if (from) emit('resize', from.height + event.clientY - from.y)
}

function onUp(): void {
  if (!grip.value) return
  grip.value = null
  emit('end')
}
</script>

<template>
  <div
    class="group absolute inset-x-0 h-[5px] cursor-row-resize touch-none"
    role="separator"
    aria-orientation="horizontal"
    :aria-label="t('channelRow.resize', { channel })"
    :title="t('channelRow.resizeTip')"
    @pointerdown="onDown"
    @pointermove="onMove"
    @pointerup="onUp"
    @pointercancel="onUp"
    @dblclick="emit('reset')"
  >
    <i
      class="absolute inset-x-0 top-[2px] h-px bg-ink-3"
      :class="grip ? 'block' : 'hidden group-hover:block'"
    />
  </div>
</template>
