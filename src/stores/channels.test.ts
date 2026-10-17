import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { useChannelsStore } from './channels'

describe('channels store', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('keeps one trigger condition per channel and lists armed channels in order', () => {
    const channels = useChannelsStore()
    channels.setTrigger(1, 'high')
    channels.setTrigger(0, 'fall')
    expect(channels.armed).toEqual([
      { index: 0, condition: 'fall' },
      { index: 1, condition: 'high' },
    ])
    channels.setTrigger(0, 'none')
    expect(channels.triggerOf(0)).toBe('none')
    expect(channels.armed).toHaveLength(1)
  })

  it('keeps the row heights dragged and gives a channel back its default', () => {
    const channels = useChannelsStore()
    channels.setHeight(3, 60)
    channels.setHeight(5, 42)
    expect(channels.heights).toEqual({ 3: 60, 5: 42 })
    channels.setHeight(3, null)
    expect(channels.heights).toEqual({ 5: 42 })
  })

  it('toggles groups from their default state', () => {
    const channels = useChannelsStore()
    expect(channels.isCollapsed('can', true)).toBe(true)
    channels.toggleGroup('can', true)
    expect(channels.isCollapsed('can', true)).toBe(false)
    channels.toggleGroup('i2c')
    expect(channels.isCollapsed('i2c')).toBe(true)
  })
})
