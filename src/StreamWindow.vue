<script setup lang="ts">
import { setLucideProps } from '@lucide/vue'
import { onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import StreamPanel from '@/components/stream/StreamPanel.vue'
import { useRootPreferences } from '@/composables/useRootPreferences'
import { setWindowTitle, tellMain } from '@/ipc/windowLink'
import { useDecodeStore } from '@/stores/decode'
import { useStreamWindowGuest } from '@/windows/streamGuest'

/**
 * The data stream's own window (DESIGN.md §9): the panel alone, filling the window, fed by the
 * main window with the capture it shows and listing the events from the backend itself.
 */
setLucideProps({ size: 16, strokeWidth: 1.5, nonScalingStroke: true, class: 'flex-none' })
useRootPreferences()
useStreamWindowGuest()
const { t } = useI18n()
const decode = useDecodeStore()

watch(
  () => t('stream.windowTitle'),
  (title) => {
    setWindowTitle(title).catch((error: unknown) => console.error('window title failed', error))
  },
  { immediate: true },
)

/** Back beside the timeline: the main window docks the panel and closes this window. */
function dock(): void {
  tellMain({ type: 'dock' }).catch((error: unknown) => console.error('main window', error))
}

onMounted(() => void decode.init())
</script>

<template>
  <div class="flex h-full overflow-hidden bg-canvas text-body text-ink">
    <StreamPanel detached @dock="dock" />
  </div>
</template>
