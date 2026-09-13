/**
 * The icon set.
 *
 * Inline SVG strings rather than an icon font or a sprite sheet: there are few enough of
 * them that the bytes are negligible, and `currentColor` means they inherit theme colours
 * without any extra plumbing.
 */

/**
 * The Oracle mark.
 *
 * Reconstructed the way the artwork was made: three overlapping circles fused by a gooey
 * filter — a heavy blur followed by an alpha ramp that snaps the soft edges back to hard
 * ones, which is what produces the metaball necks between the lobes. A mask punches the
 * central hole, and the fourth circle stays separate, forever about to join.
 *
 * Filter and mask ids must be unique per instance: SVG ids are global to the document, and
 * a second copy would otherwise silently reuse the first one's definitions.
 */
let markInstances = 0;

export function brandMark(size = 20): string {
  const id = `om${markInstances++}`;

  return `
<svg viewBox="0 0 100 100" width="${size}" height="${size}" fill="none" aria-hidden="true">
  <defs>
    <filter id="${id}-goo" x="-35%" y="-35%" width="170%" height="170%" color-interpolation-filters="sRGB">
      <feGaussianBlur in="SourceGraphic" stdDeviation="4.6" result="blur"/>
      <feColorMatrix in="blur" type="matrix"
        values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 26 -12"/>
    </filter>
    <mask id="${id}-mask">
      <rect width="100" height="100" fill="black"/>
      <g filter="url(#${id}-goo)" fill="white">
        <circle cx="50" cy="55" r="21"/>
        <circle cx="27" cy="27" r="12.5"/>
        <circle cx="51" cy="85" r="10.5"/>
      </g>
      <circle cx="50" cy="52" r="7.6" fill="black"/>
    </mask>
  </defs>
  <rect width="100" height="100" fill="currentColor" mask="url(#${id}-mask)"/>
  <circle cx="80" cy="28" r="10" fill="currentColor"/>
</svg>`;
}

const stroke = (body: string, viewBox = "0 0 16 16") =>
  `<svg viewBox="${viewBox}" fill="none" stroke="currentColor" stroke-width="1.5"
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

const filled = (body: string, viewBox = "0 0 16 16") =>
  `<svg viewBox="${viewBox}" fill="currentColor" aria-hidden="true">${body}</svg>`;

export const icons = {
  play: filled(`<path d="M5 3.2a.6.6 0 0 1 .92-.5l6.3 4.3a.6.6 0 0 1 0 1l-6.3 4.3A.6.6 0 0 1 5 11.8z"/>`),
  stop: filled(`<rect x="4" y="4" width="8" height="8" rx="1.6"/>`),
  restart: stroke(`<path d="M13.5 8a5.5 5.5 0 1 1-1.7-3.96"/><path d="M13.6 2.4v3h-3"/>`),

  search: stroke(`<circle cx="7.2" cy="7.2" r="4.4"/><path d="m10.6 10.6 3 3"/>`),
  plus: stroke(`<path d="M8 3.4v9.2M3.4 8h9.2"/>`),
  close: stroke(`<path d="m4 4 8 8M12 4l-8 8"/>`),
  minimise: stroke(`<path d="M3.5 8h9"/>`),
  maximise: stroke(`<rect x="3.6" y="3.6" width="8.8" height="8.8" rx="1.6"/>`),
  chevron: stroke(`<path d="m6 3.5 4.5 4.5L6 12.5"/>`),

  // A gear drawn from its geometry rather than a copied path: a hub, a rim, and eight
  // teeth on 45° spokes. Every coordinate is derived, so nothing lands outside the viewBox
  // and gets clipped the way a hand-tweaked path did.
  settings: stroke(
    `<circle cx="8" cy="8" r="2"/>` +
      `<circle cx="8" cy="8" r="4.6"/>` +
      `<path d="M12.6 8h1.6M11.25 11.25l1.13 1.13M8 12.6v1.6M4.75 11.25l-1.13 1.13` +
      `M3.4 8H1.8M4.75 4.75 3.62 3.62M8 3.4V1.8M11.25 4.75l1.13-1.13"/>`,
  ),

  folder: stroke(`<path d="M2 4.6A1.6 1.6 0 0 1 3.6 3h2.3l1.4 1.7h5.1A1.6 1.6 0 0 1 14 6.3v5.1A1.6 1.6 0 0 1 12.4 13H3.6A1.6 1.6 0 0 1 2 11.4z"/>`),
  external: stroke(`<path d="M9.3 2.6H13.4v4.1"/><path d="m13.4 2.6-6 6"/><path d="M12.2 9.6v2.8a1.4 1.4 0 0 1-1.4 1.4H3.6a1.4 1.4 0 0 1-1.4-1.4V5.2a1.4 1.4 0 0 1 1.4-1.4h2.8"/>`),
  trash: stroke(`<path d="M2.8 4.3h10.4M6.2 4.3V3.1a.9.9 0 0 1 .9-.9h1.8a.9.9 0 0 1 .9.9v1.2M12 4.3v8.3a1.2 1.2 0 0 1-1.2 1.2H5.2A1.2 1.2 0 0 1 4 12.6V4.3"/>`),
  edit: stroke(`<path d="M8.5 2.9H3.4A1.4 1.4 0 0 0 2 4.3v8.3A1.4 1.4 0 0 0 3.4 14h8.3a1.4 1.4 0 0 0 1.4-1.4V7.5"/><path d="M11.9 1.9a1.5 1.5 0 0 1 2.2 2.1L8 10.1 5.2 10.8l.7-2.8z"/>`),
  copy: stroke(`<rect x="5.4" y="5.4" width="8.2" height="8.2" rx="1.4"/><path d="M2.4 10.6V3.8a1.4 1.4 0 0 1 1.4-1.4h6.8"/>`),
  star: stroke(`<path d="m8 1.9 1.88 3.8 4.2.62-3.04 2.96.72 4.18L8 11.5l-3.76 1.96.72-4.18L1.92 6.32l4.2-.62z"/>`),

  branch: stroke(`<circle cx="4.3" cy="3.6" r="1.6"/><circle cx="4.3" cy="12.4" r="1.6"/><circle cx="11.7" cy="3.6" r="1.6"/><path d="M4.3 5.2v5.6M11.7 5.2v1.2a2.6 2.6 0 0 1-2.6 2.6H6.9"/>`),
  cloud: stroke(`<path d="M4.4 12.4a2.9 2.9 0 0 1-.3-5.78 4 4 0 0 1 7.72-.6A2.9 2.9 0 0 1 11.6 12.4z"/>`),
  terminal: stroke(`<path d="m3.4 4.6 3 3.4-3 3.4M8 11.4h4.6"/>`),
  activity: stroke(`<path d="M1.9 8h2.8l1.9-5 2.8 10 1.9-5h2.8"/>`),
  scan: stroke(`<path d="M2.4 5.6V3.8a1.4 1.4 0 0 1 1.4-1.4h1.8M10.4 2.4h1.8a1.4 1.4 0 0 1 1.4 1.4v1.8M13.6 10.4v1.8a1.4 1.4 0 0 1-1.4 1.4h-1.8M5.6 13.6H3.8a1.4 1.4 0 0 1-1.4-1.4v-1.8"/><path d="M2.4 8h11.2"/>`),
  list: stroke(`<path d="M5.6 4h8M5.6 8h8M5.6 12h8M2.6 4h.01M2.6 8h.01M2.6 12h.01"/>`),
  grid: stroke(`<rect x="2.4" y="2.4" width="4.6" height="4.6" rx="1.2"/><rect x="9" y="2.4" width="4.6" height="4.6" rx="1.2"/><rect x="2.4" y="9" width="4.6" height="4.6" rx="1.2"/><rect x="9" y="9" width="4.6" height="4.6" rx="1.2"/>`),
  sun: stroke(`<circle cx="8" cy="8" r="3"/><path d="M8 1.4v1.4M8 13.2v1.4M3.34 3.34l1 1M11.66 11.66l1 1M1.4 8h1.4M13.2 8h1.4M3.34 12.66l1-1M11.66 4.34l1-1"/>`),
  moon: stroke(`<path d="M13.4 9.1A5.8 5.8 0 1 1 6.9 2.6a4.5 4.5 0 0 0 6.5 6.5z"/>`),
  alert: stroke(`<path d="M8 5.4v3.2M8 11.2h.01"/><circle cx="8" cy="8" r="6.1"/>`),
  check: stroke(`<path d="m3.2 8.4 3.2 3.2 6.4-7.2"/>`),
  info: stroke(`<circle cx="8" cy="8" r="6.1"/><path d="M8 7.4v3.6M8 5.2h.01"/>`),
  power: stroke(`<path d="M8 2.4v5.2"/><path d="M12.2 4.6a5.6 5.6 0 1 1-8.4 0"/>`),
} as const;

export type IconName = keyof typeof icons;
