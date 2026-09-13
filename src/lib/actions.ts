/**
 * User-triggered operations.
 *
 * Each one wraps a command with the optimistic state change, the error toast, and the
 * refresh that belongs with it, so no component has to remember that sequence.
 */

import { api, type Project, type ProjectView } from "./api";
import { get, patchProject, refreshProjects, reportError } from "./store";
import { toast } from "./toast";
import { openUrl } from "@tauri-apps/plugin-opener";

export async function start(project: ProjectView): Promise<void> {
  if (!project.local) {
    toast({
      tone: "info",
      title: `${project.name} has no local setup`,
      message: "Add a folder and a command in the project settings before launching it.",
    });
    return;
  }

  // Optimistic: the spinner should appear on the click, not on the round trip.
  patchProject(project.id, { status: "starting" });

  try {
    await api.start(project.id);
  } catch (error) {
    patchProject(project.id, { status: "stopped" });
    reportError(`Could not start ${project.name}`, error);
  }
}

export async function stop(project: ProjectView): Promise<void> {
  try {
    await api.stop(project.id);
    patchProject(project.id, { status: "stopped", pid: null });
  } catch (error) {
    reportError(`Could not stop ${project.name}`, error);
  }
}

export async function restart(project: ProjectView): Promise<void> {
  patchProject(project.id, { status: "starting" });

  try {
    await api.restart(project.id);
  } catch (error) {
    patchProject(project.id, { status: "stopped" });
    reportError(`Could not restart ${project.name}`, error);
  }
}

export async function toggle(project: ProjectView): Promise<void> {
  const live = project.status === "running" || project.status === "starting";
  return live ? stop(project) : start(project);
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
    (project) => project.local && project.status !== "running" && project.status !== "starting",
  );

  for (const project of idle) {
    await start(project);
  }
}

export async function stopAll(): Promise<void> {
  const live = get().projects.filter(
    (project) => project.status === "running" || project.status === "starting",
  );

  for (const project of live) {
    await stop(project);
  }
}
