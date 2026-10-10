<script setup lang="ts">
import { onBeforeUnmount, onMounted, useTemplateRef } from 'vue'

import { motionTiming } from '@/motion/timing'

/**
 * A box that eases to its content's height (DESIGN.md §13): when what it holds grows or
 * shrinks, as a panel switches views, the box goes from the height shown to the new one
 * instead of jumping, and clips the content meanwhile. The content itself changes at once.
 */
const box = useTemplateRef<HTMLElement>('box')
const content = useTemplateRef<HTMLElement>('content')
let observer: ResizeObserver | undefined
let running: Animation | undefined
/** The content height the box has settled on, or is heading for. */
let settled = 0

onMounted(() => {
  if (!content.value) return
  settled = content.value.offsetHeight
  if (typeof ResizeObserver === 'undefined') return
  observer = new ResizeObserver(follow)
  observer.observe(content.value)
})

onBeforeUnmount(() => {
  observer?.disconnect()
  running?.cancel()
})

function follow(): void {
  const outer = box.value
  const inner = content.value
  if (!outer || !inner) return
  const next = inner.offsetHeight
  if (next === settled) return
  // Mid-way through a change the box is wherever it has got to; otherwise at the old height.
  const from = running ? outer.offsetHeight : settled
  settled = next
  running?.cancel()
  running = undefined
  const { duration, easing } = motionTiming('enter')
  if (duration <= 0 || typeof outer.animate !== 'function') {
    outer.style.overflow = ''
    return
  }
  const animation = outer.animate([{ height: `${from}px` }, { height: `${next}px` }], {
    duration,
    easing,
  })
  running = animation
  outer.style.overflow = 'clip'
  animation.finished.then(
    () => {
      if (running !== animation) return
      running = undefined
      outer.style.overflow = ''
    },
    // Cancelled by a newer change, or as the box goes away.
    () => undefined,
  )
}
</script>

<template>
  <div ref="box">
    <div ref="content">
      <slot />
    </div>
  </div>
</template>
