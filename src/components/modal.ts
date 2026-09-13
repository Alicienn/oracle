/** A single modal host, so two dialogs can never end up stacked on top of each other. */

import { h, icon } from "../lib/dom";
import { icons } from "../lib/icons";

interface ModalOptions {
  title: string;
  body: HTMLElement;
  /** Rendered right-aligned in the footer, in the order given. */
  actions?: HTMLElement[];
  /** Called after the modal has been removed, for any reason. */
  onClose?: () => void;
  width?: number;
}

let open: { scrim: HTMLElement; onClose?: () => void } | null = null;

export function showModal(options: ModalOptions): () => void {
  closeModal();

  const dialog = h(
    "div",
    {
      class: "modal glass glass--raised",
      role: "dialog",
      "aria-modal": "true",
      "aria-label": options.title,
      style: options.width ? { width: `${options.width}px` } : undefined,
      onClick: (event: Event) => event.stopPropagation(),
    },
    h(
      "div",
      { class: "modal__head" },
      h("h2", null, options.title),
      h(
        "button",
        {
          class: "btn btn--ghost btn--icon",
          type: "button",
          "aria-label": "Close",
          onClick: () => closeModal(),
        },
        icon(icons.close),
      ),
    ),
    h("div", { class: "modal__body" }, options.body),
    options.actions?.length ? h("div", { class: "modal__foot" }, ...options.actions) : null,
  );

  // Clicking the backdrop dismisses; clicking the dialog does not, thanks to the stop above.
  const scrim = h("div", { class: "scrim", onClick: () => closeModal() }, dialog);

  document.body.appendChild(scrim);
  open = { scrim, onClose: options.onClose };

  // Focus the first thing worth typing into, so the dialog is usable from the keyboard.
  const first = dialog.querySelector<HTMLElement>(
    "input, textarea, select, button:not([aria-label='Close'])",
  );
  first?.focus();

  return closeModal;
}

export function closeModal(): void {
  if (!open) return;

  const { scrim, onClose } = open;
  open = null;
  scrim.remove();
  onClose?.();
}

export function isModalOpen(): boolean {
  return open !== null;
}

// Escape closes whatever is open. Registered once, at module load.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && open) {
    event.preventDefault();
    closeModal();
  }
});
