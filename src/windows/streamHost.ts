import { onScopeDispose, watch } from 'vue'

import { onMainMessage, onStreamWindowClosed, tellStream } from '@/ipc/windowLink'
import { useCaptureStore } from '@/stores/capture'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'

import type { ToMain, ToStream } from './messages'

/**
 * The main window's side of the data stream window (DESIGN.md §9): it tells the window what it
 * owns (the capture shown, the segment, the search, the stream listed) while the window is
 * open, and does what the window asks of the timeline.
 */
export function useStreamWindowHost(): void {
  const capture = useCaptureStore()
  const finder = useSearchStore()
  const stream = useStreamStore()
  const view = useViewStore()

  function tell(message: ToStream): void {
    if (!stream.windowOpen) return
    tellStream(message).catch((error: unknown) => console.error('data stream window', error))
  }

  const captureMessage = (): ToStream => ({
    type: 'capture',
    summary: capture.summary,
    live: capture.live,
  })
  const listingMessage = (): ToStream => ({
    type: 'listing',
    listing: { decoder: stream.decoder, stream: stream.stream },
  })

  function tellEverything(): void {
    tell(captureMessage())
    tell({ type: 'segment', segment: view.segment })
    tell({ type: 'found', result: finder.result, current: finder.current })
    tell(listingMessage())
  }

  function receive(message: ToMain): void {
    switch (message.type) {
      case 'hello':
        tellEverything()
        break
      case 'reveal':
        stream.reveal(message.span)
        break
      case 'hot':
        stream.hot = message.span
        break
      case 'listing':
        if (message.listing.decoder !== null) {
          stream.select(message.listing.decoder, message.listing.stream)
        }
        break
      case 'dock':
        stream.dock()
        break
    }
  }

  watch(
    () => [capture.summary, capture.live] as const,
    () => tell(captureMessage()),
  )
  watch(
    () => view.segment,
    (segment) => tell({ type: 'segment', segment }),
  )
  watch(
    () => finder.result,
    (result) => tell({ type: 'found', result, current: finder.current }),
  )
  watch(
    () => finder.current,
    (current) => tell({ type: 'current', current }),
  )
  watch(
    () => [stream.decoder, stream.stream] as const,
    () => tell(listingMessage()),
  )

  const unlisten: (() => void)[] = []
  let disposed = false
  const keep = (pending: Promise<() => void>) =>
    void pending.then((stop) => (disposed ? stop() : unlisten.push(stop)))
  keep(onMainMessage(receive))
  keep(
    onStreamWindowClosed(() => {
      stream.windowOpen = false
      stream.hot = null
    }),
  )
  onScopeDispose(() => {
    disposed = true
    for (const stop of unlisten) stop()
  })
}
