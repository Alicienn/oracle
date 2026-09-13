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
  const tile = h("span", {
    class: "icon-tile",
    style: { width: `${size}px`, height: `${size}px`, background: project.resolvedAccent },
  });

  if (project.icon.type === "file") {
    const image = h("img", {
      src: convertFileSrc(project.icon.value),
      alt: "",
      loading: "lazy",
    });
    // A missing or unreadable file falls back to initials rather than a broken image.
    image.addEventListener("error", () => {
      image.remove();
      tile.textContent = initials(project.name);
    });
    tile.appendChild(image);
  } else if (project.icon.type === "favicon") {
    const image = h("img", { src: project.icon.value, alt: "", loading: "lazy" });
    image.addEventListener("error", () => {
      image.remove();
      tile.textContent = initials(project.name);
    });
    tile.appendChild(image);
  } else {
    tile.textContent = initials(project.name);
    tile.style.fontSize = `${Math.round(size * 0.4)}px`;
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

interface ButtonOptions {
  label?: string;
  title?: string;
  variant?: "primary" | "danger" | "ghost";
  iconName?: IconName;
  disabled?: boolean;
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
      onClick: (event: Event) => {
        event.stopPropagation();
        options.onClick(event as MouseEvent);
      },
    },
    options.iconName && icon(icons[options.iconName]),
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

/** A segmented control. Returns the element; selection is driven by `value`. */
export function segmented<T extends string>(
  options: { value: T; label: string; iconName?: IconName; title?: string }[],
  value: T,
  onChange: (next: T) => void,
): HTMLElement {
  return h(
    "div",
    { class: "filters", role: "group" },
    ...options.map((option) =>
      h(
        "button",
        {
          type: "button",
          "aria-pressed": String(option.value === value),
          title: option.title ?? option.label,
          onClick: () => onChange(option.value),
        },
        option.iconName && icon(icons[option.iconName]),
        option.label,
      ),
    ),
  );
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
