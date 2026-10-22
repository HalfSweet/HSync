<script setup lang="ts">
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useSettingsStore } from '@/stores/settings'
import { useStreamStore } from '@/stores/stream'

import DecoderDialog from './decode/DecoderDialog.vue'
import DecoderImportWarnings from './decode/DecoderImportWarnings.vue'

import AppHeader from './header/AppHeader.vue'
import ImportDialog from './import/ImportDialog.vue'
import OverviewStrip from './overview/OverviewStrip.vue'
import SettingsPage from './settings/SettingsPage.vue'
import StatusBar from './status/StatusBar.vue'
import StreamPanel from './stream/StreamPanel.vue'
import TimelineView from './timeline/TimelineView.vue'

const capture = useCaptureStore()
const stream = useStreamStore()
const decode = useDecodeStore()
const settings = useSettingsStore()
</script>

<template>
  <div class="flex h-full min-h-[640px] flex-col overflow-hidden bg-chrome text-body text-ink">
    <AppHeader />
    <div class="flex min-h-0 flex-1">
      <main class="relative flex min-w-0 flex-1 flex-col overflow-clip">
        <DecoderImportWarnings />
        <OverviewStrip />
        <TimelineView />
        <Transition name="drawer">
          <StreamPanel v-if="stream.open" />
        </Transition>
      </main>
    </div>
    <StatusBar />
    <Transition name="dialog">
      <DecoderDialog v-if="decode.editor" />
    </Transition>
    <Transition name="dialog">
      <ImportDialog v-if="capture.pendingImport" />
    </Transition>
    <Transition name="dialog">
      <SettingsPage v-if="settings.open" />
    </Transition>
  </div>
</template>
