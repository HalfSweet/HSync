/**
 * The capture lengths offered in the acquisition menu, as DSView's sampling bar builds them
 * (`samplingbar.cpp:472-618`, DESIGN.md §4): from the longest possible duration down a 1-2-5
 * sequence until fewer than 1024 samples remain. Minutes, hours and days step through 50 s,
 * 50 min and 20 h on the way down.
 */

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
/** The fewest samples a capture takes. */
export const MIN_SAMPLES = 1024
/** Share of this machine's memory a capture may fill before the menu warns about it. */
export const MEMORY_SHARE = 0.5

/** Durations in seconds, longest first. */
export function durationChoices(samplerate: number, maxSamples: number): number[] {
  if (!(samplerate > 0) || !(maxSamples >= MIN_SAMPLES)) return []
  const choices: number[] = []
  let duration = maxSamples / samplerate
  for (;;) {
    choices.push(duration)
    const unit = duration >= DAY ? DAY : duration >= HOUR ? HOUR : duration >= MINUTE ? MINUTE : 1
    const decade = 10 ** Math.floor(Math.log10(duration / unit))
    // Past a step by more than rounding: 5 × 1e-6 falls just short of the 5 µs it should be.
    const past = (step: number) => duration > step * decade * unit * (1 + 1e-9)
    if (past(5)) duration = 5 * decade * unit
    else if (past(2)) duration = 2 * decade * unit
    else if (past(1)) duration = decade * unit
    else if (decade > 1) duration *= 0.5
    else if (unit === DAY) duration = 20 * HOUR
    else if (unit === HOUR) duration = 50 * MINUTE
    else if (unit === MINUTE) duration = 50
    else duration *= 0.5
    if (duration * samplerate < MIN_SAMPLES) return choices
  }
}

/** Samples for `seconds` at `samplerate`, rounded up to a multiple of 1024 as DSView does. */
export function samplesFor(seconds: number, samplerate: number): number {
  const samples = Math.ceil(seconds * samplerate)
  return Math.max(MIN_SAMPLES, Math.ceil(samples / 1024) * 1024)
}

/**
 * Samples per channel a Stream capture of `channels` channels may take: as many as fit, bit
 * packed, in {@link MEMORY_SHARE} of this machine's `memory`, in whole kilo-samples, and no more
 * than the device's own `deviceMax`.
 */
export function memoryLimit(memory: number | null, channels: number, deviceMax: number): number {
  if (memory === null || !(channels > 0)) return deviceMax
  const samples = Math.floor((memory * MEMORY_SHARE * 8) / channels / 1024) * 1024
  return Math.min(deviceMax, Math.max(MIN_SAMPLES, samples))
}

/** The choice closest to `seconds`, so a rate change keeps the capture about as long. */
export function closestDuration(choices: readonly number[], seconds: number): number | undefined {
  let best: number | undefined
  for (const choice of choices) {
    if (
      best === undefined ||
      Math.abs(Math.log(choice / seconds)) < Math.abs(Math.log(best / seconds))
    )
      best = choice
  }
  return best
}
