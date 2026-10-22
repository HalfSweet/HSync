import { useI18n } from 'vue-i18n'

import { activeCatalog, typeOf } from '@/decode/catalog'
import { moduleLocale } from '@/decode/copy'

/**
 * What decoders, their options and their events are called (DESIGN.md §6.3, §9, §11): the
 * copy a loaded module carries, in the interface's language where it has it, else the
 * interface's own copy for a decoder of that name (`decoder.<type>.…`), else the identifier
 * itself. Node types, fields and kinds come from the backend, so their keys are checked at
 * run time rather than typed.
 */
export function useDecoderCopy() {
  const i18n = useI18n()
  const named = (key: string, fallback: string): string =>
    i18n.te(key) ? i18n.t(key as never) : fallback
  const copy = (type: string) => moduleLocale(typeOf(type, activeCatalog.value), i18n.locale.value)
  // Compact transports and selected TAPs retain the canonical JTAG state vocabulary.
  const eventFallback = (type: string): string =>
    typeOf(type, activeCatalog.value)?.outputs.some((port) => port.schema === 'jtag.events')
      ? 'jtag'
      : type

  return {
    /** What a node type is called. */
    typeName: (type: string): string => copy(type)?.name ?? named(`decoder.${type}.name`, type),
    /** A node type's one-line summary; empty without one. */
    typeSummary: (type: string): string =>
      copy(type)?.summary ?? named(`decoder.${type}.summary`, ''),
    /** The label of a configuration field. */
    fieldLabel: (type: string, field: string): string =>
      copy(type)?.fields[field] ?? named(`decoder.${type}.field.${field}`, field),
    /** The name of an enumerated value of a configuration field. */
    valueLabel: (type: string, field: string, value: string): string =>
      copy(type)?.values[field]?.[value] ??
      named(
        `decoder.${type}.value.${field}.${value}`,
        named(`decoder.${eventFallback(type)}.value.${field}.${value}`, value),
      ),
    /** The name of a kind of event. */
    eventLabel: (type: string, kind: string): string =>
      copy(type)?.events[kind] ?? named(`decoder.${type}.event.${kind}`, kind),
    /** The label of an event's field. */
    eventFieldLabel: (type: string, field: string): string =>
      copy(type)?.eventFields[field] ?? named(`decoder.${type}.eventField.${field}`, field),
    /** The name of a category: bus, signal line or device. */
    categoryName: (category: string): string => named(`decoder.category.${category}`, category),
  }
}
