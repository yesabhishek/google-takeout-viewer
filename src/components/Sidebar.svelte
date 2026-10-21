<script lang="ts">
  import {
    mdiImageMultipleOutline,
    mdiAccountMultipleOutline,
    mdiImageAlbum,
    mdiPlayBoxMultipleOutline,
    mdiMotionPlayOutline,
    mdiStarOutline,
    mdiArchiveOutline,
    mdiTrashCanOutline,
    mdiHarddisk,
    mdiAlertCircleOutline,
  } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { app, navigate, type View } from "../lib/state.svelte";
  import { fmtCount } from "../lib/format";
  import { openUrl, WEBSITE } from "../lib/api";

  type Entry = { label: string; icon: string; view: View; count?: number };

  const c = $derived(app.status?.counts);
  const main = $derived<Entry[]>([
    { label: "Photos", icon: mdiImageMultipleOutline, view: { kind: "photos" } },
    { label: "People", icon: mdiAccountMultipleOutline, view: { kind: "people" }, count: c?.people },
    { label: "Albums", icon: mdiImageAlbum, view: { kind: "albums" }, count: c?.albums },
  ]);
  const library = $derived<Entry[]>([
    { label: "Favorites", icon: mdiStarOutline, view: { kind: "favorites" } },
    { label: "Videos", icon: mdiPlayBoxMultipleOutline, view: { kind: "videos" }, count: c?.videos },
    { label: "Live & motion", icon: mdiMotionPlayOutline, view: { kind: "live" }, count: c?.live },
    { label: "Archive", icon: mdiArchiveOutline, view: { kind: "archive" } },
    { label: "Trash", icon: mdiTrashCanOutline, view: { kind: "trash" } },
  ]);

  function active(v: View): boolean {
    const k = app.view.kind;
    if (v.kind === "albums") return k === "albums" || k === "album";
    if (v.kind === "people") return k === "people" || k === "person";
    return k === v.kind;
  }

  const scan = $derived(app.status?.scan);
  const thumbs = $derived(app.status?.thumbs);
  const offline = $derived(app.status?.sources.filter((s) => !s.online) ?? []);
  const pct = (d: number, t: number) => (t > 0 ? Math.min(100, (d / t) * 100) : 0);
</script>

<nav class="sidebar">
  <div class="group">
    {#each main as e (e.label)}
      <button class="item" class:active={active(e.view)} onclick={() => navigate(e.view)}>
        <Icon path={e.icon} />
        <span>{e.label}</span>
      </button>
    {/each}
  </div>
  <div class="heading">Library</div>
  <div class="group">
    {#each library as e (e.label)}
      <button class="item" class:active={active(e.view)} onclick={() => navigate(e.view)}>
        <Icon path={e.icon} />
        <span>{e.label}</span>
        {#if e.count}<span class="count">{fmtCount(e.count)}</span>{/if}
      </button>
    {/each}
  </div>

  <div class="foot">
    {#if offline.length}
      <button class="card warn" onclick={() => navigate({ kind: "library" })}>
        <Icon path={mdiAlertCircleOutline} size={20} />
        <div>
          <div class="t">Drive not connected</div>
          <div class="s">Thumbnails still work; connect it to open originals.</div>
        </div>
      </button>
    {/if}
    {#if app.status?.drive.struggling}
      <button class="card warn" onclick={() => navigate({ kind: "library" })}>
        <Icon path={mdiAlertCircleOutline} size={20} />
        <div>
          <div class="t">Drive is having trouble</div>
          <div class="s">Some files couldn’t be read, so reading has slowed down to protect it.</div>
        </div>
      </button>
    {/if}
    {#if scan?.running}
      <button class="card" onclick={() => navigate({ kind: "library" })}>
        <div class="t">Importing your library</div>
        <div class="s">{scan.message || "Scanning…"}</div>
        <div class="bar"><div style="width:{scan.total ? pct(scan.done, scan.total) : 6}%"></div></div>
      </button>
    {:else if thumbs && thumbs.total > 0 && thumbs.done < thumbs.total}
      <button class="card" onclick={() => navigate({ kind: "library" })}>
        <div class="t">{thumbs.paused ? "Thumbnails paused" : "Preparing thumbnails"}</div>
        <div class="s">{fmtCount(thumbs.done)} of {fmtCount(thumbs.total)}</div>
        <div class="bar"><div style="width:{pct(thumbs.done, thumbs.total)}%"></div></div>
      </button>
    {/if}
    <button class="item" class:active={app.view.kind === "library"} onclick={() => navigate({ kind: "library" })}>
      <Icon path={mdiHarddisk} />
      <span>Library & storage</span>
    </button>
    <button class="credit" title="Visit {WEBSITE}" onclick={() => openUrl(WEBSITE)}>Made by Abhishek Choudhury</button>
  </div>
</nav>

<style>
  .sidebar {
    width: var(--sidebar-w);
    flex: none;
    display: flex;
    flex-direction: column;
    padding: 8px 12px 12px 0;
    overflow-y: auto;
  }
  .group {
    display: flex;
    flex-direction: column;
  }
  .heading {
    margin: 20px 0 6px 28px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-3);
    letter-spacing: 0.04em;
  }
  .item {
    height: 48px;
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 0 16px 0 28px;
    border-radius: 0 24px 24px 0;
    color: var(--text-2);
    font-weight: 500;
    text-align: left;
    transition: background 0.12s;
  }
  .item span:first-of-type {
    flex: 1;
  }
  .item:hover {
    background: var(--surface-2);
  }
  .item.active {
    background: var(--primary-container);
    color: var(--on-primary-container);
  }
  .count {
    font-size: 12px;
    font-weight: 400;
    color: var(--text-3);
  }
  .item.active .count {
    color: inherit;
  }
  .foot {
    margin-top: auto;
    padding-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .card {
    margin-left: 16px;
    padding: 12px 14px;
    border-radius: 12px;
    background: var(--surface-2);
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .card.warn {
    flex-direction: row;
    gap: 10px;
    background: color-mix(in srgb, #f9ab00 18%, var(--bg));
    color: var(--text);
  }
  .t {
    font-weight: 500;
    font-size: 13px;
  }
  .s {
    font-size: 12px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card.warn .s {
    white-space: normal;
  }
  .credit {
    margin: 2px 0 0 28px;
    align-self: flex-start;
    font-size: 11px;
    color: var(--text-3);
    padding: 2px 0;
  }
  .credit:hover {
    color: var(--primary);
    text-decoration: underline;
  }
  .bar {
    margin-top: 8px;
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--primary) 20%, transparent);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--primary);
    border-radius: 2px;
    transition: width 0.4s;
  }
</style>
