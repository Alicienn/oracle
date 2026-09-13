/** The settings dialog. */

import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { GlassLevel, Theme } from "../lib/api";
import { h, fill } from "../lib/dom";
import { glassLevel } from "../lib/glass";
import { get, reportError, saveSettings } from "../lib/store";
import { brandMark } from "../lib/icons";
import { button, segmented, toggleRow } from "./common";
import { closeModal, showModal } from "./modal";
import { showUpdatePrompt } from "./updatePrompt";
import { checkNow } from "../lib/updates";

export function showSettings(): void {
  const state = get();
  const settings = state.settings;

  const appearance = h("div", { class: "section" });
  const renderAppearance = () => {
    const current = get().settings;

    fill(
      appearance,
      h("p", { class: "section__label" }, "Appearance"),
      h(
        "div",
        { class: "field" },
        h("label", null, "Theme"),
        segmented<Theme>(
          [
            { value: "light", label: "Light", iconName: "sun" },
            { value: "dark", label: "Dark", iconName: "moon" },
            { value: "system", label: "System" },
          ],
          current.theme,
          (theme) => {
            void saveSettings({ theme });
            renderAppearance();
          },
        ),
      ),
      h(
        "div",
        { class: "field" },
        h("label", null, "Glass"),
        segmented<GlassLevel>(
          [
            { value: "full", label: "Full", title: "Refraction, blur and specular highlight" },
            { value: "reduced", label: "Reduced", title: "Blur and highlight, no refraction" },
            { value: "opaque", label: "Opaque", title: "Flat surfaces" },
          ],
          current.glass,
          (glass) => {
            void saveSettings({ glass });
            renderAppearance();
          },
        ),
        // Say so when the frame-rate probe overrode the choice, rather than letting the
        // setting silently disagree with what is on screen.
        current.glass === "full" && glassLevel() !== "full"
          ? h(
              "p",
              { class: "field__hint" },
              "This machine could not hold 60 fps with refraction on, so it is running reduced.",
            )
          : null,
      ),
    );
  };
  renderAppearance();

  // ---- Startup -----------------------------------------------------------
  const startup = h(
    "div",
    { class: "section" },
    h("p", { class: "section__label" }, "Startup"),
    toggleRow(
      "Start with Windows",
      "Registers Oracle in the startup list. No administrator rights needed.",
      settings.startWithWindows,
      (startWithWindows) => void saveSettings({ startWithWindows }),
    ),
    toggleRow(
      "Start hidden in the tray",
      "Comes up as a tray icon only, without opening the window.",
      settings.startHidden,
      (startHidden) => void saveSettings({ startHidden }),
    ),
    toggleRow(
      "Launch flagged projects",
      "Starts every project marked to run when Oracle starts.",
      settings.autostartProjects,
      (autostartProjects) => void saveSettings({ autostartProjects }),
    ),
    toggleRow(
      "Close to tray",
      "Closing the window keeps Oracle running. Quit from the tray menu.",
      settings.minimiseToTray,
      (minimiseToTray) => void saveSettings({ minimiseToTray }),
    ),
  );

  // ---- Monitoring --------------------------------------------------------
  const intervalInput = h("input", {
    class: "input",
    type: "number",
    min: "5",
    max: "3600",
    value: String(settings.healthIntervalSecs),
    onChange: (event: Event) => {
      const value = Math.max(5, Number((event.target as HTMLInputElement).value) || 30);
      void saveSettings({ healthIntervalSecs: value });
    },
  });

  const shortcutInput = h("input", {
    class: "input input--mono",
    value: settings.panelShortcut,
    onChange: (event: Event) => {
      void saveSettings({ panelShortcut: (event.target as HTMLInputElement).value.trim() });
    },
  });

  const monitoring = h(
    "div",
    { class: "section" },
    h("p", { class: "section__label" }, "Monitoring"),
    h(
      "div",
      { class: "field" },
      h("label", null, "Health check interval (seconds)"),
      intervalInput,
      h("p", { class: "field__hint" }, "How often remote projects are polled."),
    ),
    h(
      "div",
      { class: "field" },
      h("label", null, "Panel shortcut"),
      shortcutInput,
      h("p", { class: "field__hint" }, "Takes effect the next time Oracle starts."),
    ),
  );

  // ---- Scan roots --------------------------------------------------------
  const rootList = h("div", { class: "section" });

  const renderRoots = () => {
    const roots = get().settings.scanRoots;

    fill(
      rootList,
      h("p", { class: "section__label" }, "Folders to scan"),
      roots.length === 0
        ? h(
            "p",
            { class: "field__hint" },
            "No folders yet. Add the directories your projects live in.",
          )
        : h(
            "div",
            null,
            ...roots.map((root) =>
              h(
                "div",
                { class: "toggle" },
                h("div", { class: "toggle__text" }, h("strong", null, root)),
                button({
                  iconName: "trash",
                  variant: "ghost",
                  title: `Stop scanning ${root}`,
                  onClick: () => {
                    void saveSettings({
                      scanRoots: get().settings.scanRoots.filter((item) => item !== root),
                    }).then(renderRoots);
                  },
                }),
              ),
            ),
          ),
      h(
        "div",
        { style: { marginTop: "12px" } },
        button({
          label: "Add folder",
          iconName: "plus",
          onClick: async () => {
            try {
              const picked = await openDialog({ directory: true, multiple: false });
              if (typeof picked !== "string") return;
              if (get().settings.scanRoots.includes(picked)) return;

              await saveSettings({ scanRoots: [...get().settings.scanRoots, picked] });
              renderRoots();
            } catch (error) {
              reportError("Could not add the folder", error);
            }
          },
        }),
      ),
    );
  };
  renderRoots();

  // ---- About -------------------------------------------------------------
  const mark = h("div", {
    html: brandMark(28),
    style: { color: "var(--accent)", display: "flex" },
  });

  const updateStatus = h("span", { class: "field__hint" });

  const checkButton = button({
    label: "Check for updates",
    onClick: async () => {
      checkButton.disabled = true;
      updateStatus.textContent = "Checking…";

      const { update, error } = await checkNow();

      checkButton.disabled = false;

      if (error) {
        // Named plainly: the user asked, so "could not check" is the answer, not silence.
        updateStatus.textContent = "Could not reach the release feed.";
        return;
      }

      if (!update) {
        updateStatus.textContent = "Oracle is up to date.";
        return;
      }

      updateStatus.textContent = "";
      closeModal();
      showUpdatePrompt(update, get().version);
    },
  });

  const about = h(
    "div",
    { class: "section" },
    h("p", { class: "section__label" }, "About"),
    h(
      "div",
      { style: { display: "flex", alignItems: "center", gap: "12px" } },
      mark,
      h(
        "div",
        null,
        h("strong", null, "Oracle"),
        h(
          "div",
          { style: { color: "var(--muted)", fontSize: "var(--text-sm)" } },
          `Version ${state.version || "0.1.0"}${state.gitAvailable ? "" : " · git not found on PATH"}`,
        ),
      ),
    ),
    toggleRow(
      "Check for updates at launch",
      "Asks GitHub once, when Oracle starts, whether a newer release exists.",
      settings.checkUpdates,
      (checkUpdates) => void saveSettings({ checkUpdates }),
    ),
    h(
      "div",
      { style: { display: "flex", alignItems: "center", gap: "12px", marginTop: "8px" } },
      checkButton,
      updateStatus,
    ),
  );

  showModal({
    title: "Settings",
    body: h("div", null, appearance, startup, monitoring, rootList, about),
    actions: [button({ label: "Done", variant: "primary", onClick: () => closeModal() })],
  });
}
