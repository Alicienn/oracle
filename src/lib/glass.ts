/**
 * The runtime half of the glass system.
 *
 * Three jobs: inject the SVG filters the CSS refers to, drive the pointer-tracking specular
 * highlight, and drop the expensive layer when the machine cannot keep up.
 */

import { onFrame } from "./dom";
import type { GlassLevel, Theme } from "./api";

/**
 * The refraction filters.
 *
 * `feTurbulence` generates a smooth noise field; `feDisplacementMap` then uses its red and
 * green channels to push each backdrop pixel sideways. That displacement is the whole point:
 * it is what `backdrop-filter: blur()` cannot do, and what separates glass from frost.
 *
 * The blur between them softens the noise so the distortion reads as a gentle optical bend
 * rather than static. Scale rises with the size of the element the filter is used on —
 * small floating chrome bends light more visibly than a large flat panel.
 */
const FILTERS = `
<svg id="lg-filters" aria-hidden="true" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <filter id="lg-refract-sm" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.013 0.017" numOctaves="2" seed="11" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="2.4" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="7" xChannelSelector="R" yChannelSelector="G"/>
    </filter>

    <filter id="lg-refract-md" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.008 0.011" numOctaves="2" seed="7" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="3" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="12" xChannelSelector="R" yChannelSelector="G"/>
    </filter>

    <filter id="lg-refract-lg" x="-12%" y="-12%" width="124%" height="124%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.006 0.009" numOctaves="3" seed="3" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="4" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="20" xChannelSelector="R" yChannelSelector="G"/>
    </filter>
  </defs>
</svg>`;

let currentLevel: GlassLevel = "full";

export function installGlass(): void {
  if (document.getElementById("lg-filters")) return;

  const host = document.createElement("div");
  host.innerHTML = FILTERS;
  const node = host.firstElementChild;
  if (node) document.body.appendChild(node);

  trackPointer();
  suppressDuringGestures();
}

/** Applies the user's chosen intensity. */
export function setGlassLevel(level: GlassLevel): void {
  currentLevel = level;
  document.documentElement.dataset.glass = level;
}

export function setTheme(theme: Theme): void {
  if (theme === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = theme;
  }
}

/**
 * Writes the pointer position into the element under the cursor.
 *
 * One delegated listener on the document rather than one per card: with dozens of glass
 * surfaces, per-element listeners would cost more than the effect is worth. Writes are
 * coalesced to one per frame.
 */
function trackPointer(): void {
  const update = onFrame((event: PointerEvent) => {
    const target = (event.target as Element | null)?.closest?.(".glass--live");
    if (!(target instanceof HTMLElement)) return;

    const box = target.getBoundingClientRect();
    target.style.setProperty("--mx", `${((event.clientX - box.left) / box.width) * 100}%`);
    target.style.setProperty("--my", `${((event.clientY - box.top) / box.height) * 100}%`);
  });

  document.addEventListener("pointermove", update, { passive: true });
}

/**
 * Turns refraction off while the window is being resized or dragged.
 *
 * Recomputing a displacement map every frame during a resize is the one thing that reliably
 * drops this UI below 60 fps. The attribute is removed shortly after the gesture stops.
 */
function suppressDuringGestures(): void {
  let timer: number | undefined;

  const begin = () => {
    document.documentElement.dataset.gesture = "active";
    if (timer !== undefined) clearTimeout(timer);
    timer = window.setTimeout(() => {
      delete document.documentElement.dataset.gesture;
    }, 220);
  };

  window.addEventListener("resize", begin, { passive: true });
  document.addEventListener("pointerdown", (event) => {
    // Only a drag on the title bar moves the window.
    if ((event.target as Element | null)?.closest?.("[data-drag-region]")) begin();
  });
}

/**
 * Measures the frame budget over a short window and steps the glass down if the machine
 * cannot sustain it.
 *
 * Returns the level actually in force, so the caller can tell the user why the setting they
 * chose is not the one being rendered.
 */
export function probePerformance(requested: GlassLevel): Promise<GlassLevel> {
  if (requested !== "full") {
    setGlassLevel(requested);
    return Promise.resolve(requested);
  }

  return new Promise((resolve) => {
    let frames = 0;
    const started = performance.now();

    const tick = () => {
      frames += 1;
      const elapsed = performance.now() - started;

      if (elapsed < 600) {
        requestAnimationFrame(tick);
        return;
      }

      const fps = (frames / elapsed) * 1000;
      // Below this the refraction filter is costing more than it contributes.
      const level: GlassLevel = fps < 50 ? "reduced" : "full";
      setGlassLevel(level);
      resolve(level);
    };

    requestAnimationFrame(tick);
  });
}

export function glassLevel(): GlassLevel {
  return currentLevel;
}
