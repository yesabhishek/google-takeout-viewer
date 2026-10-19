// Bridge to the Rust core. Inside the app it uses Tauri IPC and the `gp://`
// media protocol; in a plain browser (`npm run dev` + `takeout-viewer serve`)
// it talks to the local HTTP dev bridge instead.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const BRIDGE = "http://127.0.0.1:1421";
const isWindows = typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);
export const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform || navigator.userAgent);

export async function call<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isTauri) return invoke<T>("api", { name, args });
  const r = await fetch(`${BRIDGE}/api/${name}`, { method: "POST", body: JSON.stringify(args) });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export async function callBin(name: string, args: Record<string, unknown> = {}): Promise<ArrayBuffer> {
  if (isTauri) return invoke<ArrayBuffer>("api_bin", { name, args });
  const r = await fetch(`${BRIDGE}/api/${name}`, { method: "POST", body: JSON.stringify(args) });
  if (!r.ok) throw new Error(await r.text());
  return r.arrayBuffer();
}

export type MediaRoute = "thumb" | "media" | "motion" | "preview";

export function mediaUrl(route: MediaRoute, id: number, bust?: string | number): string {
  const q = bust !== undefined ? `?v=${bust}` : "";
  if (!isTauri) return `${BRIDGE}/gp/${route}/${id}${q}`;
  return isWindows ? `http://gp.localhost/${route}/${id}${q}` : `gp://localhost/${route}/${id}${q}`;
}

export async function onEvent<T>(name: string, cb: (payload: T) => void): Promise<UnlistenFn> {
  if (!isTauri) return () => {};
  return listen<T>(name, (e) => cb(e.payload));
}

export async function pickFolder(title: string): Promise<string | null> {
  if (!isTauri) {
    return window.prompt(`${title} (dev mode: type a path)`) || null;
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, multiple: false, title });
  return typeof r === "string" ? r : null;
}

export async function confirmDialog(message: string, okLabel = "OK"): Promise<boolean> {
  if (!isTauri) return window.confirm(message);
  const { ask } = await import("@tauri-apps/plugin-dialog");
  return ask(message, { title: "Takeout Viewer", kind: "warning", okLabel, cancelLabel: "Cancel" });
}

export async function revealPath(path: string) {
  if (!isTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

export async function openUrl(url: string) {
  if (!isTauri) {
    window.open(url, "_blank");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

// ---------------------------------------------------------------------------
// Types mirrored from the Rust side

export interface Source {
  id: number;
  path: string;
  online: boolean;
  lastScan: number | null;
  files: number;
}

export interface ScanStatus {
  running: boolean;
  phase: string;
  done: number;
  total: number;
  found: number;
  message: string;
  error: string | null;
  finished_at: number | null;
}

export interface ExportStatus {
  running: boolean;
  done: number;
  total: number;
  failed: number;
  current: string;
  dest: string;
  errors: string[];
}

export interface Status {
  generation: number;
  sources: Source[];
  counts: { photos: number; videos: number; live: number; people: number; albums: number };
  scan: ScanStatus;
  export: ExportStatus;
  thumbs: { done: number; total: number; paused: boolean };
  formats: { id: string; label: string; ext: string }[];
  platform: string;
  dataDir: string;
  ffmpeg: boolean;
}

export interface Person {
  id: number;
  name: string;
  count: number;
  latest: number | null;
  cover: number | null;
}

export interface Album {
  id: number;
  title: string;
  description: string | null;
  count: number;
  start: number | null;
  end: number | null;
  cover: number | null;
}

export interface ItemDetails {
  id: number;
  online: boolean;
  path: string;
  name: string;
  kind: number;
  size: number;
  width: number;
  height: number;
  durationMs: number | null;
  taken: number;
  camera: string | null;
  lat: number | null;
  lon: number | null;
  description: string | null;
  favorite: boolean;
  live: boolean;
  originalId: number | null;
  archived: boolean;
  trashed: boolean;
  dateSource: "takeout" | "exif" | "file";
  folder: string;
  duplicates: number;
  mime: string;
  ext: string;
  displayable: boolean;
  playable: boolean;
  people: { id: number; name: string }[];
  albums: { id: number; title: string }[];
}

export interface ExportOptions {
  format: string;
  quality?: number;
  maxPx?: number | null;
  includeVideos?: boolean;
  includeMotion?: boolean;
  keepDates?: boolean;
}
