<script lang="ts">
  import { onMount } from "svelte";
  import { mdiArrowLeft, mdiTrayArrowDown, mdiAccountOutline, mdiFolderPlusOutline } from "@mdi/js";
  import TopBar from "./components/TopBar.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import PhotoGrid from "./components/PhotoGrid.svelte";
  import Viewer from "./components/Viewer.svelte";
  import PeoplePage from "./components/PeoplePage.svelte";
  import AlbumsPage from "./components/AlbumsPage.svelte";
  import LibraryPage from "./components/LibraryPage.svelte";
  import Welcome from "./components/Welcome.svelte";
  import ExportDialog from "./components/ExportDialog.svelte";
  import Icon from "./components/Icon.svelte";
  import { app, back, filterOf, refreshStatus, toast, viewKey } from "./lib/state.svelte";
  import { isTauri, isMac, mediaUrl, onEvent, revealPath, call, type ExportStatus, type ScanStatus } from "./lib/api";
  import { addSource } from "./lib/actions";
  import { fmtCount, fmtRange, plural } from "./lib/format";

  const s = $derived(app.status);
  const hasSources = $derived(!!s && s.sources.length > 0);
  const scanning = $derived(!!s?.scan.running);

  const titles: Record<string, string> = {
    favorites: "Favorites",
    videos: "Videos",
    live: "Live & motion photos",
    archive: "Archive",
    trash: "Trash",
  };

  const empty = $derived.by(() => {
    const v = app.view;
    switch (v.kind) {
      case "photos":
        return scanning
          ? { t: "Importing your library…", x: "Photos appear here as they are indexed — newest first." }
          : { t: "No photos found", x: "Check that the folder contains your unzipped Google Takeout export." };
      case "favorites":
        return { t: "No favorites yet", x: "Photos you starred in Google Photos show up here. Press F in the viewer to star one." };
      case "videos":
        return { t: "No videos", x: "" };
      case "live":
        return { t: "No Live or motion photos", x: "iPhone Live Photos and Pixel/Samsung motion photos appear here." };
      case "archive":
        return { t: "Nothing archived", x: "Items you archived in Google Photos are kept out of the main timeline and shown here." };
      case "trash":
        return { t: "Trash is empty", x: "Items that were in your Google Photos trash when you exported show up here." };
      case "search":
        return { t: `No results for “${v.q}”`, x: "Try a file name, a person, an album, a camera, a year like 2019 or a month like July." };
      default:
        return { t: "Nothing here yet", x: "" };
    }
  });

  const count = $derived(app.current?.n ?? 0);
  const range = $derived.by(() => {
    const t = app.current;
    if (!t || t.n === 0) return "";
    return fmtRange(t.ts[t.n - 1], t.ts[0]);
  });

  function exportAll() {
    if (app.current) app.exporting = Array.from(app.current.ids);
  }

  // ---- live updates -------------------------------------------------------
  let lastExport: ExportStatus | null = null;
  $effect(() => {
    const e = s?.export;
    if (!e) return;
    if (lastExport?.running && !e.running) {
      const ok = e.done - e.failed;
      const msg = e.failed ? `Exported ${fmtCount(ok)} of ${fmtCount(e.total)} — ${e.failed} failed` : `Exported ${plural(ok, "item")}`;
      toast(msg, isTauri ? { label: isMac ? "Show in Finder" : "Show folder", run: () => revealPath(e.dest) } : undefined, 9000);
      if (e.errors.length) console.warn("export errors", e.errors);
    }
    lastExport = e;
  });

  onMount(() => {
    refreshStatus();
    const unlisten: Array<() => void> = [];
    onEvent<ScanStatus>("scan-progress", (p) => {
      if (app.status) app.status = { ...app.status, scan: p };
    }).then((u) => unlisten.push(u));
    onEvent<ExportStatus>("export-progress", (p) => {
      if (app.status) app.status = { ...app.status, export: p };
    }).then((u) => unlisten.push(u));
    onEvent("library-changed", () => refreshStatus()).then((u) => unlisten.push(u));

    const timer = setInterval(() => refreshStatus(), isTauri ? 3000 : 1500);

    if (isTauri) {
      import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
        getCurrentWebview()
          .onDragDropEvent((e) => {
            const p = e.payload;
            if (p.type === "enter" || p.type === "over") app.dragging = true;
            else if (p.type === "leave") app.dragging = false;
            else if (p.type === "drop") {
              app.dragging = false;
              if (p.paths.length) addSource(p.paths[0]);
            }
          })
          .then((u) => unlisten.push(u)),
      );
    }
    return () => {
      clearInterval(timer);
      unlisten.forEach((u) => u());
    };
  });
</script>

{#snippet gridHeader()}
  {@const v = app.view}
  {#if v.kind === "album"}
    <div class="ghead">
      <button class="icon-btn" title="Back" onclick={back}><Icon path={mdiArrowLeft} /></button>
      <div class="gt">
        <h1>{v.title}</h1>
        <div class="gs">{range}{#if range} · {/if}{plural(count, "item")}</div>
      </div>
      {#if count}<button class="btn tonal" onclick={exportAll}><Icon path={mdiTrayArrowDown} size={20} /> Export album</button>{/if}
    </div>
  {:else if v.kind === "person"}
    <div class="ghead">
      <button class="icon-btn" title="Back" onclick={back}><Icon path={mdiArrowLeft} /></button>
      {#if app.current && app.current.n}
        <img class="avatar" src={mediaUrl("thumb", app.current.ids[0])} alt="" />
      {:else}
        <span class="avatar ph"><Icon path={mdiAccountOutline} /></span>
      {/if}
      <div class="gt">
        <h1>{v.name}</h1>
        <div class="gs">{plural(count, "item")}{#if range} · {range}{/if}</div>
      </div>
      {#if count}<button class="btn tonal" onclick={exportAll}><Icon path={mdiTrayArrowDown} size={20} /> Export all</button>{/if}
    </div>
  {:else if v.kind === "search"}
    <div class="ghead slim">
      <div class="gt">
        <h1>Results for “{v.q}”</h1>
        <div class="gs">{plural(count, "item")}</div>
      </div>
      {#if count}<button class="btn text" onclick={exportAll}><Icon path={mdiTrayArrowDown} size={20} /> Export results</button>{/if}
    </div>
  {:else if titles[v.kind]}
    <div class="ghead slim">
      <div class="gt">
        <h1>{titles[v.kind]}</h1>
        {#if count}<div class="gs">{plural(count, "item")}</div>{/if}
      </div>
    </div>
  {/if}
{/snippet}

<div class="app">
  <TopBar />
  <div class="body">
    {#if app.sidebar && hasSources}
      <Sidebar />
    {/if}
    <main>
      {#if !s}
        <div class="boot"></div>
      {:else if !hasSources && app.view.kind !== "library"}
        <Welcome />
      {:else if app.view.kind === "people"}
        <PeoplePage />
      {:else if app.view.kind === "albums"}
        <AlbumsPage />
      {:else if app.view.kind === "library"}
        <LibraryPage />
      {:else}
        {#key viewKey(app.view)}
          <PhotoGrid filter={filterOf(app.view)} emptyTitle={empty.t} emptyText={empty.x}>
            {@render gridHeader()}
          </PhotoGrid>
        {/key}
      {/if}
    </main>
  </div>

  {#if app.viewer}
    <Viewer />
  {/if}
  {#if app.exporting}
    <ExportDialog />
  {/if}

  <div class="toasts">
    {#if s?.export.running}
      <div class="toast progress">
        <div class="tt">Exporting {fmtCount(s.export.done)} of {fmtCount(s.export.total)}</div>
        <div class="bar"><div style="width:{(s.export.done / Math.max(1, s.export.total)) * 100}%"></div></div>
        <button class="btn text" onclick={() => call("cancel_export")}>Cancel</button>
      </div>
    {/if}
    {#each app.toasts as t (t.id)}
      <div class="toast">
        <span>{t.text}</span>
        {#if t.action}<button class="btn text inv" onclick={t.action.run}>{t.action.label}</button>{/if}
      </div>
    {/each}
  </div>

  {#if app.dragging}
    <div class="dropzone">
      <div class="dz">
        <Icon path={mdiFolderPlusOutline} size={48} />
        <div>Drop your Takeout folder to add it</div>
      </div>
    </div>
  {/if}
</div>

<style>
  .app {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  main {
    flex: 1;
    min-width: 0;
    position: relative;
  }
  .boot {
    height: 100%;
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 20px 64px 12px 16px;
  }
  .ghead.slim {
    padding: 16px 64px 4px 20px;
  }
  .gt {
    flex: 1;
    min-width: 0;
  }
  .ghead h1 {
    font-size: 28px;
    font-weight: 400;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ghead.slim h1 {
    font-size: 22px;
  }
  .gs {
    color: var(--text-3);
    font-size: 13px;
    margin-top: 4px;
  }
  .avatar {
    width: 64px;
    height: 64px;
    border-radius: 50%;
    object-fit: cover;
    background: var(--placeholder);
    display: grid;
    place-items: center;
    color: var(--text-3);
    flex: none;
  }
  .toasts {
    position: fixed;
    left: 24px;
    bottom: 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 300;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    min-width: 300px;
    max-width: 560px;
    background: #303030;
    color: #f2f2f2;
    border-radius: 6px;
    padding: 6px 8px 6px 16px;
    min-height: 48px;
    display: flex;
    align-items: center;
    gap: 16px;
    box-shadow: var(--shadow-lg);
    animation: up 0.2s ease;
  }
  .toast span {
    flex: 1;
    padding: 8px 0;
  }
  @keyframes up {
    from {
      transform: translateY(12px);
      opacity: 0;
    }
  }
  .toast.progress {
    flex-wrap: wrap;
    padding: 12px 8px 8px 16px;
  }
  .tt {
    flex: 1;
  }
  .toast .bar {
    order: 3;
    flex-basis: 100%;
    height: 4px;
    background: rgba(255, 255, 255, 0.2);
    border-radius: 2px;
    overflow: hidden;
    margin: 4px 8px 4px 0;
  }
  .toast .bar div {
    height: 100%;
    background: #8ab4f8;
    transition: width 0.3s;
  }
  .btn.inv,
  .toast .btn.text {
    color: #8ab4f8;
    height: 36px;
  }
  .dropzone {
    position: fixed;
    inset: 0;
    z-index: 400;
    background: color-mix(in srgb, var(--primary) 12%, transparent);
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .dz {
    width: 100%;
    height: 100%;
    border: 3px dashed var(--primary);
    border-radius: 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--primary);
    font-size: 20px;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
</style>
