/**
 * Keyboard navigation of the timeline (DESIGN.md §9.10): `+` and `-` zoom, `←` and `→` pan
 * (further with Shift), `Home` shows the whole capture and `Esc` drops the edge measurement, or
 * else the segment.
 */

export type TimelineAction =
  | { kind: 'zoom'; factor: number }
  /** Pan by a share of the visible width; positive is towards later samples. */
  | { kind: 'pan'; share: number }
  | { kind: 'fit' }
  /** Let go of the edge measurement, or else of the segment. */
  | { kind: 'clear' }

type Keys = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>

/** The timeline action of a key press, or `null` for keys it leaves alone. */
export function timelineAction(event: Keys): TimelineAction | null {
  if (event.ctrlKey || event.metaKey || event.altKey) return null
  switch (event.key) {
    case '+':
    case '=':
      return { kind: 'zoom', factor: 2 }
    case '-':
    case '_':
      return { kind: 'zoom', factor: 0.5 }
    case 'ArrowLeft':
      return { kind: 'pan', share: event.shiftKey ? -0.5 : -0.1 }
    case 'ArrowRight':
      return { kind: 'pan', share: event.shiftKey ? 0.5 : 0.1 }
    case 'Home':
      return { kind: 'fit' }
    case 'Escape':
      return { kind: 'clear' }
    default:
      return null
  }
}

/** Whether a key press belongs to something else: a text field, or an open menu or dialog. */
export function keyTakenElsewhere(
  target: EventTarget | null,
  root: ParentNode = document,
): boolean {
  const element = target instanceof Element ? target : null
  if (element?.closest('input, textarea, select, [contenteditable="true"]')) return true
  return root.querySelector('[role="menu"], [role="dialog"], [role="listbox"]') !== null
}
