// Colours for the map's dots when they follow something other than the item's type. Plain hex,
// since the WebGL layer doesn't read every CSS colour notation.

/** Items added long ago to just now: a muted slate towards amber, as a light and a dark theme need. */
export const AGE_COLORS = {
  dark: { old: '#5b6485', recent: '#ffb454' },
  light: { old: '#a3adc8', recent: '#e07a00' },
}

export function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

const byte = (v: number) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')

/** `t` of the way from colour `a` to colour `b` (0 to 1). */
export function mix(a: string, b: string, t: number): string {
  const [ar, ag, ab] = hexToRgb(a)
  const [br, bg, bb] = hexToRgb(b)
  return `#${byte(ar + (br - ar) * t)}${byte(ag + (bg - ag) * t)}${byte(ab + (bb - ab) * t)}`
}

/** Hue in degrees, saturation and lightness in percent. */
export function hsl(h: number, s: number, l: number): string {
  const a = (s / 100) * Math.min(l / 100, 1 - l / 100)
  const f = (n: number) => {
    const k = (n + h / 30) % 12
    return (l / 100 - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))) * 255
  }
  return `#${byte(f(0))}${byte(f(8))}${byte(f(4))}`
}
