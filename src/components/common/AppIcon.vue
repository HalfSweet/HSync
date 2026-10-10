<script setup lang="ts">
/** The 16 px stroke icons of the design. */
export type IconName =
  | 'chevron-down'
  | 'chevron-left'
  | 'chevron-right'
  | 'trigger'
  | 'play'
  | 'stop'
  | 'search'
  | 'export'
  | 'settings'
  | 'plus'
  | 'minus'
  | 'stream'
  | 'copy'
  | 'close'
  | 'check'
  | 'fit'

defineProps<{ name: IconName }>()
</script>

<template>
  <svg
    v-if="name === 'play'"
    class="size-4 flex-none fill-current"
    viewBox="0 0 16 16"
    aria-hidden="true"
  >
    <path d="M5 3.2v9.6L12.8 8z" />
  </svg>
  <svg
    v-else-if="name === 'stop'"
    class="size-4 flex-none fill-current"
    viewBox="0 0 16 16"
    aria-hidden="true"
  >
    <rect x="4" y="4" width="8" height="8" rx="1" />
  </svg>
  <svg v-else class="icon" viewBox="0 0 16 16" aria-hidden="true">
    <path v-if="name === 'chevron-down'" d="M4 6l4 4 4-4" />
    <path v-else-if="name === 'chevron-left'" d="M9.5 4.5L6 8l3.5 3.5" />
    <path v-else-if="name === 'chevron-right'" d="M6.5 4.5L10 8l-3.5 3.5" />
    <path v-else-if="name === 'trigger'" d="M2.5 11.5h4.5v-7h6.5" />
    <template v-else-if="name === 'search'">
      <circle cx="7" cy="7" r="4.25" />
      <path d="M10.2 10.2L13.5 13.5" />
    </template>
    <path v-else-if="name === 'export'" d="M8 2.5v7.5M5 7l3 3 3-3M3 13h10" />
    <template v-else-if="name === 'settings'">
      <path d="M2.5 5h5.5M11.5 5h2M2.5 11h2M8 11h5.5" />
      <circle cx="9.75" cy="5" r="1.75" />
      <circle cx="6.25" cy="11" r="1.75" />
    </template>
    <path v-else-if="name === 'plus'" d="M8 3.5v9M3.5 8h9" />
    <path v-else-if="name === 'minus'" d="M3.5 8h9" />
    <path v-else-if="name === 'stream'" d="M3 4.5h10M3 8h6.5M3 11.5h8.5" />
    <template v-else-if="name === 'copy'">
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M3 10.5v-6A1.5 1.5 0 0 1 4.5 3h6" />
    </template>
    <path v-else-if="name === 'close'" d="M4 4l8 8M12 4l-8 8" />
    <path v-else-if="name === 'check'" d="M3.5 8.5l3 3 6-7" />
    <path v-else-if="name === 'fit'" d="M2.5 8h11M5 5.5L2.5 8 5 10.5M11 5.5L13.5 8 11 10.5" />
  </svg>
</template>
