<script setup lang="ts">
import AppHeader from './header/AppHeader.vue'
</script>

<template>
  <div class="flex h-full min-h-[640px] flex-col overflow-hidden bg-chrome text-body text-ink">
    <AppHeader />
    <div class="flex min-h-0 flex-1">
      <main class="flex min-w-0 flex-1 flex-col bg-canvas" />
    </div>
  </div>
</template>
