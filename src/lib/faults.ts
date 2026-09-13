/**
 * Making the frontend's own failures visible.
 *
 * Every region is rebuilt by one `render` call. An exception thrown anywhere inside it
 * abandons the rest — so a single mistake leaves the window half-drawn, with no error, no
 * toast, and nothing in the interface to suggest anything went wrong. That is exactly what
 * happened the first time the embedded web app panel was rendered: the column switched
 * modes, the render threw, and the result was an empty black panel.
 *
 * A desktop app has no console anyone is watching. So the app reports its own faults.
 */

import { toast } from "./toast";

/** Faults already shown, so a throw on every frame does not become a wall of toasts. */
const seen = new Set<string>();

function report(title: string, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  const stack = error instanceof Error ? error.stack : undefined;

  // Keyed on the message: the same fault repeating is one fault, however often it fires.
  if (seen.has(message)) return;
  seen.add(message);

  toast({
    tone: "error",
    title,
    message,
    // The stack is what makes this actionable, and the toast keeps it behind a disclosure.
    detail: stack,
    sticky: true,
  });
}

/** Catches what escapes: an error in a handler, or a rejected promise nobody awaited. */
export function watchForFaults(): void {
  window.addEventListener("error", (event) => {
    report("Something went wrong in the interface", event.error ?? event.message);
  });

  window.addEventListener("unhandledrejection", (event) => {
    report("An operation failed and was not handled", event.reason);
  });
}

/**
 * Runs the render, surfacing a throw instead of leaving the window half-drawn.
 *
 * The state is not rolled back — there is nothing to roll back to — but the failure is
 * named, which is the difference between a bug and a mystery.
 */
export function guard(label: string, work: () => void): void {
  try {
    work();
  } catch (error) {
    report(`Oracle could not draw the ${label}`, error);
  }
}
