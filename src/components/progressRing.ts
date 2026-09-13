/**
 * A ring that fills as work progresses, then becomes a tick.
 *
 * One element for the whole sequence rather than a bar swapped for an icon: the completed
 * ring is already the shape the tick sits inside, so finishing reads as the same object
 * arriving rather than two states cutting between each other.
 *
 * Progress is driven by `stroke-dashoffset`, which the browser can interpolate, so setting
 * a fraction animates to it instead of jumping. Before the first byte — or when the server
 * never declared a length — the ring has nothing to measure and spins instead.
 */

const SIZE = 72;
const RADIUS = 30;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** Drawn inside the ring, and long enough that the dash covers the whole stroke. */
const TICK = "M25 37.5 L33 45.5 L48 28";
const TICK_LENGTH = 40;

export interface Ring {
  element: SVGElement;
  /** A fraction in 0..1, or `null` while there is nothing to measure. */
  set(fraction: number | null): void;
  /** Completes the ring and draws the tick. */
  succeed(): void;
}

export function progressRing(): Ring {
  const node = document.createElementNS("http://www.w3.org/2000/svg", "svg");

  node.setAttribute("class", "ring");
  node.setAttribute("viewBox", `0 0 ${SIZE} ${SIZE}`);
  node.setAttribute("width", String(SIZE));
  node.setAttribute("height", String(SIZE));
  node.setAttribute("aria-hidden", "true");
  node.dataset.state = "waiting";

  // The arc is rotated so that it grows from twelve o'clock, where a reader expects a dial
  // to start, rather than from three.
  node.innerHTML = `
    <circle class="ring__track" cx="${SIZE / 2}" cy="${SIZE / 2}" r="${RADIUS}"/>
    <circle class="ring__arc" cx="${SIZE / 2}" cy="${SIZE / 2}" r="${RADIUS}"
      transform="rotate(-90 ${SIZE / 2} ${SIZE / 2})"
      stroke-dasharray="${CIRCUMFERENCE.toFixed(2)}"
      stroke-dashoffset="${CIRCUMFERENCE.toFixed(2)}"/>
    <path class="ring__tick" d="${TICK}"
      stroke-dasharray="${TICK_LENGTH}" stroke-dashoffset="${TICK_LENGTH}"/>
  `;

  const arc = node.querySelector<SVGCircleElement>(".ring__arc");

  const set = (fraction: number | null) => {
    if (!arc) return;

    if (fraction === null) {
      node.dataset.state = "waiting";
      return;
    }

    node.dataset.state = "working";
    const clamped = Math.min(1, Math.max(0, fraction));
    arc.setAttribute("stroke-dashoffset", (CIRCUMFERENCE * (1 - clamped)).toFixed(2));
  };

  const succeed = () => {
    // The ring closes first and the tick follows, which the CSS delays: a tick appearing
    // over a half-empty ring would contradict it.
    arc?.setAttribute("stroke-dashoffset", "0");
    node.dataset.state = "done";
  };

  return { element: node, set, succeed };
}
