<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, shallowRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import { getBackend } from '@/ipc/backend'
import { useUpdateStore } from '@/stores/update'

/**
 * About HSync (DESIGN.md §9.11): the version and updates to it, the license and where the parts
 * come from.
 */
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const text = useBackendText()
const update = useUpdateStore()
const dialog = useTemplateRef<HTMLElement>('dialog')
const version = shallowRef<string | null>(null)

/** How far the download is, when the server told its size. */
const percent = computed(() => {
  const progress = update.progress
  return progress?.total ? Math.min(100, Math.floor((progress.done / progress.total) * 100)) : null
})

onKeyStroke('Escape', () => emit('close'))
onMounted(async () => {
  dialog.value?.focus()
  try {
    version.value = await (await getBackend()).appVersion()
  } catch (error) {
    console.error('app version failed', error)
  }
})
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center bg-veil"
    @click.self="emit('close')"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="t('about.title')"
      tabindex="-1"
      class="flex w-[min(420px,calc(100vw-32px))] flex-col rounded-6 border border-ink bg-raise text-small outline-none"
    >
      <header
        class="flex h-10 flex-none items-center justify-between border-b border-line pr-2 pl-4"
      >
        <h2 class="font-medium">{{ t('about.title') }}</h2>
        <button class="btn-icon" :aria-label="t('about.close')" @click="emit('close')">
          <AppIcon name="close" />
        </button>
      </header>
      <div class="flex flex-col gap-3 p-4">
        <div class="flex flex-col gap-0.5">
          <span class="text-base font-medium">HSync</span>
          <span class="text-ink-2">{{ t('about.tagline') }}</span>
          <span v-if="version" class="text-caption text-ink-3 tabular-nums">{{
            t('about.version', { version })
          }}</span>
        </div>
        <div class="flex flex-col gap-1">
          <div v-if="update.available" class="flex min-h-[26px] items-center gap-2">
            <span class="tabular-nums">{{
              t('about.update.available', { version: update.available.version })
            }}</span>
            <span v-if="update.state === 'downloading'" class="text-ink-3 tabular-nums">{{
              percent === null
                ? t('about.update.downloadingUnsized')
                : t('about.update.downloading', { percent })
            }}</span>
            <span v-else-if="update.state === 'restarting'" class="text-ink-3">{{
              t('about.update.restarting')
            }}</span>
            <button v-else class="btn-ghost h-[26px]" @click="update.install()">
              {{ t('about.update.install') }}
            </button>
          </div>
          <div v-else class="flex min-h-[26px] items-center gap-2">
            <button
              class="btn-ghost h-[26px]"
              :disabled="update.state === 'checking'"
              @click="update.check()"
            >
              {{ t('about.update.check') }}
            </button>
            <span v-if="update.state === 'checking'" role="status" class="text-ink-3">{{
              t('about.update.checking')
            }}</span>
            <span v-else-if="update.state === 'latest'" role="status" class="text-ink-3">{{
              t('about.update.latest')
            }}</span>
          </div>
          <span v-if="update.error" role="alert" class="text-caption text-alert">{{
            text.error(update.error)
          }}</span>
        </div>
        <p class="text-ink-2">{{ t('about.license') }}</p>
        <p class="text-ink-2">{{ t('about.credits') }}</p>
      </div>
    </div>
  </div>
</template>
