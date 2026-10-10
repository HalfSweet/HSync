<script setup lang="ts">
import type { MeasureGeometry } from '@/timeline/measure'

/**
 * The edge measurement over the rows (DESIGN.md §6.5): a dot on each edge (a cursor line for an
 * end that follows the pointer between edges), a line between the ends across the second end's
 * row, a dashed drop from the first end when it is on another row, and the read-out above. A
 * ring marks the edge a click would start from.
 */
defineProps<{
  geometry: MeasureGeometry | null
  /** The time between the ends, or `null` while there is only the hint. */
  primary: string | null
  secondary: string | null
  /** Both edges are chosen, so the read-out is announced. */
  done: boolean
  /** The edge under the pointer that a click picks, in track pixels and rows' y. */
  target: { x: number; y: number } | null
}>()
</script>

<template>
  <div class="pointer-events-none absolute inset-y-0 right-0 left-(--lab) z-[2] overflow-hidden">
    <template v-if="geometry">
      <i
        v-if="geometry.drop"
        class="absolute border-l border-dashed border-ink"
        :style="{
          left: `${geometry.drop.x}px`,
          top: `${geometry.drop.top}px`,
          height: `${geometry.drop.height}px`,
        }"
        aria-hidden="true"
      />
      <i
        v-if="geometry.span"
        class="absolute h-px -translate-y-1/2 bg-ink"
        :style="{
          left: `${geometry.span.left}px`,
          width: `${geometry.span.width}px`,
          top: `${geometry.span.y}px`,
        }"
        aria-hidden="true"
      />
      <template v-for="(end, index) in [geometry.from, geometry.to]" :key="index">
        <i
          v-if="end?.visible && end.edge"
          data-part="dot"
          class="absolute size-[10px] -translate-1/2 rounded-full border-[1.5px] border-canvas bg-ink"
          :style="{ left: `${end.x}px`, top: `${end.top + end.height / 2}px` }"
          aria-hidden="true"
        />
        <i
          v-else-if="end?.visible"
          data-part="cursor"
          class="absolute w-px -translate-x-1/2 bg-ink"
          :style="{ left: `${end.x}px`, top: `${end.top + 3}px`, height: `${end.height - 6}px` }"
          aria-hidden="true"
        />
      </template>
      <div
        v-if="geometry.label && (primary || secondary)"
        :role="done ? 'status' : undefined"
        class="absolute flex -translate-x-1/2 -translate-y-full items-baseline gap-2 rounded-4 bg-ink px-2 py-1 text-small whitespace-nowrap text-canvas tabular-nums shadow-[0_0_0_1px_var(--canvas)]"
        :style="{ left: `${geometry.label.center}px`, top: `${geometry.label.bottom}px` }"
      >
        <b v-if="primary" class="font-semibold">{{ primary }}</b>
        <span v-if="secondary" :class="primary && 'opacity-75'">{{ secondary }}</span>
      </div>
    </template>
    <i
      v-if="target"
      data-part="target"
      class="absolute size-[11px] -translate-1/2 rounded-full border-[1.5px] border-ink"
      :style="{ left: `${target.x}px`, top: `${target.y}px` }"
      aria-hidden="true"
    />
  </div>
</template>
