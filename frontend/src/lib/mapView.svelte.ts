// How the map is shown, remembered on this computer: what the dots' colours mean, what their size
// means, and how much stays bright around the picked or hovered item.

export interface MapView {
  /** `type`: text, code, link… `topic`: the colour of the topic's region. `age`: when it was added. */
  colorBy: 'type' | 'topic' | 'age'
  /** `links`: items with more links are bigger. `same`: all alike. */
  sizeBy: 'links' | 'same'
  /** How many steps of links around the picked or hovered item stay bright. */
  reach: 1 | 2
  /** Hide everything outside that reach of the picked item, instead of dimming it. */
  hideRest: boolean
}

export const VIEW_DEFAULT: MapView = { colorBy: 'type', sizeBy: 'links', reach: 1, hideRest: false }

const KEY = 'plexus.mapView'

function read(): MapView {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
    return {
      colorBy: ['type', 'topic', 'age'].includes(saved.colorBy) ? saved.colorBy : VIEW_DEFAULT.colorBy,
      sizeBy: ['links', 'same'].includes(saved.sizeBy) ? saved.sizeBy : VIEW_DEFAULT.sizeBy,
      reach: saved.reach === 2 ? 2 : 1,
      hideRest: saved.hideRest === true,
    }
  } catch {
    return { ...VIEW_DEFAULT }
  }
}

export const mapView = $state<MapView>(read())

export function setMapView(patch: Partial<MapView>) {
  Object.assign(mapView, patch)
  try {
    localStorage.setItem(KEY, JSON.stringify($state.snapshot(mapView)))
  } catch {}
}
