<script setup lang="ts">
import { onMounted } from 'vue'

import AppShell from '@/components/AppShell.vue'
import { useRootPreferences } from '@/composables/useRootPreferences'
import { useCaptureStore } from '@/stores/capture'

useRootPreferences()
const capture = useCaptureStore()

onMounted(() => {
  void capture.init()
  if (import.meta.env.DEV) void import('@/dev/devParams').then((dev) => dev.applyDevParams())
})
</script>

<template>
  <AppShell />
</template>
