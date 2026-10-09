import type { UiError } from './bindings/UiError'

export type { UiError }

/** Whether a rejected command carried a backend {@link UiError}. */
export function isUiError(error: unknown): error is UiError {
  return (
    typeof error === 'object' &&
    error !== null &&
    typeof (error as { code?: unknown }).code === 'string' &&
    typeof (error as { params?: unknown }).params === 'object'
  )
}

/** Normalises anything a command may reject with into a {@link UiError}. */
export function toUiError(error: unknown): UiError {
  if (isUiError(error)) return error
  const detail = error instanceof Error ? error.message : String(error)
  return { code: 'error.app.unexpected', params: { detail } }
}
