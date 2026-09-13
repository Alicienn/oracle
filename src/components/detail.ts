/** The right-hand detail panel. */

import type { GitStatus, LogLine, ProjectView, Usage } from "../lib/api";
import { h, fill, icon } from "../lib/dom";
import { bytes, percent, relativeTime, shortPath, sparkPath } from "../lib/format";
import { icons } from "../lib/icons";
import { api } from "../lib/api";
import { open, remove, restart, revealFolder, toggle, toggleFavorite } from "../lib/actions";
import { get, loadGit, loadLogs, patch, type Tab } from "../lib/store";
import { showProjectForm } from "./projectForm";
import {
  button,
  iconTile,
  isLive,
  pendingLabel,
  remoteLabel,
  statusDot,
  statusLabel,
} from "./common";

const TABS: { id: Tab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "logs", label: "Logs" },
  { id: "metrics", label: "Metrics" },
  { id: "git", label: "Git" },
];

export function renderDetail(host: HTMLElement, project: ProjectView | null): void {
  if (!project) {
    fill(host);
    return;
  }

  const state = get();
  const live = isLive(project.status, state.pending[project.id]);

  const head = h(
    "div",
    { class: "detail__head" },
    iconTile(project, 34),
    h(
      "div",
      { class: "detail__title" },
      h("h2", null, project.name),
      h(
        "p",
        null,
        project.local
          ? `${statusLabel(project.status)}${project.pid ? ` · pid ${project.pid}` : ""}`
          : remoteLabel(project.remoteStatus),
      ),
    ),
    button({
      iconName: "star",
      variant: "ghost",
      title: project.favorite ? "Remove from favourites" : "Add to favourites",
      onClick: () => void toggleFavorite(project),
    }),
    button({
      iconName: "edit",
      variant: "ghost",
      title: "Edit project",
      onClick: () => showProjectForm(project),
    }),
    button({
      iconName: "close",
      variant: "ghost",
      title: "Close panel",
      onClick: () => patch({ selectedId: null }),
    }),
  );

  const tabs = h(
    "div",
    { class: "tabs", role: "tablist" },
    ...TABS.map((tab) =>
      h(
        "button",
        {
          type: "button",
          role: "tab",
          "aria-selected": String(state.tab === tab.id),
          onClick: () => {
            patch({ tab: tab.id });
            if (tab.id === "logs") void loadLogs(project.id);
            if (tab.id === "git") void loadGit(project.id);
          },
        },
        tab.label,
      ),
    ),
  );

  const body =
    state.tab === "logs"
      ? logsPane(project, state.logs[project.id] ?? [])
      : h(
          "div",
          { class: "detail__body" },
          state.tab === "metrics"
            ? metricsPane(project, state.usage[project.id])
            : state.tab === "git"
              ? gitPane(project, state.git[project.id])
              : overviewPane(project, live),
        );

  fill(host, head, tabs, body);
}

// ---------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------

function overviewPane(project: ProjectView, live: boolean): HTMLElement {
  const rows: HTMLElement[] = [];
  const pending = get().pending[project.id];

  const row = (label: string, value: HTMLElement | string) => {
    rows.push(h("dt", null, label));
    rows.push(h("dd", null, value));
  };

  row("Type", project.kindLabel);

  if (project.local) {
    row(
      "Folder",
      h(
        "span",
        { title: project.local.root },
        shortPath(project.local.root, 3) || "not set",
      ),
    );
    row("Command", h("code", null, project.local.command || "not set"));
    if (project.local.port) row("Port", String(project.local.port));
    row("Status", h("span", null, statusDot(project.status), ` ${statusLabel(project.status)}`));
    if (Object.keys(project.local.env).length > 0) {
      row("Environment", `${Object.keys(project.local.env).length} variables`);
    }
  }

  if (project.remote) {
    row("Health check", h("span", { title: project.remote.url }, project.remote.url));
    row("Remote status", remoteLabel(project.remoteStatus));
  }

  if (project.repo?.slug) {
    row("Repository", project.repo.slug);
  }

  if (project.tags.length > 0) {
    row("Tags", h("span", null, ...project.tags.map((tag) => h("span", { class: "tag" }, tag))));
  }

  const actions = h(
    "div",
    { style: { display: "flex", flexWrap: "wrap", gap: "8px", marginBottom: "24px" } },
    project.local
      ? button({
          label: pending
            ? pending.kind === "start"
              ? "Starting…"
              : "Stopping…"
            : live
              ? "Stop"
              : "Start",
          iconName: live ? "stop" : "play",
          variant: live ? undefined : "primary",
          title: pending ? pendingLabel(pending.kind, project.name) : undefined,
          pending: pending?.since,
          disabled: pending?.kind === "stop",
          onClick: () => void toggle(project),
        })
      : null,
    project.local && live
      ? button({
          label: "Restart",
          iconName: "restart",
          // Restarting while a start or stop is still in flight would race it.
          disabled: pending !== undefined,
          onClick: () => void restart(project),
        })
      : null,
    project.resolvedUrl
      ? button({ label: "Open", iconName: "external", onClick: () => void open(project) })
      : null,
    project.local
      ? button({
          label: "Folder",
          iconName: "folder",
          onClick: () => void revealFolder(project),
        })
      : null,
    button({
      label: "Delete",
      iconName: "trash",
      variant: "danger",
      onClick: () => {
        // A project is cheap to re-add and deleting it touches nothing on disk, so a
        // confirmation dialog would be friction without protection.
        void remove(project);
        patch({ selectedId: null });
      },
    }),
  );

  return h(
    "div",
    null,
    actions,
    h("dl", { class: "kv" }, ...rows),
    project.notes
      ? h(
          "div",
          { class: "section", style: { marginTop: "24px" } },
          h("p", { class: "section__label" }, "Notes"),
          h("p", { style: { userSelect: "text" } }, project.notes),
        )
      : null,
  );
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

function logsPane(project: ProjectView, lines: LogLine[]): HTMLElement {
  const stream = h("div", { class: "logs" });

  if (lines.length === 0) {
    fill(
      stream,
      h(
        "p",
        { style: { color: "var(--muted)" } },
        project.status === "stopped"
          ? "Nothing yet. Start the project to see its output."
          : "Waiting for output…",
      ),
    );
  } else {
    fill(
      stream,
      ...lines.map((line) =>
        h(
          "div",
          { class: "logs__line", dataset: { stream: line.stream } },
          h("span", { class: "logs__seq" }, String(line.seq).padStart(4, "0")),
          h("span", null, line.text),
        ),
      ),
    );
  }

  // Jump to the newest line after the pane is in the document.
  requestAnimationFrame(() => {
    stream.scrollTop = stream.scrollHeight;
  });

  const bar = h(
    "div",
    { class: "logs__bar" },
    h(
      "span",
      { style: { flex: "1", color: "var(--muted)", fontSize: "var(--text-sm)" } },
      `${lines.length} line${lines.length === 1 ? "" : "s"}`,
    ),
    button({
      iconName: "copy",
      variant: "ghost",
      title: "Copy all",
      onClick: () => {
        void navigator.clipboard.writeText(lines.map((line) => line.text).join("\n"));
      },
    }),
    button({
      iconName: "trash",
      variant: "ghost",
      title: "Clear",
      onClick: () => {
        void api.clearLogs(project.id).then(() => loadLogs(project.id));
      },
    }),
  );

  return h("div", { class: "detail__body detail__body--flush" }, bar, stream);
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

function metricsPane(project: ProjectView, usage: Usage | undefined): HTMLElement {
  if (!usage) {
    return h(
      "p",
      { style: { color: "var(--muted)" } },
      "No measurements. Metrics are collected while a project is running.",
    );
  }

  const chart = (values: number[], accent: string) => {
    const { line, area } = sparkPath(values, 260, 34);
    const node = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    node.setAttribute("class", "spark gauge__chart");
    node.setAttribute("viewBox", "0 0 260 34");
    node.setAttribute("preserveAspectRatio", "none");
    node.innerHTML = `<path class="spark__fill" d="${area}" style="fill:${accent}22"/><path d="${line}" style="stroke:${accent}"/>`;
    return node;
  };

  return h(
    "div",
    null,
    h(
      "div",
      { class: "gauges" },
      h(
        "div",
        { class: "gauge glass glass--sunken" },
        h("div", { class: "gauge__value" }, percent(usage.current.cpu)),
        h("div", { class: "gauge__label" }, "CPU across the process tree"),
        chart(usage.cpuHistory, "var(--accent)"),
      ),
      h(
        "div",
        { class: "gauge glass glass--sunken" },
        h("div", { class: "gauge__value" }, bytes(usage.current.memory)),
        h("div", { class: "gauge__label" }, "Memory"),
        chart(usage.memoryHistory, "var(--ok)"),
      ),
    ),
    h(
      "dl",
      { class: "kv", style: { marginTop: "24px" } },
      h("dt", null, "Processes"),
      h("dd", null, String(usage.current.processes)),
      h("dt", null, "Process id"),
      h("dd", null, project.pid ? String(project.pid) : "—"),
    ),
    h(
      "p",
      { class: "field__hint", style: { marginTop: "16px" } },
      "CPU is percent of one core, summed over every descendant, so a project using four cores reads 400%.",
    ),
  );
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

function gitPane(project: ProjectView, status: GitStatus | null | undefined): HTMLElement {
  if (status === undefined) {
    return h("p", { style: { color: "var(--muted)" } }, "Reading…");
  }

  if (status === null) {
    return h(
      "p",
      { style: { color: "var(--muted)" } },
      project.local
        ? "This folder is not a git repository."
        : "This project has no local folder to inspect.",
    );
  }

  const rows: HTMLElement[] = [];
  const row = (label: string, value: HTMLElement | string) => {
    rows.push(h("dt", null, label));
    rows.push(h("dd", null, value));
  };

  row(
    "Branch",
    h("span", null, icon(icons.branch), ` ${status.branch ?? "detached HEAD"}`),
  );

  if (status.upstream) row("Upstream", status.upstream);

  row(
    "Sync",
    status.ahead || status.behind
      ? `${status.ahead} ahead, ${status.behind} behind`
      : status.upstream
        ? "In sync"
        : "No upstream",
  );

  row(
    "Worktree",
    status.dirtyFiles === 0
      ? "Clean"
      : `${status.dirtyFiles} changed file${status.dirtyFiles === 1 ? "" : "s"}`,
  );

  if (status.lastCommit) {
    row("Last commit", h("code", null, status.lastCommit.hash));
    row("Message", status.lastCommit.subject);
    row(
      "When",
      `${relativeTime(status.lastCommit.at * 1000)} by ${status.lastCommit.author}`,
    );
  }

  return h(
    "div",
    null,
    h("dl", { class: "kv" }, ...rows),
    project.repo?.remoteUrl
      ? h(
          "div",
          { style: { marginTop: "24px" } },
          button({
            label: "Open repository",
            iconName: "external",
            onClick: () => {
              void import("@tauri-apps/plugin-opener").then((module) =>
                module.openUrl(webUrl(project.repo?.remoteUrl ?? "")),
              );
            },
          }),
        )
      : null,
  );
}

/** Turns an SSH remote into something a browser can open. */
function webUrl(remote: string): string {
  const ssh = remote.match(/^git@([^:]+):(.+?)(\.git)?$/);
  if (ssh) return `https://${ssh[1]}/${ssh[2]}`;
  return remote.replace(/\.git$/, "");
}
