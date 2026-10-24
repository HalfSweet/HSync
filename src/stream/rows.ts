/** How a data stream row shows an event's fields (DESIGN.md §9). */
import { isCounter, type FieldText } from '@/decode/present'

/** A field in a row: its label, left out where it repeats the event's kind, and its value. */
export interface RowCell {
  label: string | null
  text: string
}

export interface RowParts {
  /** The number of the transfer the event belongs to, which has a column of its own. */
  counter: string | null
  cells: RowCell[]
}

/**
 * The fields of an event of kind `kind` (as named) for its row: the first that numbers the
 * transfer apart, the others in the contract's order, labelled by `labelOf`.
 */
export function rowParts(
  fields: readonly FieldText[],
  kind: string,
  labelOf: (name: string) => string,
): RowParts {
  const counterAt = fields.findIndex(({ name }) => isCounter(name))
  const sameAsKind = (label: string) => label.toLocaleLowerCase() === kind.toLocaleLowerCase()
  return {
    counter: fields[counterAt]?.text ?? null,
    cells: fields
      .filter((_, at) => at !== counterAt)
      .map(({ name, text }) => {
        const label = labelOf(name)
        return { label: sameAsKind(label) ? null : label, text }
      }),
  }
}
