<script lang="ts">
  import { mdiTrayArrowDown } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { app, toast } from "../lib/state.svelte";
  import { call, pickFolder, type ExportOptions } from "../lib/api";
  import { FLAG_VIDEO, FLAG_LIVE } from "../lib/timeline";
  import { plural } from "../lib/format";

  const ids = app.exporting ?? [];
  const t = app.viewer?.timeline ?? app.current;
  const { videos, live } = (() => {
    let videos = 0;
    let live = 0;
    for (const id of ids) {
      const i = t?.index.get(id);
      if (i === undefined || !t) continue;
      if (t.flags[i] & FLAG_VIDEO) videos++;
      else if (t.flags[i] & FLAG_LIVE) live++;
    }
    return { videos, live };
  })();
  const photos = ids.length - videos;

  function saved<T>(key: string, def: T): T {
    try {
      const v = localStorage.getItem(key);
      return v === null ? def : (JSON.parse(v) as T);
    } catch {
      return def;
    }
  }

  const formats = [{ id: "original", label: "Original", ext: "" }, ...(app.status?.formats ?? [])];
  let format = $state<string>(saved("export.format", "original"));
  if (!formats.some((f) => f.id === format)) format = "original";
  let quality = $state<number>(saved("export.quality", 0.9));
  let maxPx = $state<number | null>(saved("export.maxPx", null));
  let includeVideos = $state<boolean>(saved("export.videos", true));
  let includeMotion = $state<boolean>(saved("export.motion", false));
  let keepDates = $state<boolean>(saved("export.dates", true));
  let busy = $state(false);

  const lossy = $derived(["jpeg", "heic", "webp", "avif"].includes(format));
  const sizes: { label: string; px: number | null }[] = [
    { label: "Full size", px: null },
    { label: "4K · 3840 px", px: 3840 },
    { label: "2048 px", px: 2048 },
    { label: "1024 px", px: 1024 },
  ];

  function close() {
    app.exporting = null;
  }

  async function start() {
    const dest = await pickFolder(`Export ${plural(ids.length, "item")} to…`);
    if (!dest) return;
    busy = true;
    const options: ExportOptions = {
      format,
      quality,
      maxPx: format === "original" ? null : maxPx,
      includeVideos,
      includeMotion,
      keepDates,
    };
    try {
      localStorage.setItem("export.format", JSON.stringify(format));
      localStorage.setItem("export.quality", JSON.stringify(quality));
      localStorage.setItem("export.maxPx", JSON.stringify(maxPx));
      localStorage.setItem("export.videos", JSON.stringify(includeVideos));
      localStorage.setItem("export.motion", JSON.stringify(includeMotion));
      localStorage.setItem("export.dates", JSON.stringify(keepDates));
    } catch {
      /* ignore */
    }
    try {
      await call("export", { ids, dest, options });
      app.selection.clear();
      close();
    } catch (e) {
      toast(String(e));
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" onclick={close}>
  <div class="dialog" role="dialog" tabindex="-1" aria-modal="true" aria-labelledby="exp-title" onclick={(e) => e.stopPropagation()}>
    <div class="icon"><Icon path={mdiTrayArrowDown} /></div>
    <h2 id="exp-title">Export {plural(ids.length, "item")}</h2>
    <p class="sub">
      {plural(photos, "photo")}{#if videos} · {plural(videos, "video")}{/if}{#if live} · {plural(live, "live photo")}{/if}
    </p>

    {#if photos > 0}
      <div class="label">Photo format</div>
      <div class="seg">
        {#each formats as f (f.id)}
          <button class:on={format === f.id} onclick={() => (format = f.id)}>{f.label}</button>
        {/each}
      </div>
      {#if format !== "original"}
        <div class="label">Size</div>
        <div class="seg">
          {#each sizes as s (s.label)}
            <button class:on={maxPx === s.px} onclick={() => (maxPx = s.px)}>{s.label}</button>
          {/each}
        </div>
        {#if lossy}
          <div class="label">Quality <span class="val">{Math.round(quality * 100)}%</span></div>
          <input type="range" min="0.5" max="1" step="0.01" bind:value={quality} />
        {/if}
        <p class="hint">Full-size conversions keep EXIF, GPS and orientation.</p>
      {:else}
        <p class="hint">Files are copied byte-for-byte, exactly as they are in your Takeout.</p>
      {/if}
    {/if}

    <div class="toggles">
      {#if videos}
        <label><input type="checkbox" bind:checked={includeVideos} /> Include videos (always as originals)</label>
      {/if}
      {#if live}
        <label><input type="checkbox" bind:checked={includeMotion} /> Include the video part of Live & motion photos</label>
      {/if}
      <label><input type="checkbox" bind:checked={keepDates} /> Set file dates to when the photo was taken</label>
    </div>

    <div class="actions">
      <button class="btn text" onclick={close}>Cancel</button>
      <button class="btn filled" onclick={start} disabled={busy}>Choose folder & export</button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    z-index: 200;
    display: grid;
    place-items: center;
    animation: fade 0.15s ease;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .dialog {
    width: min(560px, calc(100vw - 48px));
    background: var(--bg);
    color: var(--text);
    border-radius: 28px;
    padding: 24px 24px 20px;
    box-shadow: var(--shadow-lg);
    animation: pop 0.18s ease;
  }
  @keyframes pop {
    from {
      transform: scale(0.96);
      opacity: 0;
    }
  }
  .icon {
    color: var(--primary);
    display: flex;
    justify-content: center;
  }
  h2 {
    text-align: center;
    font-size: 22px;
    font-weight: 400;
    margin: 12px 0 2px;
  }
  .sub {
    text-align: center;
    color: var(--text-3);
    margin: 0 0 16px;
  }
  .label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-2);
    margin: 16px 0 8px;
    display: flex;
    justify-content: space-between;
  }
  .val {
    color: var(--text-3);
    font-weight: 400;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    border: 1px solid var(--outline);
    border-radius: 20px;
    overflow: hidden;
  }
  .seg button {
    flex: 1;
    height: 38px;
    padding: 0 12px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-2);
    border-left: 1px solid var(--outline);
    white-space: nowrap;
  }
  .seg button:first-child {
    border-left: 0;
  }
  .seg button.on {
    background: var(--primary-container);
    color: var(--on-primary-container);
  }
  input[type="range"] {
    width: 100%;
    accent-color: var(--primary);
  }
  .hint {
    font-size: 12px;
    color: var(--text-3);
    margin: 10px 0 0;
  }
  .toggles {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 20px 0 8px;
  }
  .toggles label {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
  }
  .toggles input {
    width: 18px;
    height: 18px;
    accent-color: var(--primary);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
  }
</style>
