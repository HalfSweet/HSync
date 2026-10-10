/** The motion tokens (tokens.css) by name: `--motion-<name>` and `--ease-<name>`. */
export type MotionName = 'enter' | 'exit' | 'slide'

export interface MotionTiming {
  /** Milliseconds; zero with reduced motion. */
  duration: number
  easing: string
}

/** Reads a motion token pair as it stands now, for animations run from script. */
export function motionTiming(name: MotionName): MotionTiming {
  const style = getComputedStyle(document.documentElement)
  return {
    duration: parseDuration(style.getPropertyValue(`--motion-${name}`)),
    easing: style.getPropertyValue(`--ease-${name}`).trim() || 'ease',
  }
}

/** `200ms` or `.2s` (as the build may write it) in milliseconds; zero if unreadable. */
export function parseDuration(text: string): number {
  const value = text.trim()
  const amount = parseFloat(value)
  if (!Number.isFinite(amount) || amount <= 0) return 0
  return value.endsWith('ms') ? amount : amount * 1000
}
