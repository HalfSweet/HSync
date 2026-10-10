<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AboutDialog from '@/components/about/AboutDialog.vue'
import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import { getBackend } from '@/ipc/backend'
import { toUiError, type UiError } from '@/ipc/errors'
import { LOG_LEVELS } from '@/ipc/logLevels'
import {
  DENSITIES,
  GRAPHICS,
  LOCALE_PREFERENCES,
  THEMES,
  usePreferencesStore,
} from '@/stores/preferences'
import { useUpdateStore } from '@/stores/update'

const { t } = useI18n()
const text = useBackendText()
const preferences = usePreferencesStore()
const update = useUpdateStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

/** Languages are named in their own language. */
const LANGUAGE_NAMES: Record<string, string> = { 'zh-CN': '简体中文', 'en-US': 'English' }

/** DSView's display options, and whether to look for updates. */
const BEHAVIOURS = [
  'centerTrigger',
  'followLatest',
  'quickScroll',
  'keepStopped',
  'checkUpdates',
] as const

/** The settings button's name, which tells of a newer release. */
const buttonLabel = computed(() =>
  update.available
    ? t('header.settingsUpdate', { version: update.available.version })
    : t('header.settings'),
)

/** Where the log file is; unknown in the browser preview, which has none. */
const logFile = shallowRef<string | null>(null)
const revealError = shallowRef<UiError | null>(null)
watch(open, async (isOpen) => {
  revealError.value = null
  if (!isOpen || logFile.value) return
  try {
    logFile.value = await (await getBackend()).logFile()
  } catch (error) {
    console.error('log file location failed', error)
  }
})

const about = shallowRef(false)

function showAbout(): void {
  open.value = false
  about.value = true
}

async function revealLog(): Promise<void> {
  revealError.value = null
  try {
    await (await getBackend()).revealLog()
  } catch (error) {
    revealError.value = toUiError(error)
  }
}
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="relative btn w-8 justify-center p-0"
      :aria-label="buttonLabel"
      :title="buttonLabel"
      aria-haspopup="dialog"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="settings" />
      <i v-if="update.available" class="absolute top-1 right-1 size-1.5 rounded-full bg-ink" />
    </button>
    <Transition name="pop">
      <div
        v-if="open"
        role="dialog"
        :aria-label="t('settings.title')"
        class="absolute top-full right-0 z-20 mt-1 flex max-h-[calc(100vh-64px)] w-72 origin-top-right flex-col gap-3 overflow-y-auto rounded-6 border border-ink bg-raise p-3 text-small"
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
          <span class="text-caption text-ink-3">{{ t('settings.graphics.label') }}</span>
          <div role="group" class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5">
            <button
              v-for="option in GRAPHICS"
              :key="option"
              class="seg"
              :aria-pressed="preferences.graphics === option"
              @click="preferences.graphics = option"
            >
              {{ t(`settings.graphics.${option}`) }}
            </button>
          </div>
          <span v-if="preferences.activeRenderer" class="text-caption text-ink-3">{{
            t(`settings.graphics.active.${preferences.activeRenderer}`)
          }}</span>
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
        <div class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('settings.behaviour.label') }}</span>
          <label v-for="behaviour in BEHAVIOURS" :key="behaviour" class="flex items-center gap-2">
            <input v-model="preferences[behaviour]" type="checkbox" class="size-4 accent-ink" />
            <span>{{ t(`settings.behaviour.${behaviour}`) }}</span>
          </label>
        </div>
        <div class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('settings.diagnostics.label') }}</span>
          <div
            role="group"
            :aria-label="t('settings.diagnostics.level')"
            class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5"
          >
            <button
              v-for="level in LOG_LEVELS"
              :key="level"
              class="seg"
              :aria-pressed="preferences.logLevel === level"
              @click="preferences.logLevel = level"
            >
              {{ t(`settings.diagnostics.levels.${level}`) }}
            </button>
          </div>
          <div class="flex items-center gap-2">
            <label class="flex flex-1 items-center gap-2">
              <input v-model="preferences.logToFile" type="checkbox" class="size-4 accent-ink" />
              <span>{{ t('settings.diagnostics.file') }}</span>
            </label>
            <button v-if="logFile" class="btn-ghost h-[22px]" :title="logFile" @click="revealLog">
              {{ t('settings.diagnostics.reveal') }}
            </button>
          </div>
          <span v-if="revealError" role="alert" class="text-caption text-alert">{{
            text.error(revealError)
          }}</span>
        </div>
        <div class="flex items-center gap-2">
          <button class="btn-ghost h-[26px]" @click="showAbout">
            {{ t('settings.about') }}
          </button>
          <span v-if="update.available" class="text-caption text-ink-2 tabular-nums">{{
            t('settings.updateAvailable', { version: update.available.version })
          }}</span>
        </div>
      </div>
    </Transition>
    <Transition name="dialog">
      <AboutDialog v-if="about" @close="about = false" />
    </Transition>
  </div>
</template>
