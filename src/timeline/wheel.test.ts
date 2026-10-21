import { describe, expect, it } from 'vitest'

import {
  NOTCH_ZOOM,
  wheelAction,
  wheelNotches,
  wheelZoom,
  type Wheel,
  type WheelContext,
} from './wheel'

const turn = (patch: Partial<Wheel>): Wheel => ({
  deltaX: 0,
  deltaY: 0,
  deltaMode: 0,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...patch,
})

const tracks: WheelContext = { wheelZoom: false, overLabels: false, controlHeld: false }
const zooming: WheelContext = { wheelZoom: true, overLabels: false, controlHeld: false }

describe('wheelAction', () => {
  it('zooms with Ctrl or ⌘, in by turning up and out by turning down', () => {
    const zoomIn = wheelAction(turn({ deltaY: -100, ctrlKey: true }), tracks)
    expect(zoomIn?.kind).toBe('zoom')
    expect(zoomIn?.kind === 'zoom' && zoomIn.factor).toBeCloseTo(Math.exp(0.2))
    const zoomOut = wheelAction(turn({ deltaY: 100, metaKey: true }), tracks)
    expect(zoomOut?.kind === 'zoom' && zoomOut.factor).toBeLessThan(1)
    // Over the labels too.
    expect(
      wheelAction(turn({ deltaY: 100, ctrlKey: true }), { ...tracks, overLabels: true })?.kind,
    ).toBe('zoom')
  })

  it('pans with Shift and with sideways swipes', () => {
    expect(wheelAction(turn({ deltaY: 40, shiftKey: true }), tracks)).toEqual({
      kind: 'pan',
      dx: 40,
    })
    expect(wheelAction(turn({ deltaX: -30, deltaY: 4 }), zooming)).toEqual({
      kind: 'pan',
      dx: -30,
    })
  })

  it('leaves the wheel alone to scroll the rows unless the setting has it zoom', () => {
    expect(wheelAction(turn({ deltaY: 100 }), tracks)).toBeNull()
    const zoom = wheelAction(turn({ deltaY: 100 }), zooming)
    expect(zoom?.kind === 'zoom' && zoom.factor).toBeCloseTo(Math.exp(-0.2))
    // A trackpad's mostly vertical swipe zooms as well.
    expect(wheelAction(turn({ deltaX: 2, deltaY: 6 }), zooming)?.kind).toBe('zoom')
  })

  it('scrolls the rows over the label column even when the wheel zooms', () => {
    expect(wheelAction(turn({ deltaY: 100 }), { ...zooming, overLabels: true })).toBeNull()
  })

  it('pans a line-based wheel in lines of 16 pixels', () => {
    expect(wheelAction(turn({ deltaY: 3, deltaMode: 1, shiftKey: true }), tracks)).toEqual({
      kind: 'pan',
      dx: 48,
    })
  })
})

describe('wheelNotches', () => {
  it('counts the notches of a line-based wheel, three lines each', () => {
    expect(wheelNotches(turn({ deltaY: 3, deltaMode: 1 }))).toBe(1)
    expect(wheelNotches(turn({ deltaY: -6, deltaMode: 1 }))).toBe(-2)
    expect(wheelNotches(turn({ deltaY: 1, deltaMode: 1 }))).toBe(1)
    expect(wheelNotches(turn({ deltaY: 1, deltaMode: 2 }))).toBe(1)
  })

  it('counts notches by the legacy delta of 120 each, signed as deltaY', () => {
    // Chromium on Windows, and on macOS, where a slow notch moves 4 pixels.
    expect(wheelNotches(turn({ deltaY: 100, wheelDeltaY: -120 }))).toBe(1)
    expect(wheelNotches(turn({ deltaY: -4.000244140625, wheelDeltaY: 120 }))).toBe(-1)
    expect(wheelNotches(turn({ deltaY: 200, wheelDeltaY: -240 }))).toBe(2)
    // An odd display scale rounds a notch to 119.
    expect(wheelNotches(turn({ deltaY: 99, wheelDeltaY: -119 }))).toBe(1)
  })

  it("counts WebKit's fixed-point line deltas on macOS as a notch each", () => {
    // 0.1 line, a slow notch, then faster ones of 0.2 and 1.3 lines.
    expect(wheelNotches(turn({ deltaY: 4.000244140625, wheelDeltaY: -12 }))).toBe(1)
    expect(wheelNotches(turn({ deltaY: -7.9998779296875, wheelDeltaY: 23 }))).toBe(-1)
    expect(wheelNotches(turn({ deltaY: 52.0001220703125, wheelDeltaY: -156 }))).toBe(1)
  })

  it('finds no notches in a trackpad or a smooth wheel', () => {
    expect(wheelNotches(turn({ deltaY: 2, wheelDeltaY: -6 }))).toBeNull()
    expect(wheelNotches(turn({ deltaY: -1.5, wheelDeltaY: 4 }))).toBeNull()
    // Half pixels, though 12.5 is 20480 steps of WebKit's fixed-point lines.
    expect(wheelNotches(turn({ deltaY: 12.5, wheelDeltaY: -37 }))).toBeNull()
    expect(wheelNotches(turn({ deltaY: 0.75, wheelDeltaY: -2 }))).toBeNull()
    expect(wheelNotches(turn({ deltaY: 0, wheelDeltaY: 0 }))).toBeNull()
  })
})

describe('wheelZoom', () => {
  it('zooms a fixed step a notch, to glide', () => {
    expect(wheelZoom(turn({ deltaY: -100, wheelDeltaY: 120 }), false)).toEqual({
      kind: 'zoom',
      factor: NOTCH_ZOOM,
      notched: true,
    })
    const out = wheelZoom(turn({ deltaY: 200, wheelDeltaY: -240 }), false)
    expect(out.factor).toBeCloseTo(0.5)
  })

  it('zooms a trackpad by how far it moves, at once', () => {
    const zoom = wheelZoom(turn({ deltaY: -6, wheelDeltaY: 18 }), false)
    expect(zoom.notched).toBe(false)
    expect(zoom.factor).toBeCloseTo(Math.exp(6 * 0.002))
  })

  it('tells a pinch, Ctrl without the key, from Ctrl + wheel', () => {
    // Chromium gives a pinch the legacy delta of a notch.
    const pinch = turn({ deltaY: -3.5, wheelDeltaY: 120, ctrlKey: true })
    expect(wheelZoom(pinch, false).notched).toBe(false)
    expect(wheelZoom(pinch, false).factor).toBeCloseTo(Math.exp(3.5 * 0.002))
    expect(wheelZoom(pinch, true).notched).toBe(true)
  })

  it('zooms with notches through wheelAction too', () => {
    const zoom = wheelAction(turn({ deltaY: 3, deltaMode: 1 }), zooming)
    expect(zoom).toEqual({ kind: 'zoom', factor: 1 / NOTCH_ZOOM, notched: true })
    const held = wheelAction(turn({ deltaY: 100, wheelDeltaY: -120, ctrlKey: true }), {
      ...tracks,
      controlHeld: true,
    })
    expect(held?.kind === 'zoom' && held.notched).toBe(true)
  })
})
