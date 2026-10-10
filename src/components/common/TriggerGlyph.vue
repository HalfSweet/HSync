<script setup lang="ts">
/**
 * The picture of a trigger condition (DESIGN.md §9.7): drawn like a trace, 1.5 px round line,
 * with a 26 % wash on the high side.
 */
import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'

defineProps<{ condition: TriggerCondition }>()

const LINES: Record<TriggerCondition, string> = {
  none: 'M6 8h4',
  rise: 'M2.5 11.5H8v-7h5.5',
  fall: 'M2.5 4.5H8v7h5.5',
  both: 'M2.5 4.5h3l5 7h3M2.5 11.5h3l5-7h3',
  high: 'M2.5 4.5h11',
  low: 'M2.5 11.5h11',
}

const WASHES: Partial<Record<TriggerCondition, string>> = {
  rise: 'M8 4.5h5.5v7H8z',
  fall: 'M2.5 4.5H8v7H2.5z',
  high: 'M2.5 4.5h11v7h-11z',
}
</script>

<template>
  <svg class="icon" viewBox="0 0 16 16" aria-hidden="true">
    <path
      v-if="WASHES[condition]"
      :d="WASHES[condition]"
      class="fill-current stroke-none opacity-[.26]"
    />
    <path :d="LINES[condition]" />
  </svg>
</template>
