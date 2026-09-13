/**
 * The discovery dialog.
 *
 * Scanning proposes; it never adds anything on its own. Candidates already in the config are
 * shown greyed out rather than hidden, so it is obvious the scan found them and chose not to
 * offer them twice.
 */

import { api, type Candidate } from "../lib/api";
import { h, fill } from "../lib/dom";
import { shortPath } from "../lib/format";
import { refreshProjects, reportError, get } from "../lib/store";
import { toast } from "../lib/toast";
import { button } from "./common";
import { closeModal, showModal } from "./modal";

export function showScan(): void {
  const body = h("div", null);
  const selection = new Set<string>();

  const confirm = button({
    label: "Add selected",
    variant: "primary",
    disabled: true,
    onClick: () => void commit(),
  });

  let candidates: Candidate[] = [];

  const updateConfirm = () => {
    confirm.disabled = selection.size === 0;
    confirm.textContent = selection.size
      ? `Add ${selection.size} project${selection.size === 1 ? "" : "s"}`
      : "Add selected";
  };

  const commit = async () => {
    const chosen = candidates.filter((candidate) => selection.has(candidate.root));
    if (chosen.length === 0) return;

    confirm.disabled = true;

    try {
      await api.importCandidates(chosen);
      await refreshProjects();
      toast({
        tone: "success",
        title: `Added ${chosen.length} project${chosen.length === 1 ? "" : "s"}`,
      });
      closeModal();
    } catch (error) {
      reportError("Could not add the projects", error);
      confirm.disabled = false;
    }
  };

  const renderResults = () => {
    if (candidates.length === 0) {
      fill(
        body,
        h(
          "p",
          { class: "field__hint" },
          "Nothing found. Check the folders Oracle scans in Settings.",
        ),
      );
      return;
    }

    fill(
      body,
      h(
        "p",
        { class: "field__hint", style: { marginBottom: "16px" } },
        `Found ${candidates.length} project${candidates.length === 1 ? "" : "s"}. Pick the ones to add.`,
      ),
      ...candidates.map((candidate) => {
        const checkbox = h("input", {
          type: "checkbox",
          disabled: candidate.alreadyKnown,
          onChange: (event: Event) => {
            if ((event.target as HTMLInputElement).checked) {
              selection.add(candidate.root);
            } else {
              selection.delete(candidate.root);
            }
            updateConfirm();
          },
        }) as HTMLInputElement;

        return h(
          "label",
          {
            class: "toggle",
            style: candidate.alreadyKnown ? { opacity: "0.45", cursor: "default" } : undefined,
          },
          checkbox,
          h(
            "div",
            { class: "toggle__text", style: { flex: "1", minWidth: "0" } },
            h("strong", null, candidate.name),
            h(
              "span",
              null,
              `${shortPath(candidate.root, 3)} · ${candidate.suggestedCommand || "no command"}`,
            ),
          ),
          candidate.alreadyKnown ? h("span", { class: "tag" }, "Already added") : null,
        );
      }),
    );
  };

  const runScan = async () => {
    fill(body, h("p", { class: "field__hint" }, "Scanning…"));

    try {
      candidates = await api.scan();
      selection.clear();
      updateConfirm();
      renderResults();
    } catch (error) {
      reportError("The scan failed", error);
      fill(body, h("p", { class: "field__hint" }, "The scan could not finish."));
    }
  };

  showModal({
    title: "Find projects",
    body,
    actions: [
      button({ label: "Rescan", onClick: () => void runScan() }),
      button({ label: "Cancel", onClick: () => closeModal() }),
      confirm,
    ],
  });

  if (get().settings.scanRoots.length === 0) {
    fill(
      body,
      h(
        "p",
        { class: "field__hint" },
        "No folders are set to be scanned. Add one under Settings, then rescan.",
      ),
    );
  } else {
    void runScan();
  }
}
