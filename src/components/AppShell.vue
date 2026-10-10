<script setup lang="ts">
import { useDecodeStore } from '@/stores/decode'
import { useStreamStore } from '@/stores/stream'

import DecoderDialog from './decode/DecoderDialog.vue'

import AppHeader from './header/AppHeader.vue'
import OverviewStrip from './overview/OverviewStrip.vue'
import StatusBar from './status/StatusBar.vue'
import StreamPanel from './stream/StreamPanel.vue'
import TimelineView from './timeline/TimelineView.vue'

const stream = useStreamStore()
const decode = useDecodeStore()
</script>

<template>
  <div class="flex h-full min-h-[640px] flex-col overflow-hidden bg-chrome text-body text-ink">
    <AppHeader />
    <div class="flex min-h-0 flex-1">
      <main class="flex min-w-0 flex-1 flex-col">
        <OverviewStrip />
        <TimelineView />
        <StreamPanel v-if="stream.open" />
      </main>
    </div>
    <StatusBar />
    <DecoderDialog v-if="decode.editor" />
  </div>
</template>
