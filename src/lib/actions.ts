/**
 * User-triggered operations.
 *
 * Each one wraps a command with the optimistic state change, the error toast, and the
 * refresh that belongs with it, so no component has to remember that sequence.
 */

import { api, portConflict, type Project, type ProjectView } from "./api";
import {
  clearPending,
  get,
  patchProject,
  pendingFor,
  refreshProjects,
  reportError,
  setPending,
} from "./store";
import { isLive } from "../components/common";
import { askAboutPort } from "../components/portConflict";
import { toast } from "./toast";
import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Starts a project.
 *
 * `port` is set only when the user has accepted an alternative after a collision; it
 * applies to this run and is never written back to the project.
 */
export async function start(project: ProjectView, port?: number): Promise<void> {
  if (!project.local) {
    toast({
      tone: "info",
      title: `${project.name} has no local setup`,
      message: "Add a folder and a command in the project settings before launching it.",
    });
    return;
  }

  // Optimistic on both counts: the spinner should appear on the click, not on the round
  // trip, and the status dot should not claim the project is stopped while it boots.
  const previous = project.status;
  setPending(project.id, "start");
  patchProject(project.id, { status: "starting" });

  try {
    await api.start(project.id, port);

    // The runner now knows things the view was built without: the pid, and the port this
    // run actually holds — which is not the configured one when a collision was resolved by
    // accepting another. Without this the address shown, and opened, is the wrong one.
    await refreshProjects();
  } catch (error) {
    clearPending(project.id);
    // Back to what it was, not to "stopped". A refused start says nothing about the
    // project's state, and "already running" in particular means something *is* alive —
    // writing "stopped" over that lost track of a real process.
    patchProject(project.id, { status: previous });

    // A port collision is the one failure with an obvious next move, so it gets a decision
    // rather than a toast the user can only dismiss.
    const conflict = portConflict(error);
    if (conflict) {
      const chosen = await askAboutPort(project, conflict);
      if (chosen !== null) await start(project, chosen);
      return;
    }

    reportError(`Could not start ${project.name}`, error);
  }
}

export async function stop(project: ProjectView): Promise<void> {
  setPending(project.id, "stop");

  try {
    await api.stop(project.id);
    patchProject(project.id, { status: "stopped", pid: null });
  } catch (error) {
    reportError(`Could not stop ${project.name}`, error);
  } finally {
    // The status event normally clears this; clearing it here too covers a stop that
    // failed, where no event is coming.
    clearPending(project.id);
  }
}

export async function restart(project: ProjectView): Promise<void> {
  setPending(project.id, "start");
  patchProject(project.id, { status: "starting" });

  try {
    await api.restart(project.id);
  } catch (error) {
    clearPending(project.id);
    // A restart stops first, so a failure here does leave the project down.
    patchProject(project.id, { status: "stopped" });

    const conflict = portConflict(error);
    if (conflict) {
      const chosen = await askAboutPort(project, conflict);
      if (chosen !== null) await start(project, chosen);
      return;
    }

    reportError(`Could not restart ${project.name}`, error);
  }
}

export async function toggle(project: ProjectView): Promise<void> {
  const pending = pendingFor(project.id);

  // A second stop is meaningless, and the button is disabled for it anyway; this guards the
  // keyboard path. A pending *start* stays actionable on purpose — it is the only way to
  // abandon a project that is not coming up, and it must not read as a fresh start.
  if (pending?.kind === "stop") return;

  return isLive(project.status, pending) ? stop(project) : start(project);
}

/**
 * Opens the project in the browser.
 *
 * Goes through the opener plugin rather than `window.open`: the webview would otherwise try
 * to navigate itself, which a Tauri app has no business doing.
 */
export async function open(project: ProjectView): Promise<void> {
  const url = project.resolvedUrl;
  if (!url) {
    toast({
      tone: "info",
      title: `${project.name} has no address`,
      message: "Set a port or a URL in the project settings.",
    });
    return;
  }

  try {
    await openUrl(url);
  } catch (error) {
    reportError("Could not open the browser", error);
  }
}

export async function revealFolder(project: ProjectView): Promise<void> {
  if (!project.local) return;

  try {
    await api.revealFolder(project.local.root);
  } catch (error) {
    reportError("Could not open the folder", error);
  }
}

export async function save(project: Project): Promise<boolean> {
  const isNew = !project.id;

  try {
    if (isNew) {
      await api.addProject(project);
    } else {
      await api.updateProject(project);
    }
    await refreshProjects();
    toast({
      tone: "success",
      title: isNew ? `${project.name} added` : `${project.name} saved`,
    });
    return true;
  } catch (error) {
    reportError(isNew ? "Could not add the project" : "Could not save the project", error);
    return false;
  }
}

export async function remove(project: ProjectView): Promise<void> {
  try {
    await api.deleteProject(project.id);
    await refreshProjects();
    toast({ tone: "success", title: `${project.name} removed` });
  } catch (error) {
    reportError("Could not remove the project", error);
  }
}

export async function toggleFavorite(project: ProjectView): Promise<void> {
  // Flip locally first; the star should respond instantly.
  patchProject(project.id, { favorite: !project.favorite });

  try {
    await api.toggleFavorite(project.id);
  } catch (error) {
    patchProject(project.id, { favorite: project.favorite });
    reportError("Could not update the favourite", error);
  }
}

export async function reorder(orderedIds: string[]): Promise<void> {
  try {
    await api.reorder(orderedIds);
    await refreshProjects();
  } catch (error) {
    reportError("Could not save the new order", error);
  }
}

/** Starts everything that is configured to run locally and is not already up. */
export async function startAll(): Promise<void> {
  const idle = get().projects.filter(
    (project) => project.local && !isLive(project.status, pendingFor(project.id)),
  );

  for (const project of idle) {
    await start(project);
  }
}

export async function stopAll(): Promise<void> {
  const live = get().projects.filter((project) =>
    isLive(project.status, pendingFor(project.id)),
  );

  for (const project of live) {
    await stop(project);
  }
}
