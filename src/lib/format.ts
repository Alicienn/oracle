/** Display helpers. Pure functions, no DOM. */

/**
 * Formats a byte count the way a task manager would.
 *
 * Binary units, because that is what Windows reports and a mismatch of 7% between Oracle and
 * Task Manager would look like a bug.
 */
export function bytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 MB";

  const units = ["B", "KB", "MB", "GB", "TB"];
  let index = 0;
  let scaled = value;

  while (scaled >= 1024 && index < units.length - 1) {
    scaled /= 1024;
    index += 1;
  }

  const digits = scaled >= 100 || index <= 1 ? 0 : 1;
  return `${scaled.toFixed(digits)} ${units[index]}`;
}

/**
 * CPU as a percentage.
 *
 * The backend reports percent of a single core, so a busy four-core machine legitimately
 * reports more than 100. That is not clamped: hiding it would make a project pinning four
 * cores look identical to one using a quarter of one.
 */
export function percent(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "0%";
  return `${value < 10 ? value.toFixed(1) : Math.round(value)}%`;
}

export function milliseconds(value: number): string {
  if (value < 1000) return `${Math.round(value)} ms`;
  return `${(value / 1000).toFixed(2)} s`;
}

/** "3m ago", "2d ago". Deliberately coarse — precision here is noise. */
export function relativeTime(epochMs: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - epochMs) / 1000));

  if (seconds < 45) return "just now";
  if (seconds < 90) return "1m ago";

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;

  const months = Math.round(days / 30);
  if (months < 12) return `${months}mo ago`;

  return `${Math.round(months / 12)}y ago`;
}

/** How long something has been running: "4m", "2h 15m". */
export function uptime(sinceMs: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - sinceMs) / 1000));

  if (seconds < 60) return `${seconds}s`;

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;

  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  if (hours < 24) return remainder ? `${hours}h ${remainder}m` : `${hours}h`;

  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** Shortens a path for display without losing the part that identifies it. */
export function shortPath(path: string, segments = 2): string {
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  if (parts.length <= segments) return path;
  return `…/${parts.slice(-segments).join("/")}`;
}

export function initials(name: string): string {
  const words = name.trim().split(/[\s\-_.]+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0]!.slice(0, 2).toUpperCase();
  return (words[0]![0]! + words[1]![0]!).toUpperCase();
}

/**
 * Builds an SVG path through a series of values, normalised to the given box.
 *
 * Returns both the line and a closed version for the area fill, so a sparkline needs only
 * one pass over the data.
 */
export function sparkPath(
  values: number[],
  width: number,
  height: number,
): { line: string; area: string } {
  if (values.length < 2) return { line: "", area: "" };

  const max = Math.max(...values, 1);
  const step = width / (values.length - 1);

  const points = values.map((value, index) => {
    const x = index * step;
    // Leave a pixel of headroom so a peak is not clipped by the stroke width.
    const y = height - (value / max) * (height - 2) - 1;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });

  const line = `M${points.join("L")}`;
  const area = `${line}L${width.toFixed(2)},${height}L0,${height}Z`;

  return { line, area };
}
