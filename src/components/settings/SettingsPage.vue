<script setup lang="ts">
import {
  ArrowRightToLine,
  Crosshair,
  FolderOpen,
  Gpu,
  Info,
  Languages,
  Monitor,
  Moon,
  MoveHorizontal,
  Palette,
  Puzzle,
  RefreshCw,
  Repeat,
  Rows3,
  RulerDimensionLine,
  ScrollText,
  SlidersHorizontal,
  Stethoscope,
  Sun,
  SunMoon,
  ZoomIn,
} from '@lucide/vue'
import { onKeyStroke } from '@vueuse/core'
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  shallowRef,
  useTemplateRef,
  type Component,
} from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import { useDecoderCopy } from '@/composables/useDecoderCopy'
import { getBackend } from '@/ipc/backend'
import { pickFile } from '@/ipc/dialogs'
import { toUiError, type UiError } from '@/ipc/errors'
import { LOG_LEVELS } from '@/ipc/logLevels'
import { vIndicator } from '@/motion/indicator'
import { useDecodeStore, type ModuleEntry } from '@/stores/decode'
import {
  DENSITIES,
  GRAPHICS,
  LOCALE_PREFERENCES,
  READOUT_PLACES,
  THEMES,
  usePreferencesStore,
  type ThemePreference,
} from '@/stores/preferences'
import { SETTINGS_SECTIONS, useSettingsStore, type SettingsSection } from '@/stores/settings'
import { useUpdateStore } from '@/stores/update'

import AboutSection from './AboutSection.vue'

/**
 * The settings page (DESIGN.md §11): centred over the blurred interface, with the sections in a
 * side bar and About at its foot.
 */
const { t } = useI18n()
const text = useBackendText()
const { typeName } = useDecoderCopy()
const preferences = usePreferencesStore()
const settings = useSettingsStore()
const update = useUpdateStore()
const decode = useDecodeStore()
const dialog = useTemplateRef<HTMLElement>('dialog')
const tablist = useTemplateRef<HTMLElement>('tablist')

/** Languages are named in their own language. */
const LANGUAGE_NAMES: Record<string, string> = { 'zh-CN': '简体中文', 'en-US': 'English' }

/** DSView's display options, how the wheel works, and whether to look for updates. */
const BEHAVIOURS = [
  'centerTrigger',
  'followLatest',
  'quickScroll',
  'wheelZoom',
  'keepStopped',
  'checkUpdates',
] as const

const SECTION_ICONS: Record<SettingsSection, Component> = {
  appearance: Palette,
  behaviour: SlidersHorizontal,
  decoders: Puzzle,
  diagnostics: Stethoscope,
  about: Info,
}

const THEME_ICONS: Record<ThemePreference, Component> = { system: Monitor, light: Sun, dark: Moon }

const BEHAVIOUR_ICONS: Record<(typeof BEHAVIOURS)[number], Component> = {
  centerTrigger: Crosshair,
  followLatest: ArrowRightToLine,
  quickScroll: MoveHorizontal,
  wheelZoom: ZoomIn,
  keepStopped: Repeat,
  checkUpdates: RefreshCw,
}

// Focus goes into the page without ringing a section, and back to what opened it when it closes.
const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null
onMounted(() => dialog.value?.focus())
onBeforeUnmount(() => opener?.focus({ preventScroll: true }))

onKeyStroke('Escape', () => settings.close())

/** The note About's tab carries when a newer release is out, as its tip and for screen readers. */
function updateNote(section: SettingsSection): string | undefined {
  const available = update.available
  return section === 'about' && available
    ? t('settings.updateAvailable', { version: available.version })
    : undefined
}

function selectedTab(): HTMLElement | null {
  return tablist.value?.querySelector<HTMLElement>('[aria-selected="true"]') ?? null
}

/** Up and down, Home and End move through the sections. */
async function onTabKey(event: KeyboardEvent): Promise<void> {
  const at = SETTINGS_SECTIONS.indexOf(settings.section)
  const last = SETTINGS_SECTIONS.length - 1
  const next: Partial<Record<string, number>> = {
    ArrowDown: Math.min(last, at + 1),
    ArrowUp: Math.max(0, at - 1),
    Home: 0,
    End: last,
  }
  const index = next[event.key]
  if (index === undefined) return
  event.preventDefault()
  settings.section = SETTINGS_SECTIONS[index] as SettingsSection
  await nextTick()
  selectedTab()?.focus()
}

/** Where the log file is; unknown in the browser preview, which has none. */
const logFile = shallowRef<string | null>(null)
const revealError = shallowRef<UiError | null>(null)
onMounted(async () => {
  try {
    logFile.value = await (await getBackend()).logFile()
  } catch (error) {
    console.error('log file location failed', error)
  }
})

async function revealLog(): Promise<void> {
  revealError.value = null
  try {
    await (await getBackend()).revealLog()
  } catch (error) {
    revealError.value = toUiError(error)
  }
}

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path

/** A row of the Decoders section: a module loaded here, or one a capture file brought. */
interface ModuleRow {
  key: string
  entry: ModuleEntry
  carried: boolean
}

/** The modules loaded here, then those capture files brought. */
const moduleRows = computed<ModuleRow[]>(() => [
  ...decode.modules.map((entry) => ({ key: entry.path, entry, carried: false })),
  ...decode.carried.map((info) => ({
    key: `${info.path}#${info.name}`,
    entry: { path: info.path, info, error: null, loading: false },
    carried: true,
  })),
])

/** Asks for a module file and loads it. */
async function loadModule(): Promise<void> {
  const path = await pickFile([{ name: t('settings.decoders.filter'), extensions: ['json'] }])
  if (path) await decode.loadModule(path)
}

/** Lets a module go, whichever way it came. */
function removeModule(row: ModuleRow): void {
  if (row.carried) void decode.unloadCarried(row.entry.info?.name ?? '')
  else void decode.unloadModule(row.entry.path)
}
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center modal-veil"
    @click.self="settings.close()"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="t('settings.title')"
      tabindex="-1"
      class="flex h-[min(560px,calc(100vh-48px))] w-[min(760px,calc(100vw-48px))] flex-col overflow-hidden rounded-6 bg-raise text-small outline-none"
    >
      <header
        class="flex h-10 flex-none items-center justify-between border-b border-line pr-2 pl-4"
      >
        <h2 class="font-medium">{{ t('settings.title') }}</h2>
        <button
          class="btn-icon"
          :aria-label="t('settings.close')"
          :title="t('settings.close')"
          @click="settings.close()"
        >
          <AppIcon name="close" />
        </button>
      </header>
      <div class="flex min-h-0 flex-1">
        <nav class="flex w-44 flex-none flex-col border-r border-line bg-chrome p-2">
          <!-- About sits at the foot of the side bar, with a dot when a newer release is out. -->
          <div
            ref="tablist"
            v-indicator
            role="tablist"
            aria-orientation="vertical"
            :aria-label="t('settings.title')"
            class="flex min-h-0 flex-1 flex-col gap-0.5"
            @keydown="onTabKey"
          >
            <button
              v-for="one in SETTINGS_SECTIONS"
              :id="`settings-tab-${one}`"
              :key="one"
              role="tab"
              :aria-selected="settings.section === one"
              :aria-controls="`settings-panel-${one}`"
              :tabindex="settings.section === one ? 0 : -1"
              :title="updateNote(one)"
              class="flex h-[30px] items-center gap-2 rounded-4 px-2.5 text-left text-ink-2 hover:text-ink focus-visible:outline-offset-[-2px] aria-selected:font-medium aria-selected:text-ink"
              :class="one === 'about' && 'mt-auto'"
              @click="settings.section = one"
            >
              <component :is="SECTION_ICONS[one]" />
              {{ t(`settings.sections.${one}`) }}
              <template v-if="updateNote(one)">
                <i class="ml-auto size-1.5 flex-none rounded-full bg-ink" aria-hidden="true" />
                <span class="sr-only">{{ updateNote(one) }}</span>
              </template>
            </button>
          </div>
        </nav>

        <!-- Another section is other content, which rises in. -->
        <section
          :id="`settings-panel-${settings.section}`"
          :key="settings.section"
          role="tabpanel"
          :aria-labelledby="`settings-tab-${settings.section}`"
          class="rise-in min-w-0 flex-1 overflow-y-auto px-6 py-5"
        >
          <div
            v-if="settings.section === 'appearance'"
            class="grid grid-cols-[minmax(7rem,max-content)_minmax(0,1fr)] items-start gap-x-4 gap-y-5"
          >
            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <SunMoon class="text-ink-3" />{{ t('settings.theme.label') }}
            </span>
            <div
              v-indicator
              role="group"
              :aria-label="t('settings.theme.label')"
              class="flex gap-0.5 justify-self-start rounded-6 bg-hover p-0.5"
            >
              <button
                v-for="theme in THEMES"
                :key="theme"
                class="inline-flex seg items-center gap-1.5"
                :aria-pressed="preferences.theme === theme"
                @click="preferences.theme = theme"
              >
                <component :is="THEME_ICONS[theme]" :size="14" />
                {{ t(`settings.theme.${theme}`) }}
              </button>
            </div>

            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <Rows3 class="text-ink-3" />{{ t('settings.density.label') }}
            </span>
            <div
              v-indicator
              role="group"
              :aria-label="t('settings.density.label')"
              class="flex gap-0.5 justify-self-start rounded-6 bg-hover p-0.5"
            >
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

            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <RulerDimensionLine class="text-ink-3" />{{ t('settings.readouts.label') }}
            </span>
            <div class="flex flex-col items-start gap-1.5">
              <div
                v-indicator
                role="group"
                :aria-label="t('settings.readouts.label')"
                class="flex gap-0.5 rounded-6 bg-hover p-0.5"
              >
                <button
                  v-for="place in READOUT_PLACES"
                  :key="place"
                  class="seg"
                  :aria-pressed="preferences.readoutPlace === place"
                  @click="preferences.readoutPlace = place"
                >
                  {{ t(`settings.readouts.${place}`) }}
                </button>
              </div>
              <span class="text-caption text-ink-3">{{ t('settings.readouts.note') }}</span>
            </div>

            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <Gpu class="text-ink-3" />{{ t('settings.graphics.label') }}
            </span>
            <div class="flex flex-col items-start gap-1.5">
              <div
                v-indicator
                role="group"
                :aria-label="t('settings.graphics.label')"
                class="flex gap-0.5 rounded-6 bg-hover p-0.5"
              >
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

            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <Languages class="text-ink-3" />{{ t('settings.language.label') }}
            </span>
            <div
              v-indicator
              role="group"
              :aria-label="t('settings.language.label')"
              class="flex gap-0.5 justify-self-start rounded-6 bg-hover p-0.5"
            >
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

          <div v-else-if="settings.section === 'behaviour'" class="flex flex-col gap-3">
            <label
              v-for="behaviour in BEHAVIOURS"
              :key="behaviour"
              class="flex items-center gap-2.5"
            >
              <input v-model="preferences[behaviour]" type="checkbox" class="size-4 accent-ink" />
              <component :is="BEHAVIOUR_ICONS[behaviour]" class="text-ink-3" />
              <span>{{ t(`settings.behaviour.${behaviour}`) }}</span>
            </label>
          </div>

          <div v-else-if="settings.section === 'decoders'" class="flex flex-col gap-3">
            <p class="text-ink-3">{{ t('settings.decoders.intro') }}</p>
            <ul
              v-if="moduleRows.length > 0"
              class="flex flex-col gap-1.5"
              :aria-label="t('settings.sections.decoders')"
            >
              <li
                v-for="row in moduleRows"
                :key="row.key"
                class="flex items-start gap-3 rounded-6 bg-control px-3 py-2"
              >
                <Puzzle class="mt-0.5 flex-none text-ink-3" />
                <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span class="font-medium">
                    {{ row.entry.info ? typeName(row.entry.info.name) : fileName(row.entry.path) }}
                  </span>
                  <span class="truncate text-caption text-ink-3" :title="row.entry.path">
                    {{ row.entry.path }}
                  </span>
                  <span v-if="row.carried" class="text-caption text-ink-3">
                    {{ t('settings.decoders.carried', { file: fileName(row.entry.path) }) }}
                  </span>
                  <span v-if="row.entry.info?.shadows" class="text-caption text-ink-3">
                    {{ t('settings.decoders.replaces', { name: typeName(row.entry.info.name) }) }}
                  </span>
                  <span v-if="row.entry.error" role="alert" class="text-caption text-alert">
                    {{ text.error(row.entry.error) }}
                  </span>
                  <span v-else-if="row.entry.loading" class="text-caption text-ink-3">
                    {{ t('settings.decoders.loading') }}
                  </span>
                </div>
                <button class="btn-ghost h-[22px] flex-none text-alert" @click="removeModule(row)">
                  {{ t('settings.decoders.remove') }}
                </button>
              </li>
            </ul>
            <p v-else class="text-ink-3">{{ t('settings.decoders.empty') }}</p>
            <div>
              <button class="btn-ghost h-[22px]" @click="loadModule">
                <FolderOpen :size="14" />
                {{ t('settings.decoders.load') }}
              </button>
            </div>
          </div>

          <div
            v-else-if="settings.section === 'diagnostics'"
            class="grid grid-cols-[minmax(7rem,max-content)_minmax(0,1fr)] items-start gap-x-4 gap-y-5"
          >
            <span class="flex h-[26px] items-center gap-2 text-ink-2">
              <ScrollText class="text-ink-3" />{{ t('settings.diagnostics.level') }}
            </span>
            <div
              v-indicator
              role="group"
              :aria-label="t('settings.diagnostics.level')"
              class="flex gap-0.5 justify-self-start rounded-6 bg-hover p-0.5"
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

            <div class="col-start-2 flex flex-col items-start gap-2">
              <div class="flex items-center gap-3">
                <label class="flex items-center gap-2.5">
                  <input
                    v-model="preferences.logToFile"
                    type="checkbox"
                    class="size-4 accent-ink"
                  />
                  <span>{{ t('settings.diagnostics.file') }}</span>
                </label>
                <button
                  v-if="logFile"
                  class="btn-ghost h-[22px]"
                  :title="logFile"
                  @click="revealLog"
                >
                  <FolderOpen :size="14" />
                  {{ t('settings.diagnostics.reveal') }}
                </button>
              </div>
              <span v-if="revealError" role="alert" class="rise-in text-caption text-alert">{{
                text.error(revealError)
              }}</span>
            </div>
          </div>

          <AboutSection v-else />
        </section>
      </div>
    </div>
  </div>
</template>
