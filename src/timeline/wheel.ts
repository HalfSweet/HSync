/**
 * What a turn of the wheel over the timeline does (DESIGN.md §10): Ctrl/⌘ + wheel, which a
 * trackpad pinch sends too, zooms around the pointer; Shift + wheel and sideways swipes pan; the
 * wheel alone scrolls the rows, unless the setting has it zoom the waveforms, which it then does
 * everywhere but over the label column.
 *
 * A wheel that turns in notches zooms by the same step each notch and the view glides there; a
 * trackpad or a wheel that turns smoothly zooms with how far it moves, at once.
 */

export type WheelAction =
  /** Zoom by `factor` around the pointer; `notched` when it is a wheel's notches, to glide. */
  | { kind: 'zoom'; factor: number; notched: boolean }
  /** Pan by `dx` CSS pixels; positive is towards later samples. */
  | { kind: 'pan'; dx: number }

export type Wheel = Pick<
  WheelEvent,
  'deltaX' | 'deltaY' | 'deltaMode' | 'ctrlKey' | 'metaKey' | 'shiftKey'
> & {
  /** The legacy, non-standard delta, which Chromium and WebKitGTK report as 120 a notch. */
  wheelDeltaY?: number
}

export interface WheelContext {
  /** The wheel alone zooms (the "Zoom the waveform with the wheel alone" setting). */
  wheelZoom: boolean
  /** The pointer is over the label column. */
  overLabels: boolean
  /** The Control key is down, so Ctrl comes from it rather than from a trackpad pinch. */
  controlHeld: boolean
}

/** Pixels a line of a line-based wheel stands for. */
const LINE = 16
/** The zoom of one notch: two make the 2× of the zoom keys. */
export const NOTCH_ZOOM = Math.SQRT2
/** What Chromium and WebKitGTK report as the legacy delta of a notch. */
const NOTCH_DELTA = 120
/** Lines a notch scrolls in Firefox, which reports wheels in lines. */
const NOTCH_LINES = 3
/**
 * WebKit on macOS reports a notch as its accelerated line count times 40 pixels, the lines in
 * 16.16 fixed point: a slow notch, 0.1 line, is 4.000244… pixels. Each such event is one notch,
 * as Chromium counts them. A trackpad moves in whole or half pixels, never finer than a quarter.
 */
const FIXED_POINT_PIXEL = 40 / 65536
const TRACKPAD_PIXEL = 1 / 4

function isMultiple(value: number, step: number, tolerance: number): boolean {
  const steps = value / step
  return Math.abs(steps - Math.round(steps)) * step <= tolerance
}

/**
 * The notches of a wheel the event stands for, signed as its `deltaY`, or `null` for a trackpad
 * or a wheel that turns smoothly.
 */
export function wheelNotches(event: Wheel): number | null {
  const delta = event.deltaY
  if (delta === 0) return null
  const sign = Math.sign(delta)
  if (event.deltaMode === 1) return sign * Math.max(1, Math.round(Math.abs(delta) / NOTCH_LINES))
  if (event.deltaMode === 2) return sign
  // Chromium on Windows can report 119 when the display scale is an odd fraction.
  const legacy = Math.abs(event.wheelDeltaY ?? 0)
  if (legacy >= NOTCH_DELTA - 1 && isMultiple(legacy, NOTCH_DELTA, 1)) {
    return sign * Math.round(legacy / NOTCH_DELTA)
  }
  const pixels = Math.abs(delta)
  if (isMultiple(pixels, FIXED_POINT_PIXEL, 1e-9) && !isMultiple(pixels, TRACKPAD_PIXEL, 1e-9)) {
    return sign
  }
  return null
}

/** The zoom of a wheel event that zooms. */
export function wheelZoom(
  event: Wheel,
  controlHeld: boolean,
): Extract<WheelAction, { kind: 'zoom' }> {
  // A pinch sends Ctrl without the key and, in Chromium, a notch's legacy delta.
  const pinch = event.ctrlKey && !controlHeld
  const notches = pinch ? null : wheelNotches(event)
  if (notches !== null) return { kind: 'zoom', factor: NOTCH_ZOOM ** -notches, notched: true }
  const scale = event.deltaMode === 1 ? LINE : 1
  return { kind: 'zoom', factor: Math.exp(-event.deltaY * scale * 0.002), notched: false }
}

/** The timeline's action for a wheel event, or `null` to leave it to the scrolling rows. */
export function wheelAction(event: Wheel, context: WheelContext): WheelAction | null {
  const scale = event.deltaMode === 1 ? LINE : 1
  if (event.ctrlKey || event.metaKey) return wheelZoom(event, context.controlHeld)
  if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
    const delta = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX
    return { kind: 'pan', dx: delta * scale }
  }
  return context.wheelZoom && !context.overLabels ? wheelZoom(event, context.controlHeld) : null
}
