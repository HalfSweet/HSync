import type { Directive } from 'vue'

/**
 * `v-indicator` on a segmented control, or `v-indicator:line` on a tab bar (DESIGN.md §13):
 * the selected option's mark, its raised face or the line under it, slides to the next
 * selection instead of jumping there.
 *
 * The directive keeps the selected option's box in `--mark-*` on the container and sets
 * `data-marked` while an option is selected; motion.css draws the mark from them.
 */

/**
 * Options the mark goes under: the container's children that say they are selected, but not
 * one on its way out of a list (`holdInPlace`, src/motion/list.ts).
 */
const SELECTED =
  ':scope > [aria-pressed="true"]:not([data-leaving]), :scope > [aria-selected="true"]:not([data-leaving])'

interface Watchers {
  mutations: MutationObserver
  sizes: ResizeObserver | null
}

const watched = new WeakMap<HTMLElement, Watchers>()

/** Puts the mark under the selected option, or takes it away when none is selected. */
export function placeMark(container: HTMLElement): void {
  const selected = container.querySelector<HTMLElement>(SELECTED)
  if (!selected) {
    delete container.dataset.marked
    return
  }
  const { style } = container
  style.setProperty('--mark-x', `${selected.offsetLeft}px`)
  style.setProperty('--mark-y', `${selected.offsetTop}px`)
  style.setProperty('--mark-w', `${selected.offsetWidth}px`)
  style.setProperty('--mark-h', `${selected.offsetHeight}px`)
  container.dataset.marked = ''
}

export const vIndicator: Directive<HTMLElement> = {
  mounted(container, binding) {
    // Positions the container first, so the options' offsets are taken from it.
    container.dataset.indicator = binding.arg === 'line' ? 'line' : 'face'
    placeMark(container)

    const place = () => placeMark(container)
    // Options change size with the language and once the fonts load.
    const sizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(place)
    const observeOptions = () => {
      if (!sizes) return
      sizes.disconnect()
      sizes.observe(container)
      for (const option of container.children) sizes.observe(option)
    }
    const mutations = new MutationObserver((records) => {
      if (records.some((record) => record.type === 'childList')) observeOptions()
      place()
    })
    mutations.observe(container, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['aria-pressed', 'aria-selected', 'data-leaving'],
    })
    observeOptions()
    watched.set(container, { mutations, sizes })
  },
  unmounted(container) {
    const watchers = watched.get(container)
    watchers?.mutations.disconnect()
    watchers?.sizes?.disconnect()
    watched.delete(container)
  },
}
