<script lang="ts">
  import { mdiFolderPlusOutline, mdiRefresh, mdiClose, mdiHarddisk, mdiPause, mdiPlay, mdiFolderOpenOutline } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { app, refreshStatus, toast } from "../lib/state.svelte";
  import { call, revealPath, isTauri, confirmDialog } from "../lib/api";
  import { addFolder } from "../lib/actions";
  import { fmtAgo, fmtBytes, fmtCount, shortPath } from "../lib/format";

  const s = $derived(app.status);
  let cache = $state<{ bytes: number; files: number; path: string } | null>(null);
  let measuring = $state(false);

  async function measure() {
    measuring = true;
    cache = await call("cache_info");
    measuring = false;
  }
  async function rescan() {
    const started = await call<boolean>("rescan");
    toast(started ? "Rescanning your library…" : "A scan is already running");
    refreshStatus();
  }
  async function remove(id: number, path: string) {
    const ok = await confirmDialog(`Remove “${path}” from your library?\n\nThis only forgets the index; your files are not touched.`, "Remove");
    if (!ok) return;
    await call("remove_source", { id });
    refreshStatus();
  }
  async function togglePause() {
    await call("thumbs_pause", { paused: !s?.thumbs.paused });
    refreshStatus();
  }
  async function clearCache() {
    if (!(await confirmDialog("Delete all cached thumbnails? They will be regenerated from your originals.", "Clear cache"))) return;
    await call("clear_cache");
    cache = null;
    app.generation++;
    refreshStatus();
  }
  const pct = (d: number, t: number) => (t > 0 ? Math.min(100, (d / t) * 100) : 0);
</script>

<div class="page">
  <h1 class="page-title">Library & storage</h1>

  {#if s}
    <section>
      <div class="sh">
        <h2>Takeout folders</h2>
        <div class="acts">
          <button class="btn text" onclick={rescan} disabled={s.scan.running}><Icon path={mdiRefresh} size={20} /> Rescan</button>
          <button class="btn tonal" onclick={addFolder}><Icon path={mdiFolderPlusOutline} size={20} /> Add folder</button>
        </div>
      </div>
      {#each s.sources as src (src.id)}
        <div class="source">
          <Icon path={mdiHarddisk} />
          <div class="grow">
            <div class="path" title={src.path}>{shortPath(src.path)}</div>
            <div class="sub">
              <span class="dot" class:off={!src.online}></span>
              {src.online ? "Connected" : "Not connected"} · {fmtCount(src.files)} files · scanned {fmtAgo(src.lastScan)}
            </div>
          </div>
          {#if isTauri && src.online}
            <button class="icon-btn" title="Show in Finder" onclick={() => revealPath(src.path)}><Icon path={mdiFolderOpenOutline} /></button>
          {/if}
          <button class="icon-btn" title="Remove from library" onclick={() => remove(src.id, src.path)}><Icon path={mdiClose} /></button>
        </div>
      {:else}
        <p class="muted">No folders yet.</p>
      {/each}

      {#if s.scan.running}
        <div class="progress">
          <div class="pt">{s.scan.message || "Scanning…"}</div>
          <div class="bar"><div style="width:{s.scan.total ? pct(s.scan.done, s.scan.total) : 5}%"></div></div>
          <button class="btn text" onclick={() => call("cancel_scan")}>Cancel</button>
        </div>
      {:else if s.scan.error}
        <p class="err">Last scan: {s.scan.error}</p>
      {:else if s.scan.message}
        <p class="muted">{s.scan.message}</p>
      {/if}
    </section>

    <section>
      <h2>Overview</h2>
      <div class="stats">
        <div><b>{fmtCount(s.counts.photos)}</b><span>photos</span></div>
        <div><b>{fmtCount(s.counts.videos)}</b><span>videos</span></div>
        <div><b>{fmtCount(s.counts.live)}</b><span>live & motion</span></div>
        <div><b>{fmtCount(s.counts.people)}</b><span>people</span></div>
        <div><b>{fmtCount(s.counts.albums)}</b><span>albums</span></div>
      </div>
    </section>

    <section>
      <div class="sh">
        <h2>Thumbnails</h2>
        <div class="acts">
          {#if s.thumbs.total > s.thumbs.done}
            <button class="btn text" onclick={togglePause}>
              <Icon path={s.thumbs.paused ? mdiPlay : mdiPause} size={20} />
              {s.thumbs.paused ? "Resume" : "Pause"}
            </button>
          {/if}
          <button class="btn text" onclick={clearCache}>Clear cache</button>
        </div>
      </div>
      <p class="muted">
        Small previews are generated once and kept on this Mac, so scrolling is instant and you can browse even when the
        drive is unplugged. Visible photos are always done first.
      </p>
      {#if s.thumbs.total > 0}
        <div class="progress">
          <div class="pt">
            {s.thumbs.done >= s.thumbs.total ? "All thumbnails ready" : `${fmtCount(s.thumbs.done)} of ${fmtCount(s.thumbs.total)} prepared`}
          </div>
          <div class="bar"><div style="width:{pct(s.thumbs.done, s.thumbs.total)}%"></div></div>
        </div>
      {/if}
      <div class="kv">
        <span>Cache size</span>
        {#if cache}
          <span>{fmtBytes(cache.bytes)} · {fmtCount(cache.files)} files</span>
        {:else}
          <button class="btn text" onclick={measure} disabled={measuring}>{measuring ? "Measuring…" : "Measure"}</button>
        {/if}
      </div>
      <div class="kv"><span>Data folder</span><span class="mono" title={s.dataDir}>{shortPath(s.dataDir, 64)}</span></div>
      <div class="kv">
        <span>Unusual video formats</span>
        <span>{s.ffmpeg ? "ffmpeg found — AVI/MKV/WMV previews supported" : "Install ffmpeg for AVI/MKV/WMV previews"}</span>
      </div>
    </section>
  {/if}
</div>

<style>
  section {
    max-width: 880px;
    padding: 20px 24px;
    border: 1px solid var(--outline-soft);
    border-radius: 16px;
    margin-bottom: 20px;
  }
  h2 {
    font-size: 16px;
    font-weight: 500;
    margin: 0 0 12px;
  }
  .sh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .sh h2 {
    margin: 0;
  }
  .acts {
    display: flex;
    gap: 8px;
  }
  .source {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 12px 0;
    color: var(--text-2);
    border-top: 1px solid var(--outline-soft);
  }
  .source:first-of-type {
    border-top: 0;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .path {
    color: var(--text);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: 12px;
    color: var(--text-3);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #1e8e3e;
  }
  .dot.off {
    background: #f9ab00;
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 12px;
  }
  .pt {
    font-size: 13px;
    min-width: 260px;
  }
  .bar {
    flex: 1;
    height: 6px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--primary) 18%, transparent);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--primary);
    transition: width 0.4s;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 12px;
  }
  .stats div {
    display: flex;
    flex-direction: column;
    padding: 12px 14px;
    background: var(--surface-2);
    border-radius: 12px;
  }
  .stats b {
    font-size: 22px;
    font-weight: 400;
  }
  .stats span {
    font-size: 12px;
    color: var(--text-3);
  }
  .kv {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 24px;
    padding: 10px 0;
    border-top: 1px solid var(--outline-soft);
    font-size: 13px;
    min-height: 44px;
  }
  .kv span:first-child {
    color: var(--text-3);
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    -webkit-user-select: text;
    user-select: text;
  }
  .err {
    color: var(--danger);
  }
</style>
