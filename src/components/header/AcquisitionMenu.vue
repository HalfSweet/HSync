<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, nextTick, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { closestDuration, durationChoices, samplesFor } from '@/acquisition/durations'
import AppIcon from '@/components/common/AppIcon.vue'
import { getBackend } from '@/ipc/backend'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'
import { formatBytes, formatCount, formatSamplerate, formatTimeShort } from '@/utils/units'

/**
 * `rate │ duration │ N channels` and its menu (DESIGN.md §9.4): sample rates, the durations
 * DSView offers for the rate, and which channels to capture.
 */
const { t, locale } = useI18n()
const device = useDeviceStore()
const capture = useCaptureStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

// Opening shows the current rate and duration, which may sit far down their lists.
watch(open, async (shown) => {
  if (!shown) return
  await nextTick()
  for (const item of root.value?.querySelectorAll<HTMLElement>(
    '[data-list] [aria-pressed="true"]',
  ) ?? [])
    item.scrollIntoView({ block: 'center' })
})

const rateOption = computed(() => device.option('samplerate'))
const limitOption = computed(() => device.option('sampleLimit'))
const rates = computed(() =>
  rateOption.value?.kind.type === 'integers' ? rateOption.value.kind.values : [],
)
const maxSamples = computed(() =>
  limitOption.value?.kind.type === 'range' ? limitOption.value.kind.max : 0,
)

/** What the next capture takes: the device's settings, or the open capture's without one. */
const rate = computed(() => (device.state ? device.samplerate : capture.samplerate))
const seconds = computed(() =>
  device.state ? (rate.value > 0 ? device.sampleLimit / rate.value : 0) : capture.duration,
)
const channelCount = computed(() =>
  device.state ? device.enabledChannels.length : (capture.summary?.channels.length ?? 0),
)
const durations = computed(() => durationChoices(rate.value, maxSamples.value))
const currentDuration = computed(() => closestDuration(durations.value, seconds.value))

/** Durations beyond the device's memory need RLE compression, which DSView marks. */
const needsRle = (duration: number) => {
  const depth = device.state?.hardwareDepth
  return depth != null && samplesFor(duration, rate.value) > depth
}

// A capture is kept bit-packed in memory; past half of the machine's, warn (DESIGN.md §5.2).
const memory = shallowRef<number | null>(null)
watch(open, async (shown) => {
  if (!shown || memory.value !== null) return
  try {
    memory.value = await (await getBackend()).systemMemory()
  } catch (error) {
    console.error('memory size failed', error)
  }
})
const needed = computed(() => (device.sampleLimit * channelCount.value) / 8)
const tooLarge = computed(() => memory.value !== null && needed.value > memory.value / 2)
const locked = computed(() => capture.busy || !device.state)

const label = computed(() =>
  t('header.acquisition.aria', {
    rate: formatSamplerate(rate.value),
    duration: formatTimeShort(seconds.value),
    channels: channelCount.value,
  }),
)

async function pickRate(next: number): Promise<void> {
  const keep = seconds.value
  if (!(await device.setOption('samplerate', { type: 'integer', value: next }))) return
  // Keep the capture about as long, as DSView does.
  const choice = closestDuration(durationChoices(next, maxSamples.value), keep)
  if (choice !== undefined) await pickDuration(choice)
}

async function pickDuration(duration: number): Promise<void> {
  await device.setOption('sampleLimit', {
    type: 'integer',
    value: Math.min(samplesFor(duration, rate.value), maxSamples.value),
  })
}

function toggleChannel(channel: number): void {
  void device.setChannel(channel, !device.enabledChannels.includes(channel))
}
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="btn gap-3"
      :aria-label="label"
      aria-haspopup="dialog"
      :aria-expanded="open"
      @click="open = !open"
    >
      <span class="tabular-nums">{{ formatSamplerate(rate) }}</span>
      <i class="vr" />
      <span class="tabular-nums">{{ formatTimeShort(seconds) }}</span>
      <i class="vr" />
      <span>{{ t('header.acquisition.channels', { count: channelCount }) }}</span>
      <AppIcon name="chevron-down" />
    </button>
    <div
      v-if="open"
      role="dialog"
      :aria-label="label"
      class="absolute top-full left-0 z-20 mt-1 flex w-[360px] flex-col gap-3 rounded-6 border border-ink bg-raise p-3 text-small"
    >
      <p v-if="!device.state" class="text-ink-3">{{ t('header.acquisition.noDevice') }}</p>
      <template v-else>
        <div class="grid grid-cols-2 gap-3">
          <div class="flex min-w-0 flex-col gap-1">
            <span class="text-caption text-ink-3">{{ t('header.acquisition.samplerate') }}</span>
            <div data-list class="flex max-h-56 flex-col overflow-y-auto">
              <button
                v-for="value in rates"
                :key="value"
                class="flex h-6 shrink-0 items-center justify-between rounded-4 px-2 tabular-nums hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
                :class="value === rate && 'font-medium'"
                :aria-pressed="value === rate"
                :disabled="locked || rateOption?.enabled === false"
                @click="pickRate(value)"
              >
                <span>{{ formatSamplerate(value) }}</span>
                <AppIcon v-if="value === rate" name="check" />
              </button>
            </div>
          </div>
          <div class="flex min-w-0 flex-col gap-1">
            <span class="text-caption text-ink-3">{{ t('header.acquisition.duration') }}</span>
            <div data-list class="flex max-h-56 flex-col overflow-y-auto">
              <button
                v-for="duration in durations"
                :key="duration"
                class="flex h-6 shrink-0 items-center justify-between rounded-4 px-2 tabular-nums hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
                :class="duration === currentDuration && 'font-medium'"
                :aria-pressed="duration === currentDuration"
                :disabled="locked"
                @click="pickDuration(duration)"
              >
                <span class="inline-flex items-baseline gap-1.5"
                  >{{ formatTimeShort(duration) }}
                  <span v-if="needsRle(duration)" class="text-caption text-ink-3">{{
                    t('header.acquisition.rle')
                  }}</span></span
                >
                <AppIcon v-if="duration === currentDuration" name="check" />
              </button>
            </div>
          </div>
        </div>
        <p class="text-caption text-ink-3 tabular-nums">
          {{
            t('header.acquisition.samples', {
              count: formatCount(device.sampleLimit, locale),
            })
          }}
        </p>
        <p v-if="tooLarge" role="alert" class="text-caption text-alert tabular-nums">
          {{ t('header.acquisition.tooLarge', { size: formatBytes(needed) }) }}
        </p>
        <div class="flex flex-col gap-1.5">
          <span class="text-caption text-ink-3">{{ t('header.acquisition.channelsTitle') }}</span>
          <div class="grid grid-cols-8 gap-1">
            <button
              v-for="channel in device.state.channelCount"
              :key="channel"
              class="h-6 rounded-4 border text-caption tabular-nums disabled:opacity-55"
              :class="
                device.enabledChannels.includes(channel - 1)
                  ? 'border-ink-3 bg-sel font-medium text-ink'
                  : 'border-line text-ink-3 hover:border-ink-3'
              "
              :aria-pressed="device.enabledChannels.includes(channel - 1)"
              :disabled="locked"
              @click="toggleChannel(channel - 1)"
            >
              {{ channel - 1 }}
            </button>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>
