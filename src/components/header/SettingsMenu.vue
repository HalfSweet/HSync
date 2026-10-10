<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { DENSITIES, LOCALE_PREFERENCES, THEMES, usePreferencesStore } from '@/stores/preferences'

const { t } = useI18n()
const preferences = usePreferencesStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

/** Languages are named in their own language. */
const LANGUAGE_NAMES: Record<string, string> = { 'zh-CN': '简体中文', 'en-US': 'English' }
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="btn w-8 justify-center p-0"
      :aria-label="t('header.settings')"
      :title="t('header.settings')"
      aria-haspopup="dialog"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="settings" />
    </button>
    <div
      v-if="open"
      role="dialog"
      :aria-label="t('settings.title')"
      class="absolute top-full right-0 z-20 mt-1 flex w-64 flex-col gap-3 rounded-6 border border-ink bg-raise p-3 text-small"
    >
      <div class="flex flex-col gap-1.5">
        <span class="text-caption text-ink-3">{{ t('settings.theme.label') }}</span>
        <div role="group" class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5">
          <button
            v-for="theme in THEMES"
            :key="theme"
            class="seg"
            :aria-pressed="preferences.theme === theme"
            @click="preferences.theme = theme"
          >
            {{ t(`settings.theme.${theme}`) }}
          </button>
        </div>
      </div>
      <div class="flex flex-col gap-1.5">
        <span class="text-caption text-ink-3">{{ t('settings.density.label') }}</span>
        <div role="group" class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5">
          <button
            v-for="density in DENSITIES"
            :key="density"
            class="seg"
            :aria-pressed="preferences.density === density"
            @click="preferences.density = density"
          >
            {{ t(`settings.density.${density}`) }}
          </button>
        </div>
      </div>
      <div class="flex flex-col gap-1.5">
        <span class="text-caption text-ink-3">{{ t('settings.language.label') }}</span>
        <div role="group" class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5">
          <button
            v-for="locale in LOCALE_PREFERENCES"
            :key="locale"
            class="seg"
            :aria-pressed="preferences.locale === locale"
            @click="preferences.locale = locale"
          >
            {{ locale === 'system' ? t('settings.language.system') : LANGUAGE_NAMES[locale] }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
