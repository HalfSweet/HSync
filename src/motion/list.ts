interface Box {
  left: number
  top: number
  width: number
  height: number
}

/** Where a list's entries stood before the update under way took any out of the flow. */
const before = new WeakMap<Element, Map<Element, Box>>()

function boxesOf(list: Element): Map<Element, Box> {
  let boxes = before.get(list)
  if (!boxes) {
    boxes = new Map()
    for (const entry of list.children) {
      if (!(entry instanceof HTMLElement)) continue
      const { offsetLeft, offsetTop, offsetWidth, offsetHeight } = entry
      boxes.set(entry, {
        left: offsetLeft,
        top: offsetTop,
        width: offsetWidth,
        height: offsetHeight,
      })
    }
    before.set(list, boxes)
    // Vue removes every entry an update drops before this runs.
    queueMicrotask(() => before.delete(list))
  }
  return boxes
}

/**
 * For `<TransitionGroup name="item" @before-leave="holdInPlace">` (DESIGN.md §9.13): takes an
 * entry that is leaving out of the flow where it stands, so the others can move into its place
 * while it fades. The list must be the entry's offset parent (positioned), and a selection mark
 * on the list skips the entry meanwhile.
 *
 * When an update drops several entries, each is held where it stood before the first went: as
 * one leaves the flow the next would otherwise be measured already moved up.
 */
export function holdInPlace(element: Element): void {
  if (!(element instanceof HTMLElement)) return
  const box = (element.parentElement && boxesOf(element.parentElement).get(element)) ?? {
    left: element.offsetLeft,
    top: element.offsetTop,
    width: element.offsetWidth,
    height: element.offsetHeight,
  }
  element.dataset.leaving = ''
  Object.assign(element.style, {
    position: 'absolute',
    left: `${box.left}px`,
    top: `${box.top}px`,
    width: `${box.width}px`,
    height: `${box.height}px`,
    margin: '0',
  })
}
