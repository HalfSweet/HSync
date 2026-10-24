import { describe, expect, it } from 'vitest'

import { PANEL_MIN, TIMELINE_MIN, panelWidth, spans } from './layout'

describe('data stream layout', () => {
  it('keeps the panel within its bounds, leaving the timeline its share', () => {
    expect(panelWidth(400, 1440)).toBe(400)
    expect(panelWidth(1200, 1440)).toBe(1440 - TIMELINE_MIN)
    expect(panelWidth(120, 1440)).toBe(PANEL_MIN)
    // A window too narrow for both keeps the panel at its least; the timeline scrolls.
    expect(panelWidth(400, 800)).toBe(PANEL_MIN)
  })

  it('knows the event a segment selects', () => {
    expect(spans({ start: 10, end: 20 }, [10, 20])).toBe(true)
    expect(spans({ start: 10, end: 20 }, [10, 21])).toBe(false)
    expect(spans({ start: 10, end: 20 }, null)).toBe(false)
  })
})
