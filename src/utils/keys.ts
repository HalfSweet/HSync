/** Keyboard shortcuts with the platform's command key: ⌘ on Apple systems, Ctrl elsewhere. */

export const APPLE =
  typeof navigator !== 'undefined' &&
  /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)

/** Whether `event` is the command key plus `key` and nothing else. */
export function isCommand(event: KeyboardEvent, key: string, apple = APPLE): boolean {
  const command = apple ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey
  return command && !event.altKey && !event.shiftKey && event.key.toLowerCase() === key
}

/** How menus show the shortcut: `⌘S` or `Ctrl+S`. */
export function commandLabel(key: string, apple = APPLE): string {
  return apple ? `⌘${key.toUpperCase()}` : `Ctrl+${key.toUpperCase()}`
}
