import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'

/** Per-channel settings the user makes on the timeline: trigger conditions and open groups. */
export const useChannelsStore = defineStore('channels', () => {
  /** Trigger condition by channel index; channels without one are absent. */
  const triggers = ref<Record<number, TriggerCondition>>({})
  /** Wire colour (0..7) the user chose by channel index; others take their number's. */
  const colours = ref<Partial<Record<number, number>>>({})
  /** Collapsed state by group key; groups not listed use their default. */
  const collapsed = ref<Record<string, boolean>>({})

  /** Channels with a condition, ascending. */
  const armed = computed(() =>
    Object.entries(triggers.value)
      .map(([index, condition]) => ({ index: Number(index), condition }))
      .sort((a, b) => a.index - b.index),
  )

  function triggerOf(index: number): TriggerCondition {
    return triggers.value[index] ?? 'none'
  }

  function setTrigger(index: number, condition: TriggerCondition): void {
    const next = { ...triggers.value }
    if (condition === 'none') delete next[index]
    else next[index] = condition
    triggers.value = next
  }

  /** Clears every channel's condition. */
  function clearTriggers(): void {
    triggers.value = {}
  }

  /** Sets a channel's colour, or with `null` gives it back its number's. */
  function setColour(index: number, wire: number | null): void {
    const next = { ...colours.value }
    if (wire === null) delete next[index]
    else next[index] = wire
    colours.value = next
  }

  function isCollapsed(key: string, fallback = false): boolean {
    return collapsed.value[key] ?? fallback
  }

  function toggleGroup(key: string, fallback = false): void {
    collapsed.value = { ...collapsed.value, [key]: !isCollapsed(key, fallback) }
  }

  return {
    triggers,
    armed,
    triggerOf,
    setTrigger,
    clearTriggers,
    colours,
    setColour,
    collapsed,
    isCollapsed,
    toggleGroup,
  }
})
