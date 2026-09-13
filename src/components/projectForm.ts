/**
 * The add and edit form.
 *
 * One dialog for both, because the fields are identical and a separate "create" flow would
 * only duplicate the validation. The local and remote sections are independent: a project
 * can have either, both, or neither.
 */

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  api,
  emptyProject,
  type IconSource,
  type Project,
  type ProjectKind,
} from "../lib/api";
import { h, qs } from "../lib/dom";
import { reportError } from "../lib/store";
import { save } from "../lib/actions";
import { button } from "./common";
import { closeModal, showModal } from "./modal";

const KINDS: { value: ProjectKind; label: string }[] = [
  { value: "nextJs", label: "Next.js" },
  { value: "node", label: "Node" },
  { value: "rust", label: "Rust" },
  { value: "python", label: "Python" },
  { value: "docker", label: "Docker" },
  { value: "static", label: "Static site" },
  { value: "other", label: "Other" },
];

export function showProjectForm(existing?: Project): void {
  // Work on a copy: abandoning the dialog must leave the store untouched.
  const draft: Project = existing
    ? structuredClone(existing)
    : { ...emptyProject(), kind: "other" };

  let icon: IconSource = draft.icon;

  const field = (
    label: string,
    control: HTMLElement,
    hint?: string,
  ): HTMLElement =>
    h(
      "div",
      { class: "field" },
      h("label", null, label),
      control,
      hint && h("p", { class: "field__hint" }, hint),
    );

  const nameInput = h("input", {
    class: "input",
    id: "pf-name",
    value: draft.name,
    placeholder: "Project name",
    maxlength: "64",
  });

  const kindSelect = h(
    "select",
    { class: "input select", id: "pf-kind" },
    ...KINDS.map((kind) =>
      h("option", { value: kind.value, selected: kind.value === draft.kind }, kind.label),
    ),
  );

  // ---- Local -------------------------------------------------------------
  const rootInput = h("input", {
    class: "input input--mono",
    id: "pf-root",
    value: draft.local?.root ?? "",
    placeholder: "C:\\Users\\you\\projects\\app",
  });

  const browse = button({
    label: "Browse",
    iconName: "folder",
    onClick: async () => {
      try {
        const picked = await openDialog({ directory: true, multiple: false });
        if (typeof picked === "string") rootInput.value = picked;
      } catch (error) {
        reportError("Could not open the folder picker", error);
      }
    },
  });

  const commandInput = h("input", {
    class: "input input--mono",
    id: "pf-command",
    value: draft.local?.command ?? "",
    placeholder: "npm run dev",
  });

  const portInput = h("input", {
    class: "input",
    id: "pf-port",
    type: "number",
    min: "1",
    max: "65535",
    value: draft.local?.port ? String(draft.local.port) : "",
    placeholder: "3000",
  });

  const openUrlInput = h("input", {
    class: "input input--mono",
    id: "pf-openurl",
    value: draft.local?.openUrl ?? "",
    placeholder: "http://localhost:3000",
  });

  const envInput = h("textarea", {
    class: "input input--mono",
    id: "pf-env",
    placeholder: "KEY=value\nANOTHER=value",
  }) as HTMLTextAreaElement;
  envInput.value = Object.entries(draft.local?.env ?? {})
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");

  const autostartInput = h("input", {
    type: "checkbox",
    id: "pf-autostart",
    checked: draft.local?.autostartWithOracle ?? false,
  }) as HTMLInputElement;

  // ---- Remote ------------------------------------------------------------
  const remoteUrlInput = h("input", {
    class: "input input--mono",
    id: "pf-remote",
    value: draft.remote?.url ?? "",
    placeholder: "https://app.example.com/health",
  });

  const intervalInput = h("input", {
    class: "input",
    id: "pf-interval",
    type: "number",
    min: "5",
    max: "3600",
    value: String(draft.remote?.intervalSecs ?? 30),
  });

  // ---- Appearance --------------------------------------------------------
  const accentInput = h("input", {
    class: "input",
    id: "pf-accent",
    type: "color",
    value: draft.accent ?? "#c15f3c",
  }) as HTMLInputElement;

  // What "automatic" actually resolves to is decided by the backend: the favicon of
  // whatever the project serves, or its initials when there is nothing to fetch.
  const AUTOMATIC = "Automatic — the site's favicon when it serves one, otherwise initials";

  const iconLabel = h(
    "span",
    { class: "field__hint" },
    icon.type === "file" ? "Custom image selected" : AUTOMATIC,
  );

  const pickIcon = button({
    label: "Choose image",
    onClick: async () => {
      try {
        const picked = await openDialog({
          multiple: false,
          filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "svg", "webp"] }],
        });
        if (typeof picked !== "string") return;

        // Copy it into Oracle's own directory so the reference cannot go stale.
        const stored = await api.importIcon(picked);
        icon = { type: "file", value: stored };
        iconLabel.textContent = "Custom image selected";
      } catch (error) {
        reportError("Could not use that image", error);
      }
    },
  });

  const clearIcon = button({
    label: "Reset",
    variant: "ghost",
    onClick: () => {
      icon = { type: "auto" };
      iconLabel.textContent = AUTOMATIC;
    },
  });

  /**
   * Fetches the site's icon again, past the cache.
   *
   * The icon is downloaded once and kept, which is what makes it instant and offline. A site
   * that changes its icon — or served something wrong the one time Oracle asked — would
   * otherwise be stuck with the old file for good, so the way out is explicit.
   *
   * Only offered where there is something to fetch: an existing project with a URL.
   */
  const refetch = button({
    label: "Refetch favicon",
    onClick: async () => {
      refetch.disabled = true;
      iconLabel.textContent = "Fetching the site's icon…";

      try {
        await api.refreshFavicon(draft.id);
        // The backend answers through an event, and the icon may legitimately turn out not
        // to exist, so this reports what was done rather than what was found.
        iconLabel.textContent = "Asked the site for its icon again.";
      } catch (error) {
        iconLabel.textContent = AUTOMATIC;
        reportError("Could not fetch the icon", error);
      } finally {
        refetch.disabled = false;
      }
    },
  });

  /** Only where there is something to fetch: a saved project with a URL. */
  const canRefetch = Boolean(existing?.id && existing.remote?.url);

  const body = h(
    "form",
    { id: "project-form", onSubmit: (event: Event) => event.preventDefault() },

    field("Name", nameInput),
    field("Type", kindSelect, "Sets the default colour and the suggested command."),

    h("p", { class: "section__label" }, "Runs locally"),
    field("Folder", h("div", { class: "row" }, rootInput, browse)),
    field("Command", commandInput, "Run through the system shell, so chaining with && works."),
    h(
      "div",
      { class: "row" },
      field("Port", portInput, "Used to tell when it is ready."),
      field("Open URL", openUrlInput, "Defaults to localhost on the port above."),
    ),
    field("Environment", envInput, "One KEY=value per line, added to the inherited environment."),
    h(
      "label",
      { class: "field", style: { display: "flex", alignItems: "center", gap: "8px" } },
      autostartInput,
      h("span", null, "Start this project when Oracle starts"),
    ),

    h("p", { class: "section__label" }, "Deployed elsewhere"),
    field("Health check URL", remoteUrlInput, "Polled with a GET; 2xx and 3xx count as up."),
    field("Interval (seconds)", intervalInput),

    h("p", { class: "section__label" }, "Appearance"),
    field("Accent colour", accentInput),
    field(
      "Icon",
      h("div", { class: "row" }, pickIcon, clearIcon, canRefetch ? refetch : null),
      undefined,
    ),
    iconLabel,
  );

  const submit = button({
    label: existing?.id ? "Save changes" : "Add project",
    variant: "primary",
    onClick: async () => {
      const name = nameInput.value.trim();
      if (!name) {
        nameInput.focus();
        nameInput.style.boxShadow = "inset 0 0 0 1px var(--danger)";
        return;
      }

      draft.name = name;
      draft.kind = kindSelect.value as ProjectKind;
      draft.icon = icon;
      draft.accent = accentInput.value;

      const root = rootInput.value.trim();
      const command = commandInput.value.trim();

      draft.local =
        root || command
          ? {
              root,
              command,
              env: parseEnv(envInput.value),
              port: portInput.value ? Number(portInput.value) : null,
              openUrl: openUrlInput.value.trim() || null,
              autostartWithOracle: autostartInput.checked,
            }
          : null;

      const remoteUrl = remoteUrlInput.value.trim();
      draft.remote = remoteUrl
        ? {
            url: remoteUrl,
            check: { type: "http", method: "GET", expectStatus: [] },
            intervalSecs: Math.max(5, Number(intervalInput.value) || 30),
          }
        : null;

      if (await save(draft)) closeModal();
    },
  });

  showModal({
    title: existing?.id ? `Edit ${existing.name}` : "Add a project",
    body,
    actions: [button({ label: "Cancel", onClick: () => closeModal() }), submit],
  });

  qs<HTMLInputElement>("#pf-name").focus();
}

/**
 * Reads the environment textarea.
 *
 * Blank lines and `#` comments are skipped so a block pasted from a `.env` file works
 * without editing. Only the first `=` splits, because values routinely contain them.
 */
function parseEnv(raw: string): Record<string, string> {
  const env: Record<string, string> = {};

  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;

    const index = trimmed.indexOf("=");
    if (index <= 0) continue;

    const key = trimmed.slice(0, index).trim();
    const value = trimmed.slice(index + 1).trim();
    if (key) env[key] = value;
  }

  return env;
}

/** Exported for the unit test in `projectForm.test.ts`. */
export const __test = { parseEnv };
