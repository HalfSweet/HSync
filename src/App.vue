<script setup lang="ts">
import { useRootPreferences } from '@/composables/useRootPreferences'

useRootPreferences()
</script>

<template>
  <main class="h-full bg-canvas p-4 text-body text-ink-2">{{ $t('app.name') }}</main>
</template>
