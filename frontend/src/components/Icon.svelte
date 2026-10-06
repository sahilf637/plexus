<script lang="ts" module>
  export type IconName =
    | 'plus'
    | 'settings'
    | 'close'
    | 'search'
    | 'trash'
    | 'edit'
    | 'tree'
    | 'external'
    | 'file'
    | 'link'
    | 'layout'
    | 'check'
    | 'text'
    | 'code'
    | 'image'
    | 'globe'
    | 'clipboard'
    | 'sparkles'
    | 'folder'
    | 'refresh'
    | 'chevron'
    | 'back'
    | 'chevron-down'
    | 'history'
    | 'send'
    | 'stop'
    | 'target'
    | 'copy'
    | 'chart'
    | 'lock'
    | 'keyboard'
    | 'more'
    | 'expand'
    | 'collapse'
    | 'zoom-in'
    | 'zoom-out'
</script>

<script lang="ts">
  // Small line icons (24px grid, 1.75 stroke), so actions can be icon-first with a tooltip.
  let { name, size = 16 }: { name: IconName; size?: number } = $props()

  const paths: Record<IconName, string> = {
    chart: 'M5 20V11M12 20V4M19 20v-6',
    lock: 'M8 11V8a4 4 0 0 1 8 0v3M6 11h12a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1z',
    keyboard: 'M4 6h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM7 10h.01M11 10h.01M15 10h.01M8 14h8',
    more: 'M5 12a1 1 0 1 0 2 0 1 1 0 0 0-2 0zM11 12a1 1 0 1 0 2 0 1 1 0 0 0-2 0zM17 12a1 1 0 1 0 2 0 1 1 0 0 0-2 0z',
    expand: 'M14 4h6v6M10 20H4v-6M20 4l-6.5 6.5M4 20l6.5-6.5',
    collapse: 'M20 10h-6V4M4 14h6v6M14 10l6.5-6.5M10 14l-6.5 6.5',
    'zoom-in': 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4M11 8v6M8 11h6',
    'zoom-out': 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4M8 11h6',
    history: 'M4 12a8 8 0 1 0 2.3-5.7M4 5v4h4M12 8v4l3 2',
    send: 'M5 12l14-7-5 14-2.5-5.5z',
    stop: 'M7 7h10v10H7z',
    copy: 'M9 9h10a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V10a1 1 0 0 1 1-1zM16 9V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3',
    target: 'M12 3v3M12 18v3M3 12h3M18 12h3M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z',
    chevron: 'M9 6l6 6-6 6',
    'chevron-down': 'M6 9l6 6 6-6',
    back: 'M15 6l-6 6 6 6',
    clipboard: 'M9 4h6v3H9zM7 5H6a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-1',
    sparkles:
      'M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8zM18 16l.8 2.2L21 19l-2.2.8L18 22l-.8-2.2L15 19l2.2-.8z',
    folder: 'M3 7a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z',
    refresh: 'M20 11a8 8 0 0 0-14.5-4M4 4v3.5h3.5M4 13a8 8 0 0 0 14.5 4M20 20v-3.5h-3.5',
    plus: 'M12 5v14M5 12h14',
    settings:
      'M4 7h10M18 7h2M4 17h2M10 17h10M16 4v6M8 14v6',
    close: 'M6 6l12 12M18 6L6 18',
    search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
    edit: 'M4 20h4L19 9l-4-4L4 16zM13 7l4 4',
    tree: 'M5 6h14M9 12h10M13 18h6M5 12h.01M9 18h.01',
    trash: 'M4 7h16M10 11v6M14 11v6M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M9 7V4h6v3',
    external: 'M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5',
    file: 'M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5',
    link: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1',
    layout: 'M5 5h6v6H5zM13 13h6v6h-6zM16 5a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM8 13a3 3 0 1 0 0 6 3 3 0 0 0 0-6z',
    check: 'M5 12.5l4.5 4.5L19 7.5',
    text: 'M5 7V5h14v2M12 5v14M9 19h6',
    code: 'M9 8l-5 4 5 4M15 8l5 4-5 4',
    image: 'M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM4 16l5-5 4 4 3-3 4 4M9 8.5h.01',
    globe: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c2.5 2.5 3.5 5.5 3.5 9s-1 6.5-3.5 9c-2.5-2.5-3.5-5.5-3.5-9s1-6.5 3.5-9z',
  }
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name]} />
</svg>
