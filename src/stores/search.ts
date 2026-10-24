import { defineStore } from 'pinia'
import { computed, shallowRef, watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { SearchHit } from '@/ipc/bindings/SearchHit'
import type { SearchResult } from '@/ipc/bindings/SearchResult'
import { parsePattern, type PatternChannel } from '@/search/pattern'
import { inStreamWindow } from '@/windows/role'

import { useCaptureStore } from './capture'
import { useDecodeStore } from './decode'
import { useStreamStore } from './stream'
import { useViewStore } from './view'

/** Typing pauses this long before the decoders are searched. */
const DEBOUNCE_MS = 150

export type SearchKind = 'decode' | 'pattern'

/**
 * The search box (DESIGN.md §8): values or text in the decoded data, or a level pattern
 * stepped through from the current place.
 */
export const useSearchStore = defineStore('search', () => {
  const capture = useCaptureStore()
  const decode = useDecodeStore()
  const stream = useStreamStore()
  const view = useViewStore()

  const query = shallowRef('')
  /** What the decoders found for the query, in time order. */
  const result = shallowRef<SearchResult | null>(null)
  /** Position in `result.hits` of the hit shown. */
  const current = shallowRef<number | null>(null)
  /** Where the level pattern was last found. */
  const found = shallowRef<number | null>(null)
  /** The last step found nothing further in its direction. */
  const exhausted = shallowRef(false)

  /** Channels a pattern can name: by number, name or decoder role. */
  const channels = computed<PatternChannel[]>(
    () =>
      capture.summary?.channels.map((channel) => ({
        index: channel.index,
        names: [channel.name, capture.channelLabel(channel.index)],
      })) ?? [],
  )
  const pattern = computed(() => parsePattern(query.value, channels.value))
  const kind = computed<SearchKind | null>(() => {
    if (!query.value.trim()) return null
    return pattern.value ? 'pattern' : 'decode'
  })
  const hit = computed<SearchHit | null>(() =>
    current.value === null ? null : (result.value?.hits[current.value] ?? null),
  )

  // The decoders are searched again as the query, the listing or their results change; the
  // data stream's own window takes what the main window found instead (src/windows/).
  let sequence = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  if (!inStreamWindow)
    watch(
      () =>
        [kind.value === 'decode' ? query.value : '', capture.summary?.id, decode.revision] as const,
      ([text, id]) => {
        clearTimeout(timer)
        const mine = ++sequence
        current.value = null
        if (!text.trim() || id === undefined) {
          result.value = null
          return
        }
        timer = setTimeout(() => {
          void (async () => {
            try {
              const next = await (await getBackend()).decodeSearch(id, text)
              if (mine === sequence) result.value = next
            } catch (error) {
              console.error('search failed', error)
            }
          })()
        }, DEBOUNCE_MS)
      },
    )

  // A new pattern or capture starts from the view again.
  watch(
    () => [kind.value === 'pattern' ? query.value : '', capture.summary?.id] as const,
    () => {
      found.value = null
      exhausted.value = false
    },
  )

  /** The span the timeline marks: the hit shown or the pattern's sample. */
  const mark = computed<{ start: number; end: number } | null>(() => {
    if (kind.value === 'decode' && hit.value) return { start: hit.value.start, end: hit.value.end }
    if (kind.value === 'pattern' && found.value !== null) {
      return { start: found.value, end: found.value }
    }
    return null
  })

  /** Brings `start..end` into view, keeping the zoom unless it does not fit. */
  function reveal(start: number, end: number): void {
    const span = end - start
    const shown = view.end - view.viewport.start
    if (span > shown * 0.8) view.showRange(start - span * 0.1, end + span * 0.1)
    else if (start < view.viewport.start || end > view.end) view.centerOn((start + end) / 2)
  }

  function stepHit(forward: boolean): void {
    const hits = result.value?.hits ?? []
    if (hits.length === 0) return
    let next: number
    if (current.value === null) {
      // The first step goes from the view: the first hit in it, or the last before its end.
      next = forward
        ? hits.findIndex((one) => one.start >= view.viewport.start)
        : hits.findLastIndex((one) => one.start < view.end)
      if (next < 0) next = forward ? 0 : hits.length - 1
    } else {
      next = (current.value + (forward ? 1 : hits.length - 1)) % hits.length
    }
    current.value = next
    const shown = hits[next]
    if (!shown) return
    reveal(shown.start, shown.end)
    stream.select(shown.decoder, shown.stream)
  }

  async function stepPattern(forward: boolean): Promise<void> {
    const parsed = pattern.value
    const id = capture.summary?.id
    if (parsed?.kind !== 'conditions' || parsed.conditions.length === 0 || id === undefined) return
    const from =
      found.value === null
        ? forward
          ? view.viewport.start
          : view.end
        : found.value + (forward ? 1 : -1)
    if (from < 0) {
      exhausted.value = true
      return
    }
    const direction = forward ? 'next' : 'previous'
    const next = await (await getBackend()).find(id, parsed.conditions, Math.floor(from), direction)
    exhausted.value = next === null
    if (next === null) return
    found.value = next
    reveal(next, next)
  }

  /** Shows the next (or previous) hit or place the pattern holds. */
  async function step(forward: boolean): Promise<void> {
    if (kind.value === 'decode') stepHit(forward)
    else if (kind.value === 'pattern') await stepPattern(forward)
  }

  function clear(): void {
    query.value = ''
  }

  return { query, result, current, found, exhausted, pattern, kind, hit, mark, step, clear }
})
