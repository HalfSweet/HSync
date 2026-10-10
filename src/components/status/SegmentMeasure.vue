<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { getBackend } from '@/ipc/backend'
import type { ChannelEdges } from '@/ipc/bindings/ChannelEdges'
import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'
import { formatCount, formatSpan } from '@/utils/units'

/**
 * The marked segment in the status bar (DESIGN.md §9.6): its duration, and on click the rising
 * and falling edges of every channel inside it, as DSView's cursor measurement counts them.
 */
const { t, locale } = useI18n()
const capture = useCaptureStore()
const view = useViewStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

const span = computed<[number, number] | null>(() => {
  const segment = view.segment
  if (!segment || segment.from === segment.to) return null
  return [Math.min(segment.from, segment.to), Math.max(segment.from, segment.to)]
})

const duration = computed(() => {
  const range = span.value
  if (!range || capture.samplerate <= 0) return ''
  return formatSpan((range[1] - range[0]) / capture.samplerate)
})

/** Channels in timeline order with their labels. */
const rows = computed(() =>
  capture.groups.flatMap((group) =>
    group.channels.map((channel) => ({ index: channel.index, label: channel.label })),
  ),
)

const counts = shallowRef<Map<number, ChannelEdges>>(new Map())
let sequence = 0
watch(
  () => [open.value, span.value, capture.summary?.id] as const,
  async ([shown, range, id]) => {
    const mine = ++sequence
    if (!shown || !range || id === undefined) return
    try {
      const found = await (
        await getBackend()
      ).edgeCounts(
        id,
        range[0],
        range[1],
        rows.value.map((row) => row.index),
      )
      if (mine === sequence) counts.value = new Map(found.map((one) => [one.channel, one]))
    } catch (error) {
      console.error('edge count failed', error)
    }
  },
)

// The table closes with the segment.
watch(span, (range) => {
  if (!range) open.value = false
})

const range = computed(() => {
  const segment = span.value
  if (!segment) return ''
  return t('measure.range', {
    from: (capture.timeOf(segment[0]) * 1000).toFixed(4),
    to: (capture.timeOf(segment[1]) * 1000).toFixed(4),
  })
})
</script>

<template>
  <div v-if="span" ref="root" class="rise-in relative">
    <button
      class="-mx-1 inline-flex h-[22px] items-center gap-1 rounded-4 px-1 whitespace-nowrap text-ink-2 tabular-nums hover:bg-hover hover:text-ink aria-expanded:bg-hover aria-expanded:text-ink"
      aria-haspopup="dialog"
      :aria-expanded="open"
      :title="t('measure.tip')"
      @click="open = !open"
    >
      {{ t('measure.segment', { duration }) }}
    </button>
    <Transition name="pop">
      <div
        v-if="open"
        role="dialog"
        :aria-label="t('measure.title')"
        class="absolute bottom-full left-0 z-20 mb-1 flex max-h-[60vh] w-72 origin-bottom-left flex-col rounded-6 border border-ink bg-raise text-small"
      >
        <div class="flex flex-none flex-col gap-0.5 border-b border-line px-3 py-2">
          <span class="font-medium">{{ t('measure.title') }}</span>
          <span class="text-ink-3 tabular-nums">{{ range }} · {{ duration }}</span>
        </div>
        <div class="min-h-0 overflow-y-auto px-3 py-1.5">
          <table class="w-full tabular-nums">
            <thead class="text-ink-3">
              <tr>
                <th class="pb-1 text-left font-normal">{{ t('measure.channel') }}</th>
                <th class="pb-1 text-right font-normal">{{ t('measure.rising') }}</th>
                <th class="pb-1 text-right font-normal">{{ t('measure.falling') }}</th>
                <th class="pb-1 text-right font-normal">{{ t('measure.edges') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in rows" :key="row.index" class="h-5">
                <td class="truncate">
                  {{ row.label }}
                  <span
                    v-if="row.label !== `D${row.index}`"
                    class="font-mono text-caption text-ink-3"
                    >D{{ row.index }}</span
                  >
                </td>
                <template v-if="counts.get(row.index)">
                  <td class="text-right">
                    {{ formatCount(counts.get(row.index)?.rising ?? 0, locale) }}
                  </td>
                  <td class="text-right">
                    {{ formatCount(counts.get(row.index)?.falling ?? 0, locale) }}
                  </td>
                  <td class="text-right">
                    {{
                      formatCount(
                        (counts.get(row.index)?.rising ?? 0) +
                          (counts.get(row.index)?.falling ?? 0),
                        locale,
                      )
                    }}
                  </td>
                </template>
                <td v-else colspan="3" class="text-right text-ink-3">…</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </Transition>
  </div>
</template>
