/**
 * Transitions for regions that are rebuilt wholesale.
 *
 * The renderers here replace their contents in one pass, which is what keeps them simple —
 * and what leaves nothing to animate. These helpers wrap a render so the difference between
 * before and after can be played out, without asking any component to track its own nodes.
 *
 * Everything is driven with the Web Animations API on the handful of elements that actually
 * move. The View Transitions API would give the same morph for less code, but it snapshots
 * the entire document: with dozens of `backdrop-filter` surfaces that is both expensive and
 * visually wrong, since the glass would sample a frozen image of itself.
 */

import { glassLevel } from "./glass";

/** How long an exit or entrance runs. Matches `--t-page`. */
const DURATION = 260;

/** Delay added per card on entry, and the number of cards that still get one. */
const STAGGER = 18;
const STAGGER_LIMIT = 8;

const EASE = "cubic-bezier(0.32, 0.72, 0, 1)";

/** Cards move by less than this many pixels are not worth animating. */
const FLIP_THRESHOLD = 1;

/**
 * Whether motion should play at all.
 *
 * Two independent reasons to skip it: the user asked for less motion, or the frame-rate
 * probe in `glass.ts` already decided this machine cannot afford the full treatment. In
 * both cases the render still happens — only the animation is dropped.
 */
function motionAllowed(): boolean {
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return false;
  return glassLevel() === "full";
}

interface Box {
  top: number;
  left: number;
}

/**
 * Renders a list of keyed children, animating the difference.
 *
 * `render` must repopulate `host` and is always called exactly once, whether or not the
 * animation runs. Children are matched across the render by `data-key`, so a project that
 * both filters admit is recognised as the same card and slides rather than being destroyed
 * and rebuilt in place.
 */
export function fluidList(host: HTMLElement, render: () => void): void {
  if (!motionAllowed()) {
    render();
    return;
  }

  const before = new Map<string, Box>();
  const leaving: { node: HTMLElement; box: Box }[] = [];

  for (const child of Array.from(host.children)) {
    if (!(child instanceof HTMLElement)) continue;
    const key = child.dataset.key;
    if (!key) continue;

    const rect = child.getBoundingClientRect();
    before.set(key, { top: rect.top, left: rect.left });
    leaving.push({ node: child, box: { top: rect.top, left: rect.left } });
  }

  render();

  const surviving = new Set<string>();
  let entering = 0;

  for (const child of Array.from(host.children)) {
    if (!(child instanceof HTMLElement)) continue;
    const key = child.dataset.key;
    if (!key) continue;

    const previous = before.get(key);
    if (previous) {
      surviving.add(key);
      flip(child, previous);
    } else {
      // Staggered by position among the arrivals, not among all children: one new card
      // between eight old ones should appear at once, not after eight steps of delay.
      enter(child, entering);
      entering += 1;
    }
  }

  // A node whose key is gone from the new render is the one that needs an exit. It is
  // already detached, so it is re-parented into an overlay: animating it in place would
  // mean putting it back into a layout it no longer belongs to.
  const departed = leaving.filter(({ node }) => {
    const key = node.dataset.key;
    return key ? !surviving.has(key) : false;
  });

  if (departed.length > 0) exit(host, departed);
}

/** Plays a node from its previous position to the one it now occupies. */
function flip(node: HTMLElement, previous: Box): void {
  const rect = node.getBoundingClientRect();
  const dx = previous.left - rect.left;
  const dy = previous.top - rect.top;

  if (Math.abs(dx) < FLIP_THRESHOLD && Math.abs(dy) < FLIP_THRESHOLD) return;

  node.animate(
    [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "translate(0, 0)" }],
    { duration: DURATION, easing: EASE },
  );
}

/**
 * Fades a new node in from slightly below and out of focus.
 *
 * The blur is what makes this read as glass reforming rather than a list item sliding in.
 */
function enter(node: HTMLElement, index: number): void {
  node.animate(
    [
      { opacity: 0, transform: "translateY(8px) scale(0.985)", filter: "blur(6px)" },
      { opacity: 1, transform: "translateY(0) scale(1)", filter: "blur(0)" },
    ],
    {
      duration: DURATION,
      delay: Math.min(index, STAGGER_LIMIT) * STAGGER,
      easing: EASE,
      fill: "backwards",
    },
  );
}

/**
 * Plays departing nodes out, over a copy of the list.
 *
 * The overlay is positioned and sized to the host and takes no pointer events, so the new
 * list underneath is live and clickable while the old one is still fading.
 */
function exit(host: HTMLElement, departed: { node: HTMLElement; box: Box }[]): void {
  const hostBox = host.getBoundingClientRect();

  const overlay = document.createElement("div");
  overlay.setAttribute("aria-hidden", "true");
  Object.assign(overlay.style, {
    position: "fixed",
    top: `${hostBox.top}px`,
    left: `${hostBox.left}px`,
    width: `${hostBox.width}px`,
    height: `${hostBox.height}px`,
    pointerEvents: "none",
    overflow: "hidden",
    zIndex: "1",
  });

  let last: Animation | null = null;

  for (const { node, box } of departed) {
    const width = node.offsetWidth;
    const height = node.offsetHeight;

    Object.assign(node.style, {
      position: "absolute",
      top: `${box.top - hostBox.top}px`,
      left: `${box.left - hostBox.left}px`,
      width: `${width}px`,
      height: `${height}px`,
      margin: "0",
    });

    overlay.appendChild(node);

    last = node.animate(
      [
        { opacity: 1, transform: "scale(1)", filter: "blur(0)" },
        { opacity: 0, transform: "scale(0.96)", filter: "blur(6px)" },
      ],
      { duration: DURATION, easing: EASE, fill: "forwards" },
    );
  }

  // Parented to the body rather than next to the host: `position: fixed` resolves against
  // whichever ancestor establishes a containing block, and a `transform` or `filter` added
  // to a wrapper later would silently start misplacing the whole overlay. The body cannot
  // acquire one without breaking the window chrome, so it is the one safe anchor.
  document.body.appendChild(overlay);

  // One listener on the last animation rather than a counter: they all start together and
  // share a duration, so the last to be created is the last to finish.
  const done = () => overlay.remove();
  if (last) {
    last.addEventListener("finish", done);
    last.addEventListener("cancel", done);
  } else {
    done();
  }
}

/**
 * Adds a highlight that slides between the pressed options of a segmented control.
 *
 * The indicator is a child of the control rather than a border on the button, because only
 * a single moving element can travel between them. Position is read from the pressed
 * button's own box, so the control keeps owning its layout.
 *
 * Safe to call on every render: the indicator is reused when it already exists, which is
 * what lets it animate from where it currently is to where it now belongs.
 */
export function slidingPill(control: HTMLElement): void {
  const active = control.querySelector<HTMLElement>('[aria-pressed="true"]');
  if (!active) return;

  let pill = control.querySelector<HTMLElement>(".pill");
  const fresh = pill === null;

  if (!pill) {
    pill = document.createElement("span");
    pill.className = "pill";
    pill.setAttribute("aria-hidden", "true");
    control.prepend(pill);
  }

  const target = {
    left: active.offsetLeft,
    top: active.offsetTop,
    width: active.offsetWidth,
    height: active.offsetHeight,
  };

  // The first placement is not an animation: there is no previous position to come from,
  // and sliding in from the corner on load would be noise.
  if (fresh || !motionAllowed()) {
    Object.assign(pill.style, {
      transform: `translate(${target.left}px, ${target.top}px)`,
      width: `${target.width}px`,
      height: `${target.height}px`,
    });
    return;
  }

  const from = pill.style.transform;
  const to = `translate(${target.left}px, ${target.top}px)`;

  Object.assign(pill.style, {
    transform: to,
    width: `${target.width}px`,
    height: `${target.height}px`,
  });

  if (from && from !== to) {
    pill.animate([{ transform: from }, { transform: to }], {
      duration: DURATION,
      easing: EASE,
    });
  }
}
