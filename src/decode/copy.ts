/**
 * The copy a loaded decoder module carries (DESIGN.md §11): what the interface shows for it,
 * in the language closest to the interface's; it comes before the interface's own copy for a
 * decoder of that name, and the identifiers themselves come last.
 */
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { CatalogLocale } from '@/ipc/bindings/CatalogLocale'

/** The language of a tag: `zh` of `zh-CN`. */
const language = (tag: string): string => tag.split('-')[0]?.toLowerCase() ?? ''

/**
 * The copy of `type` for `locale`: under the tag itself, else under another tag of the same
 * language, else under the first tag the module declares; nothing for a type without copy.
 */
export function moduleLocale(
  type: CatalogEntry | undefined,
  locale: string,
): CatalogLocale | undefined {
  const locales = type?.locales
  if (!locales) return undefined
  const tags = Object.keys(locales)
  const tag =
    tags.find((one) => one === locale) ??
    tags.find((one) => language(one) === language(locale)) ??
    tags[0]
  return tag === undefined ? undefined : locales[tag]
}
