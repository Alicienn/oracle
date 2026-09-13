/**
 * Toasts.
 *
 * Every backend error carries a short message and an optional detail chain. The message goes
 * in the toast; the detail hides behind a disclosure, so a failed launch shows "could not
 * start the process" without burying the user in a Windows error code they did not ask for —
 * while still keeping it one click away.
 */

import { h, fill, icon } from "./dom";
import { icons } from "./icons";

export type Tone = "info" | "success" | "error";

export interface ToastOptions {
  tone?: Tone;
  title: string;
  message?: string;
  detail?: string | null;
  /** Stays until dismissed. For anything the user must actually read. */
  sticky?: boolean;
}

const LIFETIME = 6000;

let host: HTMLElement | null = null;

function container(): HTMLElement {
  if (host?.isConnected) return host;

  host = h("div", { class: "toasts", role: "status", "aria-live": "polite" });
  document.body.appendChild(host);
  return host;
}

export function toast(options: ToastOptions): void {
  const tone = options.tone ?? "info";
  const glyph = tone === "error" ? icons.alert : tone === "success" ? icons.check : icons.info;

  const element = h(
    "div",
    { class: "toast glass glass--raised", dataset: { tone } },
    h("div", { class: "toast__title" }, icon(glyph), h("span", null, options.title)),
    options.message && h("div", { class: "toast__message" }, options.message),
    options.detail &&
      h(
        "details",
        null,
        h("summary", null, "Details"),
        h("div", { class: "toast__detail" }, options.detail),
      ),
  );

  // Clicking anywhere on a toast dismisses it; nothing inside it is a destination.
  element.addEventListener("click", (event) => {
    // Except the disclosure, which would otherwise close before it could be read.
    if ((event.target as Element).closest("details")) return;
    remove(element);
  });

  container().appendChild(element);

  if (!options.sticky) {
    window.setTimeout(() => remove(element), LIFETIME);
  }
}

function remove(element: HTMLElement): void {
  if (!element.isConnected) return;

  element.animate(
    [
      { opacity: 1, transform: "translateY(0)" },
      { opacity: 0, transform: "translateY(8px)" },
    ],
    { duration: 180, easing: "cubic-bezier(0.32, 0.72, 0, 1)" },
  ).onfinish = () => element.remove();
}

/** Clears everything. Used when a window is hidden, so stale toasts do not reappear. */
export function clearToasts(): void {
  if (host) fill(host);
}
