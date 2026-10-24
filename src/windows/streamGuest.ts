import { onScopeDispose, watch } from 'vue'

import { onStreamMessage, tellMain } from '@/ipc/windowLink'
import { useCaptureStore } from '@/stores/capture'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'

import type { ToMain, ToStream } from './messages'

/**
 * The data stream window's side (DESIGN.md §9): it takes the main window's capture, segment,
 * search and stream as they change, lists the events itself from the backend, and hands the
 * main window the event selected, the row under the pointer and the stream listed.
 */
export function useStreamWindowGuest(): void {
  const capture = useCaptureStore()
  const finder = useSearchStore()
  const stream = useStreamStore()
  const view = useViewStore()

  function tell(message: ToMain): void {
    tellMain(message).catch((error: unknown) => console.error('main window', error))
  }

  // The window is the panel, open from the start.
  stream.open = true
  stream.revealWith((span) => {
    view.setSegment({ from: span.start, to: span.end })
    tell({ type: 'reveal', span })
  })

  function receive(message: ToStream): void {
    switch (message.type) {
      case 'capture':
        capture.summary = message.summary
        capture.live = message.live
        break
      case 'segment':
        view.setSegment(message.segment)
        break
      case 'found':
        finder.result = message.result
        finder.current = message.current
        break
      case 'current':
        finder.current = message.current
        break
      case 'listing':
        if (message.listing.decoder !== null) {
          stream.select(message.listing.decoder, message.listing.stream)
        }
        break
    }
  }

  watch(
    () => stream.hot,
    (span) => tell({ type: 'hot', span }),
  )
  watch(
    () => [stream.decoder, stream.stream] as const,
    ([decoder, index]) => tell({ type: 'listing', listing: { decoder, stream: index } }),
  )

  let stop: (() => void) | undefined
  let disposed = false
  void onStreamMessage(receive).then((unlisten) => {
    if (disposed) {
      unlisten()
      return
    }
    stop = unlisten
    tell({ type: 'hello' })
  })
  onScopeDispose(() => {
    disposed = true
    stop?.()
  })
}
