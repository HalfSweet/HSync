/**
 * The data stream panel coming and going (DESIGN.md §13): its width grows from nothing at the
 * right edge of the work area, or shrinks back to it, so that the overview strip and the
 * timeline give way to it frame by frame instead of all at once. The panel's content keeps its
 * width and slides in and out with the panel's left edge.
 */
import { motionTiming, type MotionName } from './timing'

/** The change each panel is going through. */
const running = new WeakMap<Element, Animation>()

function resize(
  box: HTMLElement,
  from: number,
  to: number,
  name: MotionName,
  done: () => void,
): void {
  running.get(box)?.cancel()
  running.delete(box)
  const { duration, easing } = motionTiming(name)
  if (duration <= 0 || typeof box.animate !== 'function') {
    done()
    return
  }
  const animation = box.animate([{ width: `${from}px` }, { width: `${to}px` }], {
    duration,
    easing,
    // A panel that has closed stays closed until it is taken away.
    fill: to === 0 ? 'forwards' : 'none',
  })
  running.set(box, animation)
  animation.finished.then(
    () => {
      running.delete(box)
      done()
    },
    // Cancelled by the opposite change, which goes on from where this one got to.
    () => undefined,
  )
}

/** Vue `Transition` hook: the panel opens to the width it is given. */
export function drawerEnter(element: Element, done: () => void): void {
  const box = element as HTMLElement
  resize(box, 0, box.offsetWidth, 'enter', done)
}

/** Vue `Transition` hook: the panel closes, from as wide as it has got, ignoring the pointer. */
export function drawerLeave(element: Element, done: () => void): void {
  const box = element as HTMLElement
  box.style.pointerEvents = 'none'
  resize(box, box.getBoundingClientRect().width, 0, 'exit', done)
}
