import { SvelteSet } from "svelte/reactivity";
import { call, type Status } from "./api";
import type { Timeline } from "./timeline";

export type View =
  | { kind: "photos" }
  | { kind: "videos" }
  | { kind: "live" }
  | { kind: "favorites" }
  | { kind: "archive" }
  | { kind: "trash" }
  | { kind: "albums" }
  | { kind: "album"; id: number; title: string }
  | { kind: "people" }
  | { kind: "person"; id: number; name: string }
  | { kind: "search"; q: string }
  | { kind: "library" };

export interface ViewerState {
  timeline: Timeline;
  index: number;
}

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

function loadNum(key: string, def: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v > 0 ? v : def;
  } catch {
    return def;
  }
}

class App {
  view = $state<View>({ kind: "photos" });
  history: View[] = [];
  status = $state<Status | null>(null);
  sidebar = $state(true);
  rowHeight = $state(loadNum("rowHeight", 210));
  selection = new SvelteSet<number>();
  /** The timeline currently on screen (selection & viewer navigate it). */
  current = $state.raw<Timeline | null>(null);
  viewer = $state.raw<ViewerState | null>(null);
  revealId = $state<number | null>(null);
  exporting = $state.raw<number[] | null>(null);
  query = $state("");
  generation = $state(0);
  toasts = $state<Toast[]>([]);
  dragging = $state(false);
}

export const app = new App();

export function setRowHeight(h: number) {
  app.rowHeight = Math.round(Math.min(420, Math.max(96, h)));
  try {
    localStorage.setItem("rowHeight", String(app.rowHeight));
  } catch {
    /* private mode */
  }
}

export function viewKey(v: View): string {
  switch (v.kind) {
    case "album":
    case "person":
      return `${v.kind}:${v.id}`;
    case "search":
      return `search:${v.q}`;
    default:
      return v.kind;
  }
}

export function filterOf(v: View): Record<string, unknown> {
  switch (v.kind) {
    case "album":
    case "person":
      return { view: v.kind, id: v.id };
    case "search":
      return { view: "search", q: v.q };
    default:
      return { view: v.kind };
  }
}

export function navigate(v: View) {
  if (viewKey(v) === viewKey(app.view)) return;
  app.history.push(app.view);
  if (app.history.length > 50) app.history.shift();
  app.selection.clear();
  app.view = v;
  if (v.kind !== "search") app.query = "";
}

export function back() {
  const prev = app.history.pop();
  app.selection.clear();
  app.view = prev ?? { kind: "photos" };
  if (app.view.kind === "search") app.query = app.view.q;
  else app.query = "";
}

export async function refreshStatus() {
  try {
    const s = await call<Status>("status");
    if (app.status && s.generation !== app.status.generation) app.generation++;
    app.status = s;
  } catch (e) {
    console.error(e);
  }
}

let toastId = 0;
export function toast(text: string, action?: Toast["action"], ms = 5000) {
  const t = { id: ++toastId, text, action };
  app.toasts = [...app.toasts, t];
  setTimeout(() => (app.toasts = app.toasts.filter((x) => x.id !== t.id)), ms);
}
