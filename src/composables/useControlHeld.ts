/**
 * Whether the Control key is down. Chromium sends a trackpad pinch as a wheel event with Ctrl set
 * though the key is up, and with the legacy delta of a wheel's notch, so the wheel tells a pinch
 * from Ctrl + wheel by this. Keys and pointer moves carry the keyboard's real state.
 */
let held = false
let listening = false

function update(event: KeyboardEvent | PointerEvent): void {
  held = event.ctrlKey
}

export function useControlHeld(): () => boolean {
  if (!listening) {
    listening = true
    const options = { capture: true, passive: true }
    window.addEventListener('keydown', update, options)
    window.addEventListener('keyup', update, options)
    window.addEventListener('pointermove', update, options)
    window.addEventListener('pointerdown', update, options)
    window.addEventListener('blur', () => {
      held = false
    })
  }
  return () => held
}
