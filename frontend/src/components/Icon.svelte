<script lang="ts" module>
  export type IconName =
    | 'plus'
    | 'settings'
    | 'close'
    | 'search'
    | 'trash'
    | 'external'
    | 'file'
    | 'link'
    | 'layout'
    | 'check'
    | 'text'
    | 'code'
    | 'image'
    | 'globe'
</script>

<script lang="ts">
  // Small line icons (24px grid, 1.75 stroke), so actions can be icon-first with a tooltip.
  let { name, size = 16 }: { name: IconName; size?: number } = $props()

  const paths: Record<IconName, string> = {
    plus: 'M12 5v14M5 12h14',
    settings:
      'M4 7h10M18 7h2M4 17h2M10 17h10M16 4v6M8 14v6',
    close: 'M6 6l12 12M18 6L6 18',
    search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4',
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
