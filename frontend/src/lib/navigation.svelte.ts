// How the map reacts to scrolling, remembered on this computer.

export interface Navigation {
  /** Which way scrolling zooms. Off: scrolling up zooms in, as a mouse wheel does. On (the
   * default): sliding two fingers up the trackpad zooms in, which with natural scrolling is the
   * wheel's opposite. */
  invertZoom: boolean
  /** Experimental: the view drifts towards the cursor, faster the further it is from the middle. */
  edgeScroll: boolean
}

const KEY = 'plexus.navigation'

function read(): Navigation {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return { invertZoom: saved.invertZoom !== false, edgeScroll: saved.edgeScroll !== false }
  } catch {
    return { invertZoom: true, edgeScroll: true }
  }
}

export const navigation = $state<Navigation>(read())

export function setNavigation(patch: Partial<Navigation>) {
  Object.assign(navigation, patch)
  try {
    localStorage.setItem(KEY, JSON.stringify($state.snapshot(navigation)))
  } catch {}
}
