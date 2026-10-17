import { describe, expect, it } from 'vitest'

import { wheelAction, type WheelContext } from './wheel'

const turn = (patch: Partial<WheelEvent>) => ({
  deltaX: 0,
  deltaY: 0,
  deltaMode: 0,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...patch,
})

const tracks: WheelContext = { wheelZoom: false, overLabels: false }
const zooming: WheelContext = { wheelZoom: true, overLabels: false }

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

  it('counts a line-based wheel in lines of 16 pixels', () => {
    expect(wheelAction(turn({ deltaY: 3, deltaMode: 1, shiftKey: true }), tracks)).toEqual({
      kind: 'pan',
      dx: 48,
    })
    const zoom = wheelAction(turn({ deltaY: 3, deltaMode: 1 }), zooming)
    expect(zoom?.kind === 'zoom' && zoom.factor).toBeCloseTo(Math.exp(-48 * 0.002))
  })
})
