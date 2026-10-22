/** Retained import diagnostics are data; copy is resolved through vue-i18n at display time. */
export interface ImportDiagnostic {
  code: string
  params: Record<string, string>
}

export interface DecoderImportWarning {
  index: number
  label: string
  runnable: boolean
  diagnostics: ImportDiagnostic[]
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Reads preserved imports without changing their raw metadata or executable entry indices. */
export function decoderImportWarnings(entries: unknown): DecoderImportWarning[] {
  if (!Array.isArray(entries)) return []
  return entries.flatMap((entry: unknown, index) => {
    if (!record(entry) || !record(entry.import) || entry.import.format !== 'dsview') return []
    const metadata = entry.import
    const diagnostics: ImportDiagnostic[] = Array.isArray(metadata.diagnostics)
      ? metadata.diagnostics.flatMap((diagnostic: unknown) => {
          if (
            !record(diagnostic) ||
            typeof diagnostic.code !== 'string' ||
            !record(diagnostic.params)
          )
            return []
          const params = Object.fromEntries(
            Object.entries(diagnostic.params).filter(
              (pair): pair is [string, string] => typeof pair[1] === 'string',
            ),
          )
          return [{ code: diagnostic.code, params }]
        })
      : []
    if (diagnostics.length === 0) return []
    const raw = record(metadata.raw) ? metadata.raw : {}
    const rawId = raw.id
    const fallback = typeof rawId === 'string' || typeof rawId === 'number' ? String(rawId) : ''
    const label = typeof entry.label === 'string' && entry.label ? entry.label : fallback
    return [{ index, label, runnable: metadata.runnable === true, diagnostics }]
  })
}
