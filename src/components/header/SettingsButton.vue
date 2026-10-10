<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useSettingsStore } from '@/stores/settings'
import { useUpdateStore } from '@/stores/update'
import { commandLabel, isCommand } from '@/utils/keys'

/** The top bar's settings button, which opens the settings page (DESIGN.md §4, §11). */
const { t } = useI18n()
const settings = useSettingsStore()
const update = useUpdateStore()

// ⌘/Ctrl+, opens and closes the settings page from anywhere.
onKeyStroke(
  (event) => isCommand(event, ','),
  (event) => {
    event.preventDefault()
    settings.toggle()
  },
)

/** The button's name, which tells of a newer release. */
const label = computed(() =>
  update.available
    ? t('header.settingsUpdate', { version: update.available.version })
    : t('header.settings'),
)
</script>

<template>
  <button
    class="relative btn w-8 justify-center p-0"
    :aria-label="label"
    :title="t('header.withShortcut', { label, shortcut: commandLabel(',') })"
    aria-haspopup="dialog"
    :aria-expanded="settings.open"
    @click="settings.toggle()"
  >
    <AppIcon name="settings" />
    <i v-if="update.available" class="absolute top-1 right-1 size-1.5 rounded-full bg-ink" />
  </button>
</template>
