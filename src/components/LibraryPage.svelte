<script lang="ts">
  import {
    mdiFolderPlusOutline,
    mdiRefresh,
    mdiClose,
    mdiHarddisk,
    mdiFolderOutline,
    mdiPause,
    mdiPlay,
    mdiFolderOpenOutline,
    mdiShieldCheckOutline,
    mdiCheck,
    mdiAlertOutline,
    mdiWeb,
    mdiGithub,
    mdiAutoFix,
    mdiTortoise,
    mdiSpeedometerMedium,
    mdiSpeedometer,
  } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import Logo from "./Logo.svelte";
  import { app, refreshStatus, toast } from "../lib/state.svelte";
  import { call, revealPath, isTauri, isMac, confirmDialog, openUrl, WEBSITE, REPO, type DriveInfo, type Source } from "../lib/api";
  import { addFolder } from "../lib/actions";
  import { describeFolder, fmtAgo, fmtBytes, fmtCount, plural, shortPath } from "../lib/format";

  const s = $derived(app.status);
  const finder = isMac ? "Finder" : "File Explorer";
  const computer = isMac ? "Mac" : "computer";
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
    const name = describeFolder(path).name;
    const ok = await confirmDialog(
      `Remove “${name}” from Takeout Viewer?\n\nThis only forgets the index and thumbnails list. Your photos and the folder itself are not touched.`,
      "Remove",
    );
    if (!ok) return;
    await call("remove_source", { id });
    refreshStatus();
  }
  async function togglePause() {
    await call("thumbs_pause", { paused: !s?.thumbs.paused });
    refreshStatus();
  }
  async function clearCache() {
    if (!(await confirmDialog("Delete all cached thumbnails? They will be made again from your originals.", "Clear cache"))) return;
    await call("clear_cache");
    cache = null;
    app.generation++;
    refreshStatus();
  }
  async function setDrive(choice: DriveInfo["choice"]) {
    if (!s || s.drive.choice === choice) return;
    const drive = await call<DriveInfo>("set_drive_mode", { mode: choice });
    app.status = { ...s, drive };
  }

  function sourceStatus(src: Source): string {
    const parts = [src.online ? "Connected" : "Not connected", plural(src.files, "file")];
    if (s?.scan.running) parts.push("indexing now");
    else if (src.lastScan) parts.push(`indexed ${fmtAgo(src.lastScan)}`);
    else if (src.files) parts.push("indexing didn’t finish, press Rescan");
    else parts.push("not indexed yet");
    return parts.join(" · ");
  }

  const driveModes: { id: DriveInfo["choice"]; label: string; icon: string; text: string }[] = [
    { id: "auto", label: "Auto", icon: mdiAutoFix, text: "Picks a safe pace for each drive." },
    { id: "gentle", label: "Gentle", icon: mdiTortoise, text: "One file at a time, with rests. For old or external hard drives." },
    { id: "balanced", label: "Balanced", icon: mdiSpeedometerMedium, text: "A couple of files at a time. For SSDs and newer drives." },
    { id: "fast", label: "Fast", icon: mdiSpeedometer, text: `As fast as your ${computer} allows. Internal SSDs only.` },
  ];
  const modeName = (m: string) => driveModes.find((d) => d.id === m)?.label ?? m;
  const pct = (d: number, t: number) => (t > 0 ? Math.min(100, (d / t) * 100) : 0);
</script>

<div class="page">
  <h1 class="page-title">Library & storage</h1>

  {#if s}
    <section>
      <div class="sh">
        <div>
          <h2>Takeout folders</h2>
          <p class="hint">Folders are only ever read. Nothing in them is changed, moved or deleted.</p>
        </div>
        <div class="acts">
          <button class="btn text" onclick={rescan} disabled={s.scan.running || !s.sources.length}>
            <Icon path={mdiRefresh} size={20} class={s.scan.running ? "spin" : ""} />
            {s.scan.running ? "Scanning…" : "Rescan"}
          </button>
          <button class="btn tonal" onclick={addFolder}><Icon path={mdiFolderPlusOutline} size={20} /> Add folder</button>
        </div>
      </div>
      {#each s.sources as src (src.id)}
        {@const f = describeFolder(src.path)}
        <div class="source">
          <span class="src-icon" class:off={!src.online}><Icon path={f.external ? mdiHarddisk : mdiFolderOutline} /></span>
          <div class="grow">
            <div class="name" title={src.path}>{f.name}</div>
            <div class="where" title={src.path}>{f.where}</div>
            <div class="sub">
              <span class="dot" class:off={!src.online}></span>
              {sourceStatus(src)}
            </div>
          </div>
          {#if isTauri && src.online}
            <button class="icon-btn" title="Show in {finder}" onclick={() => revealPath(src.path)}><Icon path={mdiFolderOpenOutline} /></button>
          {/if}
          <button class="icon-btn" title="Remove from Takeout Viewer (files stay where they are)" onclick={() => remove(src.id, src.path)}>
            <Icon path={mdiClose} />
          </button>
        </div>
      {:else}
        <div class="empty">
          <p>No Takeout folders yet.</p>
          <button class="btn filled" onclick={addFolder}><Icon path={mdiFolderPlusOutline} size={20} /> Choose a Takeout folder</button>
        </div>
      {/each}

      {#if s.scan.running}
        <div class="progress">
          <div class="pt">{s.scan.message || "Scanning…"}</div>
          <div class="bar"><div style="width:{s.scan.total ? pct(s.scan.done, s.scan.total) : 5}%"></div></div>
          <button class="btn text" onclick={() => call("cancel_scan")}>Cancel</button>
        </div>
      {:else if s.scan.error}
        <p class="note err"><Icon path={mdiAlertOutline} size={18} /> {s.scan.error}</p>
      {:else if s.scan.warning}
        <p class="note warn"><Icon path={mdiAlertOutline} size={18} /> {s.scan.message}. {s.scan.warning}</p>
      {:else if s.scan.message}
        <p class="note muted"><Icon path={mdiCheck} size={18} /> {s.scan.message}</p>
      {/if}
    </section>

    <section>
      <div class="sh">
        <div>
          <h2>Drive care</h2>
          <p class="hint">How hard Takeout Viewer reads your drive while it indexes and prepares thumbnails. Slower is kinder to old disks.</p>
        </div>
      </div>
      <div class="modes" role="radiogroup" aria-label="Drive care">
        {#each driveModes as m (m.id)}
          <button class="mode" class:on={s.drive.choice === m.id} role="radio" aria-checked={s.drive.choice === m.id} onclick={() => setDrive(m.id)}>
            <span class="mh"><Icon path={m.icon} size={20} /> {m.label}{#if m.id === "auto"}<span class="rec">Recommended</span>{/if}</span>
            <span class="mt">{m.text}</span>
          </button>
        {/each}
      </div>
      <p class="note muted">
        <span>Now reading at <b>{modeName(s.drive.mode)}</b> pace. {s.drive.reason}</span>
      </p>
      {#if s.drive.struggling}
        <p class="note warn">
          <Icon path={mdiAlertOutline} size={18} />
          Some files couldn’t be read, so Takeout Viewer has slowed down to protect the drive. Check the cable, try another
          port, or use Gentle.
        </p>
      {/if}
      <ul class="promises">
        <li><Icon path={mdiShieldCheckOutline} size={18} /> Read-only: your Takeout files are never modified, moved or deleted.</li>
        <li><Icon path={mdiShieldCheckOutline} size={18} /> Exports always create new files and never overwrite anything.</li>
        <li><Icon path={mdiShieldCheckOutline} size={18} /> “Open with default app” opens a read-only copy, so editors can’t change originals.</li>
        <li><Icon path={mdiShieldCheckOutline} size={18} /> If the drive disconnects mid-scan, scanning stops and your library is kept as it was.</li>
      </ul>
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
        <div>
          <h2>Thumbnails</h2>
          <p class="hint">
            Small previews are made once and kept on this {computer}, so scrolling is instant and you can browse with the
            drive unplugged. Visible photos always come first, and background work waits while a scan runs.
          </p>
        </div>
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
      {#if s.thumbs.total > 0}
        <div class="progress">
          <div class="pt">
            {#if s.thumbs.done >= s.thumbs.total}
              All thumbnails ready
            {:else if s.scan.running}
              Waiting for the scan to finish · {fmtCount(s.thumbs.done)} of {fmtCount(s.thumbs.total)}
            {:else}
              {fmtCount(s.thumbs.done)} of {fmtCount(s.thumbs.total)} prepared{s.thumbs.paused ? " (paused)" : ""}
            {/if}
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
        <span>{s.ffmpeg ? "ffmpeg found: AVI, MKV and WMV get previews" : "Install ffmpeg for AVI, MKV and WMV previews"}</span>
      </div>
    </section>

    <section class="about">
      <div class="about-head">
        <Logo size={56} />
        <div>
          <h2>Takeout Viewer</h2>
          <div class="ver">Version {s.version} · Free and open source</div>
        </div>
      </div>
      <p class="why">
        Google Takeout hands you your whole photo library as a pile of zip files and JSON sidecars. Great for owning your
        memories, terrible for actually looking at them. I built Takeout Viewer to make that archive feel like Google
        Photos again: on your own {computer}, offline, and without ever putting the originals at risk.
      </p>
      <p class="by">Made by <b>Abhishek Choudhury</b></p>
      <div class="links">
        <button class="btn outlined" onclick={() => openUrl(WEBSITE)}><Icon path={mdiWeb} size={20} /> abhishekchoudhury.com</button>
        <button class="btn text" onclick={() => openUrl(REPO)}><Icon path={mdiGithub} size={20} /> Source on GitHub</button>
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
  .hint {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--text-3);
    max-width: 560px;
    line-height: 1.5;
  }
  .sh {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 12px;
  }
  .sh h2 {
    margin: 0;
  }
  .acts {
    display: flex;
    gap: 8px;
    flex: none;
  }
  .source {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 14px 0;
    color: var(--text-2);
    border-top: 1px solid var(--outline-soft);
  }
  .src-icon {
    width: 44px;
    height: 44px;
    border-radius: 12px;
    display: grid;
    place-items: center;
    flex: none;
    background: var(--primary-container);
    color: var(--on-primary-container);
  }
  .src-icon.off {
    background: var(--surface-2);
    color: var(--text-3);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .name {
    color: var(--text);
    font-weight: 500;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .where {
    font-size: 13px;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    margin-top: 1px;
  }
  .sub {
    font-size: 12px;
    color: var(--text-3);
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #1e8e3e;
    flex: none;
  }
  .dot.off {
    background: #f9ab00;
  }
  .empty {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 0 4px;
    color: var(--text-3);
  }
  .empty p {
    margin: 0;
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
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: 13px;
    line-height: 1.5;
    margin: 12px 0 0;
  }
  .note :global(svg) {
    margin-top: 1px;
  }
  .note b {
    font-weight: 600;
    color: var(--text);
  }
  .err {
    color: var(--danger);
  }
  .warn {
    color: var(--text);
    background: color-mix(in srgb, #f9ab00 16%, transparent);
    padding: 10px 12px;
    border-radius: 10px;
  }
  .modes {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }
  .mode {
    display: flex;
    flex-direction: column;
    gap: 6px;
    text-align: left;
    padding: 12px 14px;
    border-radius: 12px;
    border: 1px solid var(--outline-soft);
    transition: border-color 0.15s, background 0.15s;
  }
  .mode:hover {
    background: var(--surface-2);
  }
  .mode.on {
    border-color: var(--primary);
    background: color-mix(in srgb, var(--primary) 10%, transparent);
  }
  .mh {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 500;
    flex-wrap: wrap;
  }
  .mode.on .mh {
    color: var(--primary);
  }
  .rec {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-3);
  }
  .mt {
    font-size: 12px;
    color: var(--text-3);
    line-height: 1.45;
  }
  .promises {
    list-style: none;
    margin: 16px 0 0;
    padding: 14px 0 0;
    border-top: 1px solid var(--outline-soft);
    display: grid;
    gap: 8px;
    font-size: 13px;
    color: var(--text-2);
  }
  .promises li {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .promises :global(svg) {
    color: #1e8e3e;
    margin-top: 1px;
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
  .about-head {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .about-head h2 {
    margin: 0;
    font-size: 18px;
  }
  .ver {
    font-size: 13px;
    color: var(--text-3);
    margin-top: 2px;
  }
  .why {
    font-size: 14px;
    line-height: 1.6;
    color: var(--text-2);
    max-width: 640px;
    margin: 16px 0 8px;
  }
  .by {
    margin: 0 0 16px;
    font-size: 14px;
    color: var(--text-2);
  }
  .by b {
    color: var(--text);
    font-weight: 600;
  }
  .links {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  :global(.spin) {
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (max-width: 1000px) {
    .modes {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .stats {
      grid-template-columns: repeat(3, 1fr);
    }
  }
</style>
