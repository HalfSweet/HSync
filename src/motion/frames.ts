/**
 * One animation frame clock for what moves the timeline's view (a glide, a fling, a trackpad's
 * gathered wheel events) and what draws it. A frame runs its callbacks, lets Vue apply what they
 * changed, then runs what that asked to draw, all before the frame is painted: the canvas never
 * shows a view one frame behind the rows over it.
 */
import { nextTick } from 'vue'

export type FrameCallback = (now: number) => void

let next: FrameCallback[] = []
let drawing: FrameCallback[] = []
let request = 0
/** The time of the frame being run, while it runs. */
let running: number | null = null
/** Rounds of drawing a frame runs at most; what the last asks for waits for the next frame. */
const DRAW_ROUNDS = 3

function schedule(): void {
  request ||= requestAnimationFrame((now) => void run(now))
}

/** Runs `callback` in the next animation frame. */
export function onNextFrame(callback: FrameCallback): void {
  next.push(callback)
  schedule()
}

/**
 * Runs `callback` once the frame being run has applied its changes, or in the next frame when
 * none is running: for drawing what a frame's changes moved.
 */
export function onFrameDrawn(callback: FrameCallback): void {
  drawing.push(callback)
  if (running === null) schedule()
}

async function run(now: number): Promise<void> {
  request = 0
  running = now
  try {
    const callbacks = next
    next = []
    for (const callback of callbacks) callback(now)
    // What Vue applies may ask to draw, and drawing may change what Vue shows.
    for (let round = 0; round < DRAW_ROUNDS; round++) {
      await nextTick()
      const draws = drawing
      drawing = []
      for (const callback of draws) callback(now)
      if (drawing.length === 0) break
    }
  } finally {
    running = null
    if (drawing.length > 0) schedule()
  }
}
