import { call, pickFolder } from "./api";
import { app, navigate, refreshStatus, toast } from "./state.svelte";

export async function addSource(path: string) {
  try {
    await call("add_source", { path });
    const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
    toast(`Importing “${name}”… photos appear as they are indexed`);
    if (app.view.kind === "library" || !app.status?.sources.length) navigate({ kind: "photos" });
    await refreshStatus();
  } catch (e) {
    toast(String(e));
  }
}

export async function addFolder() {
  const p = await pickFolder("Choose your Google Takeout folder (or the drive it is on)");
  if (p) await addSource(p);
}
