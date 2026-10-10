/** CSV as spreadsheets read it (RFC 4180): fields with commas, quotes or line breaks quoted. */

export function csvField(text: string): string {
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
}

export function csvLines(rows: readonly (readonly string[])[]): string {
  return rows.map((row) => row.map(csvField).join(',')).join('\r\n') + '\r\n'
}
