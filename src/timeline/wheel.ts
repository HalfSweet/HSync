/**
 * What a turn of the wheel over the timeline does (DESIGN.md §10): Ctrl/⌘ + wheel, which a
 * trackpad pinch sends too, zooms around the pointer; Shift + wheel and sideways swipes pan; the
 * wheel alone scrolls the rows, unless the setting has it zoom the waveforms, which it then does
 * everywhere but over the label column.
 */

export type WheelAction =
  /** Zoom by `factor` around the pointer. */
  | { kind: 'zoom'; factor: number }
  /** Pan by `dx` CSS pixels; positive is towards later samples. */
  | { kind: 'pan'; dx: number }

type Wheel = Pick<
  WheelEvent,
  'deltaX' | 'deltaY' | 'deltaMode' | 'ctrlKey' | 'metaKey' | 'shiftKey'
>

export interface WheelContext {
  /** The wheel alone zooms (the "Zoom the waveform with the wheel alone" setting). */
  wheelZoom: boolean
  /** The pointer is over the label column. */
  overLabels: boolean
}

/** Pixels a line of a line-based wheel stands for. */
const LINE = 16

/** The timeline's action for a wheel event, or `null` to leave it to the scrolling rows. */
export function wheelAction(event: Wheel, context: WheelContext): WheelAction | null {
  const scale = event.deltaMode === 1 ? LINE : 1
  const zoom: WheelAction = { kind: 'zoom', factor: Math.exp(-event.deltaY * scale * 0.002) }
  if (event.ctrlKey || event.metaKey) return zoom
  if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
    const delta = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX
    return { kind: 'pan', dx: delta * scale }
  }
  return context.wheelZoom && !context.overLabels ? zoom : null
}
