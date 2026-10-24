<script setup lang="ts">
import { PanelRight } from '@lucide/vue'
import { useI18n } from 'vue-i18n'

import { useShortcutTip, useShortcuts } from '@/composables/useShortcuts'
import { useStreamStore } from '@/stores/stream'

/** The top bar's data stream button, above the panel it opens and closes (DESIGN.md §4, §9). */
const { t } = useI18n()
const stream = useStreamStore()

// The data stream shortcut, unbound until the user gives it one, opens and closes the panel.
useShortcuts({ dataStream: () => stream.toggle() })
const tip = useShortcutTip()
</script>

<template>
  <button
    class="btn w-8 justify-center p-0 aria-pressed:bg-control-on max-[640px]:hidden"
    :aria-label="t('stream.title')"
    :title="tip(t('stream.title'), 'dataStream')"
    :aria-pressed="stream.shown"
    @click="stream.toggle()"
  >
    <PanelRight />
  </button>
</template>
