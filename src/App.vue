<script setup lang="ts">
import { onMounted } from 'vue'

import AppShell from '@/components/AppShell.vue'
import { useRootPreferences } from '@/composables/useRootPreferences'
import { useWindowTitle } from '@/composables/useWindowTitle'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useDeviceStore } from '@/stores/device'
import { useSessionStore } from '@/stores/session'
import { useUpdateStore } from '@/stores/update'

useRootPreferences()
useWindowTitle()
const capture = useCaptureStore()
const device = useDeviceStore()
const decode = useDecodeStore()
const update = useUpdateStore()
// Remembers each driver's settings and brings them back when it opens.
useSessionStore()

onMounted(() => {
  void capture.init()
  void device.init()
  void decode.init()
  void update.init()
  if (import.meta.env.DEV) void import('@/dev/devParams').then((dev) => dev.applyDevParams())
})
</script>

<template>
  <AppShell />
</template>
