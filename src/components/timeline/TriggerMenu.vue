<script setup lang="ts">
import { onMounted, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'

/** The trigger menu of one channel, beside the label column (DESIGN.md §9.7). */
const props = defineProps<{
  channel: string
  wire: number
  condition: TriggerCondition
  /** Offset from the top of the timeline content. */
  top: number
}>()
const emit = defineEmits<{ pick: [condition: TriggerCondition]; close: [] }>()

const CONDITIONS: TriggerCondition[] = ['none', 'rise', 'fall', 'both', 'high', 'low']

const { t } = useI18n()
const menu = useTemplateRef<HTMLElement>('menu')

function items(): HTMLElement[] {
  return [...(menu.value?.querySelectorAll<HTMLElement>('[role="menuitemradio"]') ?? [])]
}

// The keyboard starts on the condition in force.
onMounted(() => {
  items()[CONDITIONS.indexOf(props.condition)]?.focus({ preventScroll: true })
})

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    emit('close')
    return
  }
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
  event.preventDefault()
  const list = items()
  const at = list.indexOf(document.activeElement as HTMLElement)
  const step = event.key === 'ArrowDown' ? 1 : -1
  list[(at < 0 ? 0 : at + step + list.length) % list.length]?.focus()
}
</script>

<template>
  <div
    ref="menu"
    role="menu"
    :aria-label="t('trigger.menuTitle', { channel })"
    class="absolute z-[8] w-(--w-trigger-menu) rounded-6 border border-ink bg-raise p-1 text-small"
    :style="{ left: 'calc(var(--lab) + 4px)', top: `${top}px` }"
    @keydown="onKeydown"
  >
    <div
      class="flex h-6 items-center gap-2 px-2 text-caption whitespace-nowrap text-ink-3"
      :class="`wire-${wire}`"
    >
      <i class="size-[7px] flex-none rounded-full bg-wire" />
      <span>{{ t('trigger.menuTitle', { channel }) }}</span>
    </div>
    <button
      v-for="option in CONDITIONS"
      :key="option"
      role="menuitemradio"
      :aria-checked="option === condition"
      class="flex h-[26px] w-full items-center gap-2 rounded-4 px-2 whitespace-nowrap hover:bg-hover"
      :class="option === condition && 'font-medium'"
      @click="emit('pick', option)"
    >
      <TriggerGlyph :condition="option" />
      <span>{{ t(`trigger.condition.${option}`) }}</span>
      <AppIcon v-if="option === condition" name="check" class="ml-auto" />
    </button>
    <div class="mt-1 border-t border-line px-2 pt-1.5 pb-1 text-caption text-ink-3">
      {{ t('trigger.note') }}
    </div>
  </div>
</template>
