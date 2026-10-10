<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AnimatedHeight from '@/components/common/AnimatedHeight.vue'
import AppIcon from '@/components/common/AppIcon.vue'
import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import { vIndicator } from '@/motion/indicator'
import { holdInPlace } from '@/motion/list'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { useDeviceStore } from '@/stores/device'
import { useTriggerStore } from '@/stores/trigger'
import {
  MAX_SERIAL_BITS,
  MAX_STAGES,
  normalizeValue,
  valueFromHex,
  valueHex,
  type TriggerMode,
} from '@/trigger/advanced'
import { summarizeTriggers } from '@/timeline/triggerSummary'

import PatternCells from './PatternCells.vue'

/**
 * The top bar's trigger button and panel. The per-channel conditions stay on the timeline
 * (DESIGN.md §7.1); the panel holds what DSView's trigger dock has beyond them: the trigger
 * position, and multi-stage and serial triggers (§7.2).
 */
const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const device = useDeviceStore()
const trigger = useTriggerStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

const MODES: TriggerMode[] = ['simple', 'stages', 'serial']
const locked = computed(() => capture.busy)

/** Why multi-stage and serial triggers are not offered, if they are not. */
const unavailable = computed(() => {
  if (trigger.caps.advanced) return null
  if (!device.state) return t('triggerPanel.unavailable.noDevice')
  if (device.isDemo) return t('triggerPanel.unavailable.demo')
  return t('triggerPanel.unavailable.stream')
})

const summary = computed(() => summarizeTriggers(channels.armed))
const buttonTip = computed(() => {
  if (trigger.activeMode === 'stages') {
    return t('triggerPanel.summary.stages', { count: trigger.stageCount })
  }
  if (trigger.activeMode === 'serial') {
    return t('triggerPanel.summary.serial', {
      channel: capture.channelLabel(trigger.serial.dataChannel),
      bits: trigger.serial.bits,
    })
  }
  if (channels.armed.length === 0) return t('header.trigger.tipNone')
  const conditions = channels.armed
    .map(({ index, condition }) =>
      t('header.trigger.tipItem', {
        channel: capture.channelLabel(index),
        condition: t(`trigger.condition.${condition}`),
      }),
    )
    .join(t('header.trigger.tipJoin'))
  return t('header.trigger.tipSet', { conditions })
})

// Multi-stage editing: the stage shown.
const stageShown = shallowRef(0)
watch(
  () => trigger.stageCount,
  (count) => {
    if (stageShown.value >= count) stageShown.value = count - 1
  },
)
const stage = computed(() => trigger.stages[stageShown.value] ?? trigger.stages[0])

// Serial data value: typed as bits, or in hex.
const valueText = shallowRef(trigger.serial.value)
const valueError = computed(() => normalizeValue(valueText.value) === null)
watch(valueText, (text) => {
  const value = normalizeValue(text)
  if (value) trigger.serial.value = value
})
const hex = computed(() => valueHex(trigger.serial.value, trigger.serial.bits) ?? '')
function onHex(event: Event): void {
  const value = valueFromHex((event.target as HTMLInputElement).value, trigger.serial.bits)
  if (value) valueText.value = value
}

const dataChannels = computed(() => Array.from({ length: trigger.channelCount }, (_, i) => i))
const bitChoices = Array.from({ length: MAX_SERIAL_BITS }, (_, i) => i + 1)
const stageChoices = Array.from({ length: MAX_STAGES }, (_, i) => i + 1)
</script>

<template>
  <div ref="root" class="relative">
    <button
      class="btn"
      :aria-label="buttonTip"
      :title="buttonTip"
      aria-haspopup="dialog"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="trigger" />
      <span>{{ t('header.trigger.label') }}</span>
      <template v-if="trigger.activeMode === 'stages'">
        <span class="rise-in font-medium">{{
          t('triggerPanel.summary.stages', { count: trigger.stageCount })
        }}</span>
      </template>
      <template v-else-if="trigger.activeMode === 'serial'">
        <span class="rise-in font-medium">{{
          t('triggerPanel.summary.serial', {
            channel: capture.channelLabel(trigger.serial.dataChannel),
            bits: trigger.serial.bits,
          })
        }}</span>
      </template>
      <template v-else>
        <span v-if="summary.shown.length === 0" class="rise-in text-ink-3">{{
          t('header.trigger.none')
        }}</span>
        <template v-for="(item, position) in summary.shown" :key="item.index">
          <span v-if="position > 0" class="rise-in text-ink-3">+</span>
          <span class="rise-in inline-flex items-center gap-0.5 font-medium">
            <span>{{ capture.channelLabel(item.index) }}</span>
            <TriggerGlyph :condition="item.condition" />
          </span>
        </template>
        <span v-if="summary.more" class="rise-in text-ink-3">
          {{ t('header.trigger.more', { count: summary.more }) }}
        </span>
      </template>
      <AppIcon name="chevron-down" />
    </button>

    <Transition name="pop">
      <div
        v-if="open"
        role="dialog"
        :aria-label="t('triggerPanel.title')"
        class="absolute top-full left-0 z-20 mt-1 flex w-[480px] max-w-[calc(100vw-32px)] origin-top-left flex-col gap-3 overflow-x-auto rounded-6 bg-menu p-3 text-small"
      >
        <AnimatedHeight>
          <fieldset :disabled="locked" class="flex flex-col gap-3">
            <div class="flex items-center gap-3">
              <span class="w-16 flex-none text-ink-2">{{ t('triggerPanel.mode') }}</span>
              <div v-indicator role="group" class="flex gap-0.5 rounded-6 bg-hover p-0.5">
                <button
                  v-for="one in MODES"
                  :key="one"
                  class="seg"
                  :aria-pressed="trigger.activeMode === one"
                  :aria-disabled="one !== 'simple' && !trigger.caps.advanced"
                  :title="one !== 'simple' && unavailable ? unavailable : undefined"
                  @click="(one === 'simple' || trigger.caps.advanced) && (trigger.mode = one)"
                >
                  {{ t(`triggerPanel.modes.${one}`) }}
                </button>
              </div>
            </div>

            <label class="flex items-center gap-3">
              <span class="w-16 flex-none text-ink-2" :title="t('triggerPanel.positionTip')">{{
                t('triggerPanel.position')
              }}</span>
              <input
                v-model.number="trigger.position"
                type="range"
                min="1"
                :max="trigger.caps.maxPosition"
                step="1"
                class="min-w-0 flex-1 accent-ink"
                :disabled="trigger.caps.maxPosition <= 1"
                :aria-valuetext="`${trigger.activePosition}%`"
              />
              <span class="w-10 flex-none text-right tabular-nums"
                >{{ trigger.activePosition }}%</span
              >
            </label>
            <p v-if="trigger.caps.maxPosition <= 1 && device.state" class="-mt-2 pl-19 text-ink-3">
              {{ t('triggerPanel.positionFixed') }}
            </p>
            <p v-else-if="trigger.caps.maxPosition < 90" class="-mt-2 pl-19 text-ink-3">
              {{ t('triggerPanel.positionMax', { max: trigger.caps.maxPosition }) }}
            </p>

            <div class="border-t border-line-2" />

            <!-- Simple: the conditions set on the timeline. -->
            <div v-if="trigger.activeMode === 'simple'" class="rise-in flex flex-col gap-3">
              <p class="text-ink-2">{{ t('triggerPanel.simple.note') }}</p>
              <div class="flex items-center gap-2">
                <TransitionGroup
                  tag="span"
                  name="item"
                  class="relative flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1"
                  @before-leave="holdInPlace"
                >
                  <span v-if="channels.armed.length === 0" key="none" class="text-ink-3">{{
                    t('triggerPanel.simple.none')
                  }}</span>
                  <span
                    v-for="item in channels.armed"
                    :key="item.index"
                    class="inline-flex items-center gap-0.5 font-medium"
                  >
                    {{ capture.channelLabel(item.index) }}
                    <TriggerGlyph :condition="item.condition" />
                  </span>
                </TransitionGroup>
                <button
                  class="btn-ghost"
                  :disabled="channels.armed.length === 0"
                  @click="channels.clearTriggers()"
                >
                  {{ t('triggerPanel.simple.clear') }}
                </button>
              </div>
              <p v-if="unavailable" class="text-ink-3">{{ unavailable }}</p>
            </div>

            <!-- Multi-stage. -->
            <div v-else-if="trigger.activeMode === 'stages'" class="rise-in flex flex-col gap-3">
              <div class="flex items-center gap-3">
                <label class="flex items-center gap-2">
                  <span class="text-ink-2">{{ t('triggerPanel.stages.count') }}</span>
                  <select
                    v-model.number="trigger.stageCount"
                    class="h-7 rounded-6 bg-control px-1.5 text-ink tabular-nums"
                  >
                    <option v-for="count in stageChoices" :key="count" :value="count">
                      {{ count }}
                    </option>
                  </select>
                </label>
                <TransitionGroup
                  v-indicator
                  tag="div"
                  name="item"
                  role="tablist"
                  class="flex min-w-0 flex-1 flex-wrap gap-0.5"
                  @before-leave="holdInPlace"
                >
                  <button
                    v-for="index in trigger.stageCount"
                    :key="index"
                    role="tab"
                    class="seg px-1.5 tabular-nums"
                    :aria-selected="stageShown === index - 1"
                    :aria-pressed="stageShown === index - 1"
                    @click="stageShown = index - 1"
                  >
                    {{ index }}
                  </button>
                </TransitionGroup>
              </div>
              <div
                v-if="stage"
                :key="stageShown"
                role="tabpanel"
                class="rise-in flex flex-col gap-2"
              >
                <div class="flex items-start gap-3">
                  <span class="w-16 flex-none pt-3 text-ink-2">{{
                    t('triggerPanel.stages.pattern0')
                  }}</span>
                  <div class="flex flex-col gap-1">
                    <PatternCells
                      v-model="stage.pattern0"
                      :channels="trigger.channelCount"
                      :label="t('triggerPanel.stages.pattern0')"
                    />
                    <label class="flex items-center gap-1.5 text-ink-2">
                      <input v-model="stage.invert0" type="checkbox" class="accent-ink" />
                      {{ t('triggerPanel.stages.invert') }}
                    </label>
                  </div>
                </div>
                <div class="flex items-start gap-3">
                  <span class="w-16 flex-none pt-3 text-ink-2">{{
                    t('triggerPanel.stages.pattern1')
                  }}</span>
                  <div class="flex flex-col gap-1">
                    <PatternCells
                      v-model="stage.pattern1"
                      :channels="trigger.channelCount"
                      :label="t('triggerPanel.stages.pattern1')"
                    />
                    <label class="flex items-center gap-1.5 text-ink-2">
                      <input v-model="stage.invert1" type="checkbox" class="accent-ink" />
                      {{ t('triggerPanel.stages.invert') }}
                    </label>
                  </div>
                </div>
                <div class="flex items-center gap-3">
                  <span class="w-16 flex-none text-ink-2">{{
                    t('triggerPanel.stages.combine')
                  }}</span>
                  <div v-indicator role="group" class="flex gap-0.5 rounded-6 bg-hover p-0.5">
                    <button class="seg" :aria-pressed="stage.both" @click="stage.both = true">
                      {{ t('triggerPanel.stages.both') }}
                    </button>
                    <button class="seg" :aria-pressed="!stage.both" @click="stage.both = false">
                      {{ t('triggerPanel.stages.either') }}
                    </button>
                  </div>
                </div>
                <div class="flex items-center gap-3">
                  <span class="w-16 flex-none text-ink-2">{{
                    t('triggerPanel.stages.repeat')
                  }}</span>
                  <input
                    v-model.number="stage.count"
                    type="number"
                    min="1"
                    step="1"
                    class="h-7 w-24 rounded-6 bg-control px-2 text-ink tabular-nums"
                    :aria-label="t('triggerPanel.stages.repeat')"
                  />
                  <span class="text-ink-2">{{ t('triggerPanel.stages.times') }}</span>
                  <label class="flex items-center gap-1.5 text-ink-2">
                    <input v-model="stage.contiguous" type="checkbox" class="accent-ink" />
                    {{ t('triggerPanel.stages.contiguous') }}
                  </label>
                </div>
              </div>
              <p class="text-ink-3">{{ t('triggerPanel.stages.note') }}</p>
            </div>

            <!-- Serial. -->
            <div v-else class="rise-in flex flex-col gap-3">
              <div
                v-for="field in ['start', 'stop', 'clock'] as const"
                :key="field"
                class="flex items-start gap-3"
              >
                <span class="w-16 flex-none pt-3 text-ink-2">{{
                  t(`triggerPanel.serial.${field}`)
                }}</span>
                <PatternCells
                  v-model="trigger.serial[field]"
                  :channels="trigger.channelCount"
                  :label="t(`triggerPanel.serial.${field}`)"
                />
              </div>
              <div class="flex items-center gap-3">
                <span class="w-16 flex-none text-ink-2">{{ t('triggerPanel.serial.data') }}</span>
                <select
                  v-model.number="trigger.serial.dataChannel"
                  class="h-7 rounded-6 bg-control px-1.5 text-ink"
                  :aria-label="t('triggerPanel.serial.data')"
                >
                  <option v-for="channel in dataChannels" :key="channel" :value="channel">
                    {{ capture.channelLabel(channel) }}
                  </option>
                </select>
                <label class="flex items-center gap-2">
                  <span class="text-ink-2">{{ t('triggerPanel.serial.bits') }}</span>
                  <select
                    v-model.number="trigger.serial.bits"
                    class="h-7 rounded-6 bg-control px-1.5 text-ink tabular-nums"
                  >
                    <option v-for="bits in bitChoices" :key="bits" :value="bits">{{ bits }}</option>
                  </select>
                </label>
              </div>
              <div class="flex items-center gap-3">
                <span
                  class="w-16 flex-none text-ink-2"
                  :title="t('triggerPanel.serial.valueTip')"
                  >{{ t('triggerPanel.serial.value') }}</span
                >
                <input
                  v-model="valueText"
                  class="h-7 w-[20ch] flex-none rounded-6 px-2 font-mono"
                  :class="valueError ? 'bg-alert-wash text-alert' : 'bg-control text-ink'"
                  :title="t('triggerPanel.serial.valueTip')"
                  :aria-label="t('triggerPanel.serial.value')"
                  :aria-invalid="valueError"
                  spellcheck="false"
                />
                <label class="flex items-center gap-1.5">
                  <span class="text-ink-2">{{ t('triggerPanel.serial.hex') }}</span>
                  <input
                    :value="hex"
                    class="h-7 w-[7ch] flex-none rounded-6 bg-control px-2 font-mono text-ink uppercase"
                    maxlength="4"
                    spellcheck="false"
                    @change="onHex"
                  />
                </label>
              </div>
              <p v-if="valueError" role="alert" class="-mt-1 pl-19 text-alert">
                {{ t('triggerPanel.serial.invalid') }}
              </p>
            </div>

            <p v-if="trigger.activeMode !== 'simple'" class="rise-in text-ink-3">
              {{ t('triggerPanel.legend') }}
            </p>
          </fieldset>
        </AnimatedHeight>
      </div>
    </Transition>
  </div>
</template>
