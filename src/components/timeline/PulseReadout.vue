<script setup lang="ts">
/** The pulse under the pointer: a band over it and a read-out above (DESIGN.md §9.6). */
defineProps<{
  /** Left of the visible part of the pulse, in track pixels. */
  left: number
  width: number
  /** Where the read-out is centred, in track pixels. */
  center: number
  primary: string
  secondary: string | null
}>()
</script>

<template>
  <i
    class="pointer-events-none absolute top-[3px] bottom-[3px] rounded-2 bg-[color-mix(in_srgb,var(--ink)_13%,transparent)]"
    :style="{ left: `${left}px`, width: `${Math.max(width, 1)}px` }"
  />
  <div
    class="pointer-events-none absolute bottom-[calc(100%-2px)] z-[6] flex -translate-x-1/2 items-baseline gap-2 rounded-4 bg-ink px-2 py-1 text-small whitespace-nowrap text-canvas tabular-nums shadow-[0_0_0_1px_var(--canvas)]"
    :style="{ left: `${center}px` }"
  >
    <b class="font-semibold">{{ primary }}</b>
    <span v-if="secondary" class="opacity-75">{{ secondary }}</span>
  </div>
</template>
