import type { RenderTokens } from './types'

function parseHex(color: string): [number, number, number] | null {
  const hex = color.trim().replace(/^#/, '')
  if (!/^[0-9a-f]{6}$/i.test(hex)) return null
  const value = Number.parseInt(hex, 16)
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff]
}

/** An opacity given in percent (`7.5%`) or as a fraction, as a fraction. */
function opacityOf(text: string): number {
  const trimmed = text.trim()
  const value = trimmed.endsWith('%')
    ? Number.parseFloat(trimmed) / 100
    : Number.parseFloat(trimmed)
  return Number.isFinite(value) ? value : 0.1
}

/** `#rrggbb` at an opacity given in percent (`7.5%`) or as a fraction. */
export function withOpacity(color: string, opacity: string): string {
  const rgb = parseHex(color)
  if (!rgb) return color
  return `rgb(${rgb[0]} ${rgb[1]} ${rgb[2]} / ${opacityOf(opacity)})`
}

/** Reads the canvas colours from the CSS custom properties of `root`. */
export function readRenderTokens(root: Element = document.documentElement): RenderTokens {
  const style = getComputedStyle(root)
  const get = (name: string) => style.getPropertyValue(name).trim()
  const wires = Array.from({ length: 8 }, (_, index) => get(`--w${index}`))
  // Byte blocks are `--bk` times as opaque as the wash of their colour.
  const strength = Number.parseFloat(get('--bk'))
  const blockScale = Number.isFinite(strength) ? strength : 3.4
  return {
    grid: get('--grid'),
    wires,
    washes: wires.map((wire, index) => withOpacity(wire, get(`--a${index}`))),
    blocks: wires.map((wire, index) =>
      withOpacity(wire, String(Math.min(1, opacityOf(get(`--a${index}`)) * blockScale))),
    ),
  }
}
