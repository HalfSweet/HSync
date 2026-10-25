<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, nextTick, onMounted, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import {
  MEMORY_SHARE,
  closestDuration,
  durationChoices,
  memoryLimit,
  samplesFor,
} from '@/acquisition/durations'
import {
  fits,
  keptChannels,
  leadingChannels,
  presetCounts,
  tierForChannels,
  tierForRate,
  tierRates,
} from '@/acquisition/tiers'
import AppIcon from '@/components/common/AppIcon.vue'
import { useBackendText } from '@/composables/useBackendText'
import { getBackend } from '@/ipc/backend'
import type { RateTier } from '@/ipc/bindings/RateTier'
import { vIndicator } from '@/motion/indicator'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'
import { formatBytes, formatSamplerate, formatTimeShort } from '@/utils/units'

import DeviceOptions from './DeviceOptions.vue'

/**
 * `rate │ duration │ N channels` and its menu (DESIGN.md §4), in the order the settings bound
 * one another: the operation mode; the channels, with the rate each tier of them reaches; the
 * sample rates and the durations DSView offers for the rate; then the device's other options.
 */
const { t } = useI18n()
const text = useBackendText()
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

// A capture is kept bit-packed in memory, which bounds Stream captures and warns past a share.
const memory = shallowRef<number | null>(null)
onMounted(async () => {
  try {
    memory.value = await (await getBackend()).systemMemory()
  } catch (error) {
    console.error('memory size failed', error)
  }
})

const locked = computed(() => capture.busy || !device.state)

const modeOption = computed(() => device.option('operationMode'))
const modeChoices = computed(() =>
  modeOption.value?.kind.type === 'choice' ? modeOption.value.kind.choices : [],
)
const mode = computed(() =>
  modeOption.value?.value.type === 'choice' ? modeOption.value.value.value : null,
)
const modeLabel = computed(() => modeChoices.value.find((choice) => choice.id === mode.value))
const streaming = computed(() => mode.value === 'stream')
const looping = computed(() => capture.mode === 'loop' && capture.canLoop)

const tiers = computed(() => device.state?.rateTiers ?? [])
const currentTier = computed(() => device.state?.rateTier ?? null)

const rateOption = computed(() => device.option('samplerate'))
const limitOption = computed(() => device.option('sampleLimit'))
/** Rates the enabled channels allow. */
const allowed = computed(() =>
  rateOption.value?.kind.type === 'integers' ? rateOption.value.kind.values : [],
)
/** Every rate of the mode; those the enabled channels do not allow say what they need. */
const rates = computed(() =>
  [...new Set([...tierRates(tiers.value), ...allowed.value])].sort((a, b) => a - b),
)
const deviceMax = computed(() =>
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
/** Stream captures go to this machine's memory, which bounds them before the device does. */
const maxSamples = computed(() =>
  streaming.value
    ? memoryLimit(memory.value, channelCount.value, deviceMax.value)
    : deviceMax.value,
)
const durations = computed(() => durationChoices(rate.value, maxSamples.value))
const currentDuration = computed(() => closestDuration(durations.value, seconds.value))
/** A Stream capture at its longest runs until stopped, or until memory runs out. */
const untilStopped = computed(
  () =>
    streaming.value &&
    !looping.value &&
    durations.value.length > 0 &&
    currentDuration.value === durations.value[0],
)

/** The device's options but those the sections above hold. */
const otherOptions = computed(() =>
  (device.state?.options ?? []).filter(
    (option) =>
      option.key !== 'samplerate' &&
      option.key !== 'sampleLimit' &&
      option.key !== 'operationMode' &&
      !(option.key === 'channelMode' && tiers.value.length > 0),
  ),
)

/** Durations beyond the device's memory need RLE compression, which DSView marks. */
const needsRle = (duration: number) => {
  const depth = device.state?.hardwareDepth
  return depth != null && samplesFor(duration, rate.value) > depth
}

const needed = computed(() => (device.sampleLimit * channelCount.value) / 8)
const tooLarge = computed(() => memory.value !== null && needed.value > memory.value * MEMORY_SHARE)

/** What bounds the duration: the device's memory, this machine's, or the device itself. */
const limitNote = computed(() => {
  const count = channelCount.value
  if (!device.state || count === 0 || !(rate.value > 0)) return null
  const depth = device.state.hardwareDepth
  if (depth != null)
    return t('header.acquisition.limitDevice', {
      count,
      duration: formatTimeShort(depth / rate.value),
    })
  if (!streaming.value) return null
  const duration = formatTimeShort(maxSamples.value / rate.value)
  return memory.value !== null && maxSamples.value < deviceMax.value
    ? t('header.acquisition.limitMemory', { count, duration })
    : t('header.acquisition.limitMost', { duration })
})

const durationText = computed(() =>
  untilStopped.value ? t('header.acquisition.untilStopped') : formatTimeShort(seconds.value),
)
const label = computed(() =>
  t('header.acquisition.aria', {
    rate: formatSamplerate(rate.value),
    duration: durationText.value,
    channels: channelCount.value,
  }),
)

function tierTip(tier: RateTier): string {
  const rate = formatSamplerate(tier.maxSamplerate)
  if (tier.leading) return t('header.acquisition.tierLeading', { last: tier.channels - 1, rate })
  return tier.span < (device.state?.channelCount ?? 0)
    ? t('header.acquisition.tierSome', { count: tier.channels, last: tier.span - 1, rate })
    : t('header.acquisition.tierAny', { count: tier.channels, rate })
}

/** The tier a rate the enabled channels do not allow needs. */
function tierNeeded(value: number): RateTier | undefined {
  return allowed.value.includes(value) ? undefined : tiers.value[tierForRate(tiers.value, value)]
}

/** The rate list: a rate the enabled channels do not allow says which channels it needs. */
const rateRows = computed(() =>
  rates.value.map((value) => {
    const tier = tierNeeded(value)
    return {
      value,
      need: !tier
        ? null
        : tier.leading
          ? t('header.acquisition.needLeading', { last: tier.channels - 1 })
          : t('header.acquisition.needAny', { count: tier.channels }),
      tip: tier ? t('header.acquisition.needTip', { tier: tierTip(tier) }) : undefined,
      usable: tier !== undefined || allowed.value.includes(value),
    }
  }),
)

const sameChannels = (a: readonly number[], b: readonly number[]) =>
  a.length === b.length && a.every((channel, index) => channel === b[index])

/**
 * Shortcuts to a number of channels: the device's rate tiers, each with the rate it reaches,
 * or plain counts when its channels do not bound the rate.
 */
const shortcuts = computed(() =>
  tiers.value.length > 1
    ? tiers.value.map((tier, index) => ({
        count: tier.channels,
        // A tier of the first channels is a range of them, not a count: D14 alone needs 0–15.
        name: tier.leading
          ? t('header.acquisition.range', { last: tier.channels - 1 })
          : t('header.acquisition.channels', { count: tier.channels }),
        rate: tier.maxSamplerate,
        pressed: index === currentTier.value,
        tip: tierTip(tier),
        pick: () => pickTier(index),
      }))
    : presetCounts(device.state?.channelCount ?? 0).map((count) => ({
        count,
        name: t('header.acquisition.channels', { count }),
        rate: null,
        pressed: sameChannels(device.enabledChannels, leadingChannels(count)),
        tip: t('header.acquisition.preset', { last: count - 1 }),
        pick: () => keepingDuration(() => device.setChannels(leadingChannels(count))),
      })),
)

/**
 * Why a tier of the first channels is slower than the number of enabled channels allows: a
 * high channel makes the device send every channel below it.
 */
const spanHint = computed(() => {
  const tier = currentTier.value === null ? undefined : tiers.value[currentTier.value]
  const enabled = device.enabledChannels
  const highest = enabled.at(-1)
  if (!tier?.leading || highest === undefined) return null
  const fit = tiers.value.find((other) => other.channels >= enabled.length)
  if (!fit?.leading || fit.maxSamplerate <= tier.maxSamplerate) return null
  return t('header.acquisition.spanHint', {
    channel: highest,
    last: tier.channels - 1,
    rate: formatSamplerate(tier.maxSamplerate),
    fit: fit.channels - 1,
    fitRate: formatSamplerate(fit.maxSamplerate),
  })
})

/**
 * Runs a change that may move the sample rate or the longest duration, then keeps the capture
 * about as long as before, as DSView does, or running until stopped.
 */
async function keepingDuration(change: () => Promise<boolean>): Promise<void> {
  const keep = seconds.value
  const until = untilStopped.value
  const before = rate.value
  if (!(await change()) || (rate.value === before && !until)) return
  const choice = until ? durations.value[0] : closestDuration(durations.value, keep)
  if (choice !== undefined) await pickDuration(choice)
}

function pickRate(next: number): Promise<void> {
  return keepingDuration(async () => {
    const tier = tierNeeded(next)
    if (
      tier &&
      !(await device.selectRateTier(
        tiers.value.indexOf(tier),
        keptChannels(tier, device.enabledChannels),
      ))
    )
      return false
    return device.setOption('samplerate', { type: 'integer', value: next })
  })
}

async function pickDuration(duration: number): Promise<void> {
  const value = Math.min(samplesFor(duration, rate.value), maxSamples.value)
  if (value !== device.sampleLimit)
    await device.setOption('sampleLimit', { type: 'integer', value })
}

function pickMode(id: string): void {
  if (id !== mode.value)
    void keepingDuration(() => device.setOption('operationMode', { type: 'choice', value: id }))
}

function pickTier(index: number): Promise<void> {
  const tier = tiers.value[index]
  if (!tier) return Promise.resolve()
  return keepingDuration(() => device.selectRateTier(index, leadingChannels(tier.channels)))
}

function pickAll(): void {
  if (tiers.value.length > 1) void pickTier(tiers.value.length - 1)
  else
    void keepingDuration(() => device.setChannels(leadingChannels(device.state?.channelCount ?? 0)))
}

function toggleChannel(channel: number): void {
  const enabled = device.enabledChannels
  if (enabled.includes(channel)) {
    void keepingDuration(() => device.setChannel(channel, false))
    return
  }
  // A channel beyond the tier moves to the fastest tier that takes it.
  const next = [...enabled, channel].sort((a, b) => a - b)
  const tier = currentTier.value === null ? undefined : tiers.value[currentTier.value]
  const index = tier && !fits(tier, next) ? tierForChannels(tiers.value, next) : -1
  void keepingDuration(() =>
    index >= 0 ? device.selectRateTier(index, next) : device.setChannel(channel, true),
  )
}

// A Stream capture stays within memory as channels come and go; one running until stopped
// follows the longest duration up again.
watch([maxSamples, streaming], ([next, stream], [previous, wasStream]) => {
  if (!stream || locked.value || !(next > 0)) return
  const limit = device.sampleLimit
  if (limit > next || (wasStream && limit === previous && next > previous))
    void device.setOption('sampleLimit', { type: 'integer', value: next })
})
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
      <span class="tabular-nums">{{ durationText }}</span>
      <i class="vr" />
      <span>{{ t('header.acquisition.channels', { count: channelCount }) }}</span>
      <AppIcon name="chevron-down" />
    </button>
    <Transition name="pop">
      <div
        v-if="open"
        role="dialog"
        :aria-label="label"
        class="absolute top-full left-0 z-20 mt-1 flex max-h-[calc(100vh-100px)] w-[360px] origin-top-left flex-col gap-3 overflow-y-auto rounded-6 bg-menu p-3 text-small"
        :class="otherOptions.length > 0 && 'lg:w-[640px]'"
      >
        <p v-if="!device.state" class="text-ink-3">{{ t('header.acquisition.noDevice') }}</p>
        <template v-else>
          <p v-if="capture.busy" class="text-caption text-ink-3">
            {{ t('header.acquisition.locked') }}
          </p>
          <div class="flex flex-col gap-3 lg:flex-row">
            <div class="flex min-w-0 flex-col gap-4 lg:w-[336px] lg:flex-none">
              <section v-if="modeOption" class="flex flex-col gap-1.5">
                <span class="text-caption text-ink-3">{{ t('device.option.operationMode') }}</span>
                <div
                  v-if="modeChoices.length > 1"
                  v-indicator
                  role="group"
                  :aria-label="t('device.option.operationMode')"
                  class="flex gap-0.5 self-start rounded-6 bg-hover p-0.5"
                >
                  <button
                    v-for="choice in modeChoices"
                    :key="choice.id"
                    class="seg"
                    :aria-pressed="choice.id === mode"
                    :aria-disabled="locked || !modeOption.enabled"
                    @click="!locked && modeOption.enabled && pickMode(choice.id)"
                  >
                    {{ text.label(choice.label) }}
                  </button>
                </div>
                <span v-else-if="modeLabel">{{ text.label(modeLabel.label) }}</span>
                <p v-if="mode === 'buffer' || mode === 'stream'" class="text-caption text-ink-3">
                  {{ t(`header.acquisition.modeHint.${mode}`) }}
                </p>
              </section>

              <section class="flex flex-col gap-1.5">
                <div class="flex items-center gap-3 text-caption text-ink-3">
                  <span class="flex-1">{{ t('header.acquisition.channelsTitle') }}</span>
                  <button
                    class="hover:text-ink disabled:opacity-55"
                    :disabled="locked"
                    @click="pickAll"
                  >
                    {{ t('header.acquisition.all') }}
                  </button>
                  <button
                    class="hover:text-ink disabled:opacity-55"
                    :disabled="locked"
                    @click="keepingDuration(() => device.setChannels([]))"
                  >
                    {{ t('header.acquisition.none') }}
                  </button>
                </div>
                <div
                  v-if="shortcuts.length > 1"
                  v-indicator
                  role="group"
                  :aria-label="t('header.acquisition.shortcuts')"
                  class="grid gap-0.5 rounded-6 bg-hover p-0.5"
                  :style="{ gridTemplateColumns: `repeat(${shortcuts.length}, minmax(0, 1fr))` }"
                >
                  <button
                    v-for="shortcut in shortcuts"
                    :key="shortcut.count"
                    class="flex min-w-0 flex-col items-center justify-center rounded-4 px-1 py-1 leading-tight text-ink-2 hover:text-ink aria-disabled:opacity-55 aria-pressed:text-ink"
                    :aria-pressed="shortcut.pressed"
                    :aria-disabled="locked"
                    :aria-label="shortcut.tip"
                    :title="shortcut.tip"
                    @click="!locked && shortcut.pick()"
                  >
                    <span class="tabular-nums">{{ shortcut.name }}</span>
                    <span
                      v-if="shortcut.rate !== null"
                      class="text-caption text-ink-3 tabular-nums"
                      >{{
                        t('header.acquisition.upTo', { rate: formatSamplerate(shortcut.rate) })
                      }}</span
                    >
                  </button>
                </div>
                <div class="grid grid-cols-8 gap-1">
                  <button
                    v-for="channel in device.state.channelCount"
                    :key="channel"
                    class="h-6 rounded-4 text-caption tabular-nums disabled:opacity-55"
                    :class="
                      device.enabledChannels.includes(channel - 1)
                        ? 'bg-control-on font-medium text-ink'
                        : 'bg-control text-ink-3 hover:bg-control-hover'
                    "
                    :aria-pressed="device.enabledChannels.includes(channel - 1)"
                    :disabled="locked"
                    @click="toggleChannel(channel - 1)"
                  >
                    {{ channel - 1 }}
                  </button>
                </div>
                <p v-if="spanHint" class="rise-in text-caption text-ink-2 tabular-nums">
                  {{ spanHint }}
                </p>
              </section>

              <section class="flex flex-col gap-1.5">
                <div class="grid grid-cols-2 gap-3">
                  <div class="flex min-w-0 flex-col gap-1">
                    <span class="text-caption text-ink-3">{{
                      t('header.acquisition.samplerate')
                    }}</span>
                    <div data-list class="flex max-h-56 flex-col overflow-y-auto">
                      <button
                        v-for="row in rateRows"
                        :key="row.value"
                        class="flex h-6 shrink-0 items-center justify-between gap-2 rounded-4 px-2 tabular-nums hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
                        :class="row.value === rate && 'font-medium'"
                        :aria-pressed="row.value === rate"
                        :title="row.tip"
                        :disabled="locked || rateOption?.enabled === false || !row.usable"
                        @click="pickRate(row.value)"
                      >
                        <span class="inline-flex min-w-0 items-baseline gap-1.5">
                          <span :class="row.need && 'text-ink-3'">{{
                            formatSamplerate(row.value)
                          }}</span>
                          <span v-if="row.need" class="truncate text-caption text-ink-3">{{
                            row.need
                          }}</span>
                        </span>
                        <AppIcon v-if="row.value === rate" name="check" />
                      </button>
                    </div>
                  </div>
                  <div class="flex min-w-0 flex-col gap-1">
                    <span class="text-caption text-ink-3">{{
                      t('header.acquisition.duration')
                    }}</span>
                    <div data-list class="flex max-h-56 flex-col overflow-y-auto">
                      <button
                        v-for="(duration, index) in durations"
                        :key="duration"
                        class="flex h-6 shrink-0 items-center justify-between rounded-4 px-2 tabular-nums hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
                        :class="duration === currentDuration && 'font-medium'"
                        :aria-pressed="duration === currentDuration"
                        :disabled="locked"
                        @click="pickDuration(duration)"
                      >
                        <span
                          v-if="index === 0 && streaming && !looping"
                          :title="formatTimeShort(duration)"
                          >{{ t('header.acquisition.untilStopped') }}</span
                        >
                        <span v-else class="inline-flex items-baseline gap-1.5"
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
                <p v-if="limitNote" class="text-caption text-ink-3 tabular-nums">
                  {{ limitNote }}
                </p>
                <p
                  v-if="tooLarge"
                  role="alert"
                  class="rise-in text-caption text-alert tabular-nums"
                >
                  {{ t('header.acquisition.tooLarge', { size: formatBytes(needed) }) }}
                </p>
              </section>
            </div>
            <!-- Beside the lists on wide windows, below them on narrow ones. -->
            <DeviceOptions
              v-if="otherOptions.length > 0"
              :options="otherOptions"
              :locked="locked"
              class="min-w-0 flex-1 border-t border-line-2 pt-3 lg:border-t-0 lg:border-l lg:pt-0 lg:pl-3"
            />
          </div>
        </template>
      </div>
    </Transition>
  </div>
</template>
