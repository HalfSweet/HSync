<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useFileActions } from '@/composables/useFileActions'
import type { DeviceSummary } from '@/ipc/bindings/DeviceSummary'
import { holdInPlace } from '@/motion/list'
import { useCaptureStore } from '@/stores/capture'
import { DEMO_DRIVER, useDeviceStore } from '@/stores/device'
import { commandLabel } from '@/utils/keys'

import DeviceOptionsDialog from './DeviceOptionsDialog.vue'

/** The device picker of the top bar (DESIGN.md §9.4). */
const { t } = useI18n()
const device = useDeviceStore()
const capture = useCaptureStore()
const open = ref(false)
const showOptions = ref(false)
const root = useTemplateRef<HTMLElement>('root')
const files = useFileActions()

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

function nameOf(entry: { driver: string; model: string }): string {
  return entry.driver === DEMO_DRIVER ? t('device.demo') : entry.model
}

const buttonLabel = computed(() => {
  if (device.opening) return t('header.device.opening')
  if (device.state) return nameOf(device.state.info)
  const source = capture.summary?.source
  if (source?.kind === 'file') return source.path.split(/[\\/]/).pop() ?? source.path
  return t('header.device.select')
})

function note(entry: DeviceSummary): string | null {
  switch (entry.status) {
    case 'needsFirmware':
      return t('header.device.needsFirmware')
    case 'speedTooLow':
      return t('header.device.speedTooLow')
    case 'unsupported':
      return t('header.device.unsupported')
    default:
      return null
  }
}

const selectable = (entry: DeviceSummary) =>
  entry.status === 'ready' || entry.status === 'needsFirmware'

async function toggle(): Promise<void> {
  open.value = !open.value
  if (open.value) await device.refresh()
}

async function pick(entry: DeviceSummary): Promise<void> {
  if (!selectable(entry) || capture.running) return
  open.value = false
  if (entry.id !== device.state?.info.id) await device.open(entry.id)
}

function openOptions(): void {
  open.value = false
  showOptions.value = true
}

function openFile(): void {
  open.value = false
  void files.open()
}

function loadSession(): void {
  open.value = false
  void files.loadSession()
}
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="btn font-medium"
      aria-haspopup="menu"
      :aria-expanded="open"
      :disabled="device.opening !== null"
      @click="toggle"
    >
      <span>{{ buttonLabel }}</span>
      <AppIcon name="chevron-down" />
    </button>
    <Transition name="pop">
      <div
        v-if="open"
        role="menu"
        :aria-label="t('header.device.listTitle')"
        class="absolute top-full left-0 z-20 mt-1 flex w-72 origin-top-left flex-col rounded-6 border border-ink bg-raise p-1 text-small"
      >
        <div class="flex h-6 items-center justify-between px-2 text-caption text-ink-3">
          <span>{{ t('header.device.listTitle') }}</span>
          <button class="hover:text-ink" @click="device.refresh()">
            {{ t('header.device.rescan') }}
          </button>
        </div>
        <!-- Devices come and go as they are plugged in, pulled out or found by a rescan. -->
        <TransitionGroup
          tag="div"
          name="item"
          class="relative flex flex-col"
          @before-leave="holdInPlace"
        >
          <p v-if="device.devices.length === 0" key="no-devices" class="px-2 py-1.5 text-ink-3">
            {{ t('header.device.none') }}
          </p>
          <button
            v-for="entry in device.devices"
            :key="entry.id"
            role="menuitemradio"
            :aria-checked="entry.id === device.state?.info.id"
            :aria-disabled="!selectable(entry) || capture.running"
            class="flex min-h-[30px] w-full items-center gap-2 rounded-4 px-2 py-1 text-left hover:bg-hover aria-disabled:cursor-default aria-disabled:opacity-55 aria-disabled:hover:bg-transparent"
            @click="pick(entry)"
          >
            <span class="flex min-w-0 flex-1 flex-col">
              <span :class="entry.id === device.state?.info.id && 'font-medium'">{{
                nameOf(entry)
              }}</span>
              <span v-if="note(entry)" class="text-caption text-ink-3">{{ note(entry) }}</span>
            </span>
            <AppIcon v-if="entry.id === device.state?.info.id" name="check" />
          </button>
        </TransitionGroup>
        <div class="my-1 border-t border-line" />
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55"
          :disabled="capture.busy"
          @click="openFile"
        >
          <span class="flex-1 text-left">{{ t('file.open') }}</span>
          <kbd class="font-sans text-caption text-ink-3">{{ commandLabel('o') }}</kbd>
        </button>
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55"
          :disabled="capture.busy"
          @click="loadSession"
        >
          <span class="flex-1 text-left">{{ t('file.loadSession') }}</span>
        </button>
        <button
          role="menuitem"
          class="flex h-[30px] w-full items-center gap-2 rounded-4 px-2 hover:bg-hover disabled:opacity-55"
          :disabled="!device.state"
          @click="openOptions"
        >
          {{ t('header.device.options') }}
        </button>
      </div>
    </Transition>
    <Transition name="dialog">
      <DeviceOptionsDialog v-if="showOptions" @close="showOptions = false" />
    </Transition>
  </div>
</template>
