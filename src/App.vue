<script setup lang="ts">
import { onMounted } from 'vue'

import AppShell from '@/components/AppShell.vue'
import { useRootPreferences } from '@/composables/useRootPreferences'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'

useRootPreferences()
const capture = useCaptureStore()
const device = useDeviceStore()

onMounted(() => {
  void capture.init()
  void device.init()
  if (import.meta.env.DEV) void import('@/dev/devParams').then((dev) => dev.applyDevParams())
})
</script>

<template>
  <AppShell />
</template>
