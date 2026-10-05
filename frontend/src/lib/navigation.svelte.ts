// How the map reacts to scrolling, remembered on this computer.

export interface Navigation {
  /** Which way scrolling zooms. Off: scrolling up zooms in, as a mouse wheel does. On (the
   * default): sliding two fingers up the trackpad zooms in, which with natural scrolling is the
   * wheel's opposite. */
  invertZoom: boolean
}

const KEY = 'plexus.navigation'

function read(): Navigation {
  try {
    return { invertZoom: JSON.parse(localStorage.getItem(KEY) ?? '{}').invertZoom !== false }
  } catch {
    return { invertZoom: true }
  }
}

export const navigation = $state<Navigation>(read())

export function setNavigation(patch: Partial<Navigation>) {
  Object.assign(navigation, patch)
  try {
    localStorage.setItem(KEY, JSON.stringify($state.snapshot(navigation)))
  } catch {}
}
