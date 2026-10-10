<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useClockFrequency } from '@/composables/useClockFrequency'
import { useDecodeStore } from '@/stores/decode'
import { useStreamStore } from '@/stores/stream'
import { I2C_RATES, nominalRate } from '@/timeline/clock'
import { uartFormat, type BusKind, type ChannelGroup } from '@/timeline/groups'
import { formatFrequencyShort, formatSamplerate } from '@/utils/units'

const props = defineProps<{ group: ChannelGroup; collapsed: boolean }>()
const emit = defineEmits<{ toggle: [] }>()

const { t } = useI18n()
const stream = useStreamStore()
const decode = useDecodeStore()
const showing = computed(() => stream.open && stream.decoder === props.group.decoder)

const title = computed(() => (props.group.unnamed ? t('group.unnamed') : props.group.title))

const SQUARES: Record<BusKind, string> = {
  i2c: 'bg-i2c',
  uart: 'bg-uart',
  spi: 'bg-spi',
  can: 'bg-can',
  other: 'bg-ink-2',
}
const square = computed(() =>
  props.group.unnamed ? 'shadow-[inset_0_0_0_1px_var(--trace-2)]' : SQUARES[props.group.kind],
)

const clockChannel = computed(
  () =>
    props.group.channels.find((channel) => channel.role === 'scl' || channel.role === 'clk')?.index,
)
const clock = useClockFrequency(clockChannel)

/** I²C shows its nominal bus speed with the measured clock as a tip; SPI the measured clock. */
const meta = computed<{ text: string; tip?: string } | null>(() => {
  if (props.group.unnamed) return { text: String(props.group.channels.length) }
  if (props.group.kind === 'uart') {
    const format = uartFormat(props.group.options)
    return format ? { text: format } : null
  }
  if (clock.value === null) return null
  const measured = formatFrequencyShort(clock.value)
  const nominal = props.group.kind === 'i2c' ? nominalRate(clock.value, I2C_RATES) : null
  return nominal
    ? { text: formatSamplerate(nominal), tip: t('group.measuredClock', { value: measured }) }
    : { text: measured }
})
</script>

<template>
  <div class="group/label flex min-w-0 flex-1 items-center gap-2">
    <button
      class="inline-flex size-4 flex-none items-center justify-center rounded-4 text-ink-2 hover:bg-hover"
      :aria-label="t('group.toggle', { name: title })"
      :aria-expanded="!collapsed"
      @click="emit('toggle')"
    >
      <AppIcon
        name="chevron-down"
        class="motion-safe:transition-transform motion-safe:duration-[120ms]"
        :class="collapsed && '-rotate-90'"
      />
    </button>
    <i class="size-2 flex-none rounded-2" :class="square" />
    <span class="truncate" :class="group.unnamed ? 'font-medium' : 'font-semibold'">{{
      title
    }}</span>
    <button
      v-if="!group.unnamed && group.decoder !== null"
      class="ml-auto hidden h-[18px] w-5 flex-none items-center justify-center rounded-4 text-ink-2 group-hover/label:inline-flex hover:bg-hover hover:text-ink focus-visible:inline-flex max-[640px]:!hidden"
      :aria-label="t('decoderDialog.edit')"
      :title="t('decoderDialog.edit')"
      @click="group.decoder !== null && decode.openEditor(group.decoder)"
    >
      <AppIcon name="settings" />
    </button>
    <button
      v-if="!group.unnamed && group.decoder !== null"
      class="ml-auto inline-flex h-[18px] w-5 flex-none items-center justify-center rounded-4 text-ink-2 opacity-0 group-hover/label:ml-0 group-hover/label:opacity-100 hover:bg-hover hover:text-ink focus-visible:opacity-100 aria-pressed:bg-hover aria-pressed:text-ink aria-pressed:opacity-100 max-[640px]:hidden"
      :aria-label="t('group.showStream', { name: title })"
      :aria-pressed="showing"
      :title="t('group.showStreamTip')"
      @click="group.decoder !== null && stream.show(group.decoder)"
    >
      <AppIcon name="stream" />
    </button>
    <span
      v-if="meta"
      class="text-small text-ink-3 tabular-nums max-[640px]:hidden"
      :class="group.unnamed && 'ml-auto'"
      :title="meta.tip"
    >
      {{ meta.text }}
    </span>
  </div>
</template>
