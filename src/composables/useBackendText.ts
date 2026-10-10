import { useI18n } from 'vue-i18n'

import type { Label } from '@/ipc/bindings/Label'
import type { UiError } from '@/ipc/errors'
import { formatSamplerate } from '@/utils/units'

type Params = Record<string, string | number>

/** Parameters as the messages show them: frequencies in Hz get their unit. */
function present(params: Record<string, unknown>): Params {
  const out: Params = {}
  for (const [name, value] of Object.entries(params)) {
    if (name === 'rate' && typeof value === 'number') out[name] = formatSamplerate(value)
    else if (typeof value === 'number' || typeof value === 'string') out[name] = value
    else out[name] = String(value)
  }
  return out
}

/**
 * Translates text the backend sends as an i18n key with parameters: option
 * labels and errors. Unknown keys show as themselves so nothing is silently dropped.
 */
export function useBackendText() {
  const i18n = useI18n()
  const translate = (key: string, params: Record<string, unknown>): string =>
    // Keys come from the backend, so they are checked at run time rather than typed.
    i18n.te(key) ? i18n.t(key as never, present(params)) : key

  return {
    label: (label: Label) => translate(label.key, label.params),
    error: (error: UiError) => translate(error.code, error.params),
  }
}
