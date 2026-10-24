/**
 * Between the main window and the data stream's own window (DESIGN.md §9): opening and closing
 * that window, titling it, and the messages the two windows send each other. In the app the
 * messages go as Tauri events to the other window; in a browser (`pnpm dev`) the window is a
 * pop-up and they go over a BroadcastChannel, as JSON both ways.
 */
import { invoke, isTauri } from '@tauri-apps/api/core'
import { emitTo } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'

import type { ToMain, ToStream } from '@/windows/messages'
import { inStreamWindow } from '@/windows/role'

type Unsubscribe = () => void

/** The windows' labels, as `src-tauri/src/commands/window.rs` names them. */
const MAIN = 'main'
const STREAM = 'stream'
const MESSAGE_EVENT = 'hsync://window'
const STREAM_CLOSED_EVENT = 'window://stream-closed'
const CHANNEL = 'hsync-window'

interface Envelope {
  to: typeof MAIN | typeof STREAM
  message: ToMain | ToStream
}

let channel: BroadcastChannel | null = null
const broadcast = () => (channel ??= new BroadcastChannel(CHANNEL))

function post(to: Envelope['to'], message: ToMain | ToStream): Promise<void> {
  if (isTauri()) return emitTo(to, MESSAGE_EVENT, message)
  broadcast().postMessage(JSON.stringify({ to, message } satisfies Envelope))
  return Promise.resolve()
}

async function listenAs<T>(
  as: Envelope['to'],
  handler: (message: T) => void,
): Promise<Unsubscribe> {
  if (isTauri()) {
    return getCurrentWebviewWindow().listen<T>(MESSAGE_EVENT, (event) => handler(event.payload))
  }
  const listener = (event: MessageEvent<string>) => {
    const envelope = JSON.parse(event.data) as Envelope
    if (envelope.to === as) handler(envelope.message as T)
  }
  broadcast().addEventListener('message', listener)
  return Promise.resolve(() => broadcast().removeEventListener('message', listener))
}

/** Tells the data stream window something; from the main window. */
export function tellStream(message: ToStream): Promise<void> {
  return post(STREAM, message)
}

/** Tells the main window something; from the data stream window. */
export function tellMain(message: ToMain): Promise<void> {
  return post(MAIN, message)
}

/** What the data stream window is told; in that window. */
export function onStreamMessage(handler: (message: ToStream) => void): Promise<Unsubscribe> {
  return listenAs(STREAM, handler)
}

/** What the main window is told; in that window. */
export function onMainMessage(handler: (message: ToMain) => void): Promise<Unsubscribe> {
  return listenAs(MAIN, handler)
}

// In a browser, the main window keeps the pop-up and notices when it has closed.
let popup: Window | null = null
let watching: ReturnType<typeof setInterval> | undefined
const closedHandlers = new Set<() => void>()

function watchPopup(): void {
  clearInterval(watching)
  watching = setInterval(() => {
    if (popup && !popup.closed) return
    clearInterval(watching)
    popup = null
    for (const handler of closedHandlers) handler()
  }, 300)
}

/** Opens the data stream window, or brings it to the front. */
export async function openStreamWindow(): Promise<void> {
  if (isTauri()) return invoke<void>('stream_window_open')
  if (popup && !popup.closed) {
    popup.focus()
    return
  }
  const url = new URL(location.href)
  url.search = '?view=stream'
  popup = window.open(url, 'hsync-stream', 'popup,width=520,height=820')
  watchPopup()
}

/** Closes the data stream window, from either window. */
export async function closeStreamWindow(): Promise<void> {
  if (isTauri()) return invoke<void>('stream_window_close')
  if (inStreamWindow) window.close()
  else popup?.close()
}

/** Calls `handler` when the data stream window has closed; in the main window. */
export function onStreamWindowClosed(handler: () => void): Promise<Unsubscribe> {
  if (isTauri()) {
    return getCurrentWebviewWindow().listen(STREAM_CLOSED_EVENT, () => handler())
  }
  closedHandlers.add(handler)
  return Promise.resolve(() => closedHandlers.delete(handler))
}

/** Titles this window. */
export async function setWindowTitle(title: string): Promise<void> {
  if (isTauri()) return getCurrentWebviewWindow().setTitle(title)
  document.title = title
}
