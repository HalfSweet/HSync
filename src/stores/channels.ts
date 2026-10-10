import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import type { TriggerCondition } from '@/ipc/bindings/TriggerCondition'

/** Per-channel settings the user makes on the timeline: trigger conditions and open groups. */
export const useChannelsStore = defineStore('channels', () => {
  /** Trigger condition by channel index; channels without one are absent. */
  const triggers = ref<Record<number, TriggerCondition>>({})
  /** Collapsed state by group key; groups not listed use their default. */
  const collapsed = ref<Record<string, boolean>>({})
  /** Share of a triggered capture before the trigger, in percent; DSView starts at 1. */
  const triggerPosition = ref(1)

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

  function isCollapsed(key: string, fallback = false): boolean {
    return collapsed.value[key] ?? fallback
  }

  function toggleGroup(key: string, fallback = false): void {
    collapsed.value = { ...collapsed.value, [key]: !isCollapsed(key, fallback) }
  }

  return {
    triggers,
    armed,
    triggerPosition,
    triggerOf,
    setTrigger,
    collapsed,
    isCollapsed,
    toggleGroup,
  }
})
