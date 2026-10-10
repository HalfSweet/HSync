<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useFileActions } from '@/composables/useFileActions'
import { useCaptureStore } from '@/stores/capture'
import { commandLabel } from '@/utils/keys'

/** The export button of the top bar (DESIGN.md §9.4): saving the capture as a `.dsl` file. */
const { t } = useI18n()
const capture = useCaptureStore()
const files = useFileActions()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

function save(): void {
  open.value = false
  void files.save()
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
    <div
      v-if="open"
      role="menu"
      :aria-label="t('header.export')"
      class="absolute top-full right-0 z-20 mt-1 flex w-56 flex-col rounded-6 border border-ink bg-raise p-1 text-small"
    >
      <button
        role="menuitem"
        class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
        :disabled="!capture.summary || capture.busy"
        @click="save"
      >
        <span class="flex-1 text-left">{{ t('file.save') }}</span>
        <kbd class="font-sans text-caption text-ink-3">{{ commandLabel('s') }}</kbd>
      </button>
    </div>
  </div>
</template>
