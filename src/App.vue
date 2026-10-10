<script setup lang="ts">
import { onMounted } from 'vue'

import AppShell from '@/components/AppShell.vue'
import { useRootPreferences } from '@/composables/useRootPreferences'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useDeviceStore } from '@/stores/device'

useRootPreferences()
const capture = useCaptureStore()
const device = useDeviceStore()
const decode = useDecodeStore()

onMounted(() => {
  void capture.init()
  void device.init()
  void decode.init()
  if (import.meta.env.DEV) void import('@/dev/devParams').then((dev) => dev.applyDevParams())
})
</script>

<template>
  <AppShell />
</template>
