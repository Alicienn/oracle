/** Shared building blocks used by both the main window and the tray panel. */

import { convertFileSrc } from "@tauri-apps/api/core";
import type { ProjectStatus, ProjectView, RemoteStatus } from "../lib/api";
import { h, icon } from "../lib/dom";
import { initials, sparkPath } from "../lib/format";
import { icons, type IconName } from "../lib/icons";

/** The dot that carries a project's local status. */
export function statusDot(status: ProjectStatus): HTMLElement {
  return h("span", {
    class: "dot",
    dataset: { status },
    title: statusLabel(status),
    "aria-label": statusLabel(status),
  });
}

export function statusLabel(status: ProjectStatus): string {
  switch (status) {
    case "running":
      return "Running";
    case "starting":
      return "Starting";
    case "unhealthy":
      return "Not responding";
    case "crashed":
      return "Crashed";
    default:
      return "Stopped";
  }
}

/**
 * Whether the runner is holding this project, and the stop button is the one to show.
 *
 * Not the same question as "is it serving". `unhealthy` is a live process whose port never
 * answered — stopping it is the only thing left to do with it, and treating it as idle made
 * the button try to start it again and collect an "already running" error. A pending start
 * counts too: the request is in flight, so the way out is to stop it.
 *
 * `crashed` is deliberately absent. The process is gone, so the button should offer a fresh
 * start; the backend clears the dead handle when one arrives.
 */
export function isLive(status: ProjectStatus, pending?: { kind: "start" | "stop" }): boolean {
  return (
    status === "running" ||
    status === "starting" ||
    status === "unhealthy" ||
    pending?.kind === "start"
  );
}

/**
 * What a button says while its operation is in flight.
 *
 * A pending start spells out that it can still be abandoned: the button stays live, and a
 * spinner on its own does not tell anyone that clicking it now stops the project.
 */
export function pendingLabel(kind: "start" | "stop", name: string): string {
  return kind === "start" ? `Starting ${name}… — click to stop` : `Stopping ${name}…`;
}

export function remoteLabel(status: RemoteStatus): string {
  switch (status.state) {
    case "up":
      return `Up · ${status.ms} ms`;
    case "degraded":
      return `HTTP ${status.code} · ${status.ms} ms`;
    case "down":
      return status.reason;
    default:
      return "Not checked yet";
  }
}

/** Maps a remote status onto the same dot vocabulary as a local one. */
export function remoteDot(status: RemoteStatus): HTMLElement {
  const tone =
    status.state === "up" ? "up" : status.state === "unchecked" ? "stopped" : "down";

  return h("span", {
    class: "dot",
    dataset: { status: tone },
    title: remoteLabel(status),
    "aria-label": remoteLabel(status),
  });
}

/**
 * The square that stands in for a project.
 *
 * A user-supplied image when there is one, otherwise the project's initials on its accent
 * colour — which is enough to tell a rail of eight projects apart at a glance.
 */
export function iconTile(project: ProjectView, size = 28): HTMLElement {
  // The resolved icon, not the stored one: a remote project with no icon of its own is
  // drawn with the favicon the backend downloaded for it, which arrives here as an ordinary
  // local file.
  const source = project.resolvedIcon ?? project.icon;
  const tile = h("span", {
    class: "icon-tile",
    style: { width: `${size}px`, height: `${size}px`, background: project.resolvedAccent },
  });

  const lettering = () => {
    tile.textContent = initials(project.name);
    tile.style.fontSize = `${Math.round(size * 0.4)}px`;
  };

  if (source.type === "file") {
    const image = h("img", { src: convertFileSrc(source.value), alt: "" });
    // A missing or unreadable file falls back to initials rather than a broken image.
    image.addEventListener("error", () => {
      image.remove();
      lettering();
    });
    tile.appendChild(image);
  } else {
    lettering();
  }

  return tile;
}

/** A small line chart with no axes, for CPU history in a card. */
export function sparkline(values: number[], width = 58, height = 20): SVGElement | null {
  if (values.length < 2) return null;

  const { line, area } = sparkPath(values, width, height);
  const node = document.createElementNS("http://www.w3.org/2000/svg", "svg");

  node.setAttribute("class", "spark");
  node.setAttribute("viewBox", `0 0 ${width} ${height}`);
  node.setAttribute("aria-hidden", "true");
  node.innerHTML = `<path class="spark__fill" d="${area}"/><path d="${line}"/>`;

  return node;
}

/** Animation periods in components.css, needed here to compute the phase offsets. */
const SPIN_MS = 900;
const ARC_MS = 1400;
const APPEAR_MS = 180;

/**
 * An indeterminate progress ring, sized to sit where an icon would.
 *
 * Built as an arc rather than a rotating icon so it reads as "working" instead of "this
 * glyph is spinning": the dash pattern leaves a gap that travels round the circle, which is
 * the convention every platform uses for an operation of unknown length.
 *
 * `since` is when the operation began. Because every region is rebuilt on each state change
 * — once a second at minimum, from the metrics tick — a spinner node lives only a frame or
 * two, and a fresh CSS animation would restart from zero each time: a ring that twitches
 * instead of turning. Negative animation delays place each new node at the phase the
 * animation would have reached, so the motion is continuous no matter how often the node
 * behind it is replaced. The same trick skips the entrance animation on every node but the
 * first, which would otherwise pulse once a second.
 */
export function spinner(since: number, size = 16): SVGElement {
  const node = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const elapsed = Math.max(0, performance.now() - since);

  node.setAttribute("class", "spinner");
  node.setAttribute("viewBox", "0 0 16 16");
  node.setAttribute("width", String(size));
  node.setAttribute("height", String(size));
  node.setAttribute("aria-hidden", "true");
  node.innerHTML =
    '<circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>';

  node.style.animationDelay = `-${elapsed % SPIN_MS}ms, -${Math.min(elapsed, APPEAR_MS)}ms`;

  const arc = node.firstElementChild as SVGElement | null;
  if (arc) arc.style.animationDelay = `-${elapsed % ARC_MS}ms`;

  return node;
}

/**
 * Makes a container behave like a button without being one.
 *
 * Cards and panel rows hold their own buttons — play, stop, open — and a `<button>` may not
 * contain interactive content. The invalid nesting was not harmless: clicking the play
 * button also activated the card underneath it, selecting the project, because the outer
 * button's activation does not care that the inner one stopped the event from bubbling.
 *
 * `ignoreWithin` is a selector for the region that owns its own controls. Clicks landing
 * there — on a button, in the gap between two, or on the sparkline — are the action area's
 * business, never a selection.
 */
export function activatable(
  element: HTMLElement,
  onActivate: () => void,
  ignoreWithin?: string,
): HTMLElement {
  element.setAttribute("role", "button");
  element.tabIndex = 0;

  element.addEventListener("click", (event) => {
    const target = event.target;
    if (ignoreWithin && target instanceof Element && target.closest(ignoreWithin)) return;
    onActivate();
  });

  element.addEventListener("keydown", (event) => {
    // Enter and Space are what a real button answers to, and losing them would make the
    // list unusable from the keyboard.
    if (event.key !== "Enter" && event.key !== " ") return;
    if (event.target !== element) return;

    event.preventDefault();
    onActivate();
  });

  return element;
}

interface ButtonOptions {
  label?: string;
  title?: string;
  variant?: "primary" | "danger" | "ghost";
  iconName?: IconName;
  disabled?: boolean;
  /**
   * Shows a spinner in place of the icon. The value is when the operation began, which the
   * spinner needs to stay in phase across re-renders.
   *
   * Deliberately independent of `disabled`. A start can take up to a minute to be answered,
   * and a button that spins *and* refuses clicks for that long leaves no way to abandon a
   * project that is not coming up — the spinner says "working", not "untouchable".
   */
  pending?: number;
  onClick: (event: MouseEvent) => void;
}

export function button(options: ButtonOptions): HTMLButtonElement {
  const classes = ["btn"];
  if (options.variant) classes.push(`btn--${options.variant}`);
  if (!options.label) classes.push("btn--icon");

  const element = h(
    "button",
    {
      class: classes.join(" "),
      type: "button",
      title: options.title ?? options.label ?? "",
      "aria-label": options.title ?? options.label ?? "",
      disabled: options.disabled,
      dataset: { pending: options.pending !== undefined ? "true" : undefined },
      onClick: (event: Event) => {
        event.stopPropagation();
        options.onClick(event as MouseEvent);
      },
    },
    options.pending !== undefined
      ? spinner(options.pending)
      : options.iconName && icon(icons[options.iconName]),
    options.label,
  );

  return element;
}

/** A labelled on/off row, as used throughout settings. */
export function toggleRow(
  title: string,
  description: string,
  checked: boolean,
  onChange: (next: boolean) => void,
): HTMLElement {
  const control = h("button", {
    class: "switch",
    type: "button",
    role: "switch",
    "aria-checked": String(checked),
    "aria-label": title,
  });

  const row = h(
    "div",
    {
      class: "toggle",
      onClick: () => {
        const next = control.getAttribute("aria-checked") !== "true";
        control.setAttribute("aria-checked", String(next));
        onChange(next);
      },
    },
    h(
      "div",
      { class: "toggle__text" },
      h("strong", null, title),
      h("span", null, description),
    ),
    control,
  );

  return row;
}

/**
 * A segmented control. Returns the element; selection is driven by `value`.
 *
 * `pill` hands the pressed-state background to a single sliding element instead of painting
 * it on each button, which is what allows the selection to travel between them. It is opt-in
 * because it only pays off on a control that outlives its renders — one rebuilt each time
 * has no previous position to slide from. A caller that sets it owes the control a
 * `slidingPill` call once it is in the document.
 */
export function segmented<T extends string>(
  options: { value: T; label: string; iconName?: IconName; title?: string }[],
  value: T,
  onChange: (next: T) => void,
  pill = false,
): HTMLElement {
  return h(
    "div",
    { class: pill ? "filters filters--pill" : "filters", role: "group" },
    ...options.map((option) =>
      h(
        "button",
        {
          type: "button",
          "aria-pressed": String(option.value === value),
          title: option.title ?? option.label,
          // Read back by `setSegmentedValue`, which is how a control that is built once
          // keeps up with the state.
          dataset: { value: option.value },
          onClick: () => onChange(option.value),
        },
        option.iconName && icon(icons[option.iconName]),
        option.label,
      ),
    ),
  );
}

/** Moves the pressed state of a `segmented` control without rebuilding it. */
export function setSegmentedValue(control: HTMLElement, value: string): void {
  for (const button of control.querySelectorAll<HTMLElement>("button[data-value]")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === value));
  }
}

/** Swaps the glyph of a button that is built once and updated in place. */
export function setButtonIcon(element: HTMLButtonElement, name: IconName, title: string): void {
  element.querySelector("svg")?.replaceWith(icon(icons[name]));
  element.title = title;
  element.setAttribute("aria-label", title);
}

export function emptyState(
  markup: string,
  title: string,
  message: string,
  action?: HTMLElement,
): HTMLElement {
  const art = document.createElement("div");
  art.innerHTML = markup;

  return h(
    "div",
    { class: "empty" },
    art.firstElementChild ?? icon(icons.folder),
    h("h3", null, title),
    h("p", null, message),
    action,
  );
}
