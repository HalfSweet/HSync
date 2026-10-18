<script setup lang="ts">
import { useElementSize } from '@vueuse/core'
import { computed, useTemplateRef } from 'vue'

import { placeFloat, type FloatAnchor, type FloatBounds } from '@/timeline/float'

/**
 * A read-out floating over the waveforms (DESIGN.md §6.4, §6.5), when the settings put the
 * read-outs there rather than in the status bar: against the edge of the row it reads out, in a
 * semi-transparent box through which the traces behind it still show.
 */
const props = defineProps<{
  anchor: FloatAnchor
  bounds: FloatBounds
  primary: string | null
  secondary: string | null
  /** Announced, as the measurement is once both edges are chosen. */
  announce?: boolean
}>()

const box = useTemplateRef<HTMLElement>('box')
const { width, height } = useElementSize(box, undefined, { box: 'border-box' })
const place = computed(() =>
  placeFloat(props.anchor, { width: width.value, height: height.value }, props.bounds),
)
</script>

<template>
  <div
    ref="box"
    :role="announce ? 'status' : undefined"
    class="pointer-events-none absolute flex items-baseline gap-2 rounded-4 bg-float px-2 py-0.5 text-small whitespace-nowrap tabular-nums"
    :style="{ left: `${place.left}px`, top: `${place.top}px` }"
  >
    <span v-if="primary" class="font-medium text-ink">{{ primary }}</span>
    <span v-if="secondary" :class="primary ? 'text-ink-2' : 'text-ink'">{{ secondary }}</span>
  </div>
</template>
