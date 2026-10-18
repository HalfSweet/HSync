<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import ExportDialog from '@/components/export/ExportDialog.vue'
import { useFileActions } from '@/composables/useFileActions'
import { useShortcutLabel, useShortcuts } from '@/composables/useShortcuts'
import { useCaptureStore } from '@/stores/capture'

/** The export button of the top bar (DESIGN.md §4): saving the capture, and exporting it. */
const { t } = useI18n()
const capture = useCaptureStore()
const files = useFileActions()
const open = ref(false)
const showDialog = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

function save(): void {
  open.value = false
  void files.save()
}

function exportAs(): void {
  open.value = false
  showDialog.value = true
}

/** Saving and exporting need a capture, and no capture or file job under way. */
const canSave = computed(() => capture.summary !== null && !capture.busy && capture.job === null)

// The export shortcut opens the export dialog, as the menu item does.
useShortcuts({ export: () => (canSave.value ? exportAs() : false) })
const shortcut = useShortcutLabel()

function saveSession(): void {
  open.value = false
  void files.saveSession()
}
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="btn w-8 justify-center p-0"
      :aria-label="t('header.export')"
      :title="t('header.export')"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="export" />
    </button>
    <Transition name="pop">
      <div
        v-if="open"
        role="menu"
        :aria-label="t('header.export')"
        class="absolute top-full right-0 z-20 mt-1 flex w-56 origin-top-right flex-col rounded-6 bg-menu p-1 text-small"
      >
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
          :disabled="!canSave"
          @click="save"
        >
          <span class="flex-1 text-left">{{ t('file.save') }}</span>
          <kbd v-if="shortcut('save')" class="font-sans text-caption text-ink-3">{{
            shortcut('save')
          }}</kbd>
        </button>
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
          :disabled="!canSave"
          @click="exportAs"
        >
          <span class="flex-1 text-left">{{ t('file.exportItem') }}</span>
          <kbd v-if="shortcut('export')" class="font-sans text-caption text-ink-3">{{
            shortcut('export')
          }}</kbd>
        </button>
        <div class="my-1 border-t border-line-2" />
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
          :disabled="capture.busy || capture.job !== null"
          @click="saveSession"
        >
          <span class="flex-1 text-left">{{ t('file.saveSession') }}</span>
        </button>
      </div>
    </Transition>
    <Transition name="dialog">
      <ExportDialog v-if="showDialog" @close="showDialog = false" />
    </Transition>
  </div>
</template>
