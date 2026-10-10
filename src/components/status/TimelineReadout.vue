<script setup lang="ts">
import { computed } from 'vue'

import { useViewStore } from '@/stores/view'

/**
 * The timeline's read-outs in the status bar (DESIGN.md §10), where they cover no waveform: the
 * edge measurement (§6.5), then the pulse under the pointer (§6.4) after its channel's colour and
 * name. They take the room left over and cut their text short rather than wrap the bar, so the
 * timeline keeps its height while the pointer moves.
 */
const view = useViewStore()
const measure = computed(() => view.readouts.measure)
const pulse = computed(() => view.readouts.pulse)
</script>

<template>
  <div class="flex min-w-0 flex-1 items-baseline gap-4 whitespace-nowrap tabular-nums">
    <span
      v-if="measure"
      data-part="measure"
      :role="measure.done ? 'status' : undefined"
      class="min-w-0 truncate"
    >
      <span v-if="measure.primary" class="font-medium text-ink">{{ measure.primary }}</span>
      <span v-if="measure.secondary" :class="measure.primary && 'ml-2 text-ink-3'">{{
        measure.secondary
      }}</span>
    </span>
    <span
      v-if="pulse"
      data-part="pulse"
      class="min-w-0 truncate"
      :class="`wire-${pulse.channel.wire}`"
    >
      <i class="mr-1.5 inline-block size-[7px] rounded-full bg-wire align-middle" />
      <span :class="pulse.channel.numeric && 'font-mono'">{{ pulse.channel.label }}</span>
      <span class="ml-2 font-medium text-ink">{{ pulse.primary }}</span>
      <span v-if="pulse.secondary" class="ml-2 text-ink-3">{{ pulse.secondary }}</span>
    </span>
  </div>
</template>
