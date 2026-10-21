<script lang="ts">
  import {
    mdiClose,
    mdiCalendarBlankOutline,
    mdiImageOutline,
    mdiVideoOutline,
    mdiCameraOutline,
    mdiMapMarkerOutline,
    mdiFolderOutline,
    mdiAccountOutline,
    mdiImageAlbum,
    mdiMotionPlayOutline,
    mdiContentCopy,
    mdiPencilOutline,
  } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { navigate } from "../lib/state.svelte";
  import { call, isTauri, isMac, openUrl, type ItemDetails } from "../lib/api";
  import { fmtBytes, fmtDuration, fmtLongDate, fmtTime } from "../lib/format";

  let {
    details,
    onclose,
    onnavigate,
  }: { details: ItemDetails | null; onclose: () => void; onnavigate: () => void } = $props();

  const d = $derived(details);
  const mp = $derived(d && d.kind === 0 && d.width ? ((d.width * d.height) / 1e6).toFixed(1) : null);
  const dateSource = $derived(
    d?.dateSource === "takeout" ? "Date from Google Takeout" : d?.dateSource === "exif" ? "Date from photo metadata" : "Date from file (no metadata found)",
  );

  function go(v: Parameters<typeof navigate>[0]) {
    onnavigate();
    navigate(v);
  }
</script>

<aside class="info">
  <div class="ihead">
    <button class="icon-btn" title="Close" onclick={onclose}><Icon path={mdiClose} /></button>
    <span>Info</span>
  </div>
  {#if d}
    <div class="body">
      {#if d.description}
        <p class="desc"><Icon path={mdiPencilOutline} size={18} /> {d.description}</p>
      {/if}

      <div class="sec">Details</div>
      <div class="row">
        <Icon path={mdiCalendarBlankOutline} />
        <div>
          <div class="p">{fmtLongDate(d.taken)}</div>
          <div class="s">{fmtTime(d.taken)} · {dateSource}</div>
        </div>
      </div>
      <div class="row">
        <Icon path={d.kind === 1 ? mdiVideoOutline : mdiImageOutline} />
        <div class="min">
          <div class="p name" title={d.name}>{d.name}</div>
          <div class="s">
            {#if d.kind === 1 && d.durationMs}{fmtDuration(d.durationMs / 1000)} ·
            {/if}{#if mp}{mp}MP ·
            {/if}{#if d.width}{d.width} × {d.height} ·
            {/if}{fmtBytes(d.size)}
          </div>
        </div>
      </div>
      {#if d.live}
        <div class="row">
          <Icon path={mdiMotionPlayOutline} />
          <div class="p">Live / motion photo</div>
        </div>
      {/if}
      {#if d.camera}
        <div class="row">
          <Icon path={mdiCameraOutline} />
          <div class="p">{d.camera}</div>
        </div>
      {/if}
      {#if d.lat !== null && d.lon !== null}
        <div class="row">
          <Icon path={mdiMapMarkerOutline} />
          <div>
            <div class="p">{d.lat.toFixed(5)}, {d.lon.toFixed(5)}</div>
            <button class="link" onclick={() => openUrl(`https://maps.apple.com/?ll=${d.lat},${d.lon}&q=Photo`)}>Open in Maps</button>
          </div>
        </div>
      {/if}

      {#if d.people.length}
        <div class="sec">People</div>
        <div class="chips">
          {#each d.people as p (p.id)}
            <button class="chip" onclick={() => go({ kind: "person", id: p.id, name: p.name })}>
              <Icon path={mdiAccountOutline} size={18} />{p.name}
            </button>
          {/each}
        </div>
      {/if}

      {#if d.albums.length}
        <div class="sec">Albums</div>
        <div class="chips">
          {#each d.albums as a (a.id)}
            <button class="chip" onclick={() => go({ kind: "album", id: a.id, title: a.title })}>
              <Icon path={mdiImageAlbum} size={18} />{a.title}
            </button>
          {/each}
        </div>
      {/if}

      <div class="sec">On disk</div>
      <div class="row">
        <Icon path={mdiFolderOutline} />
        <div class="min">
          <div class="p">{d.folder || "Top folder"}</div>
          <div class="s path" title={d.path}>{d.path}</div>
          <div class="actions">
            {#if isTauri}
              <button class="link" onclick={() => call("reveal", { id: d.id })}>Show in {isMac ? "Finder" : "File Explorer"}</button>
            {/if}
            <button class="link" onclick={() => navigator.clipboard?.writeText(d.path)}>
              <Icon path={mdiContentCopy} size={14} /> Copy path
            </button>
          </div>
          {#if d.duplicates > 0}
            <div class="s">Also found in {d.duplicates} other {d.duplicates === 1 ? "folder" : "folders"} of your Takeout</div>
          {/if}
        </div>
      </div>
    </div>
  {/if}
</aside>

<style>
  .info {
    width: 360px;
    flex: none;
    background: var(--bg);
    color: var(--text);
    display: flex;
    flex-direction: column;
    animation: slide 0.18s ease;
  }
  @keyframes slide {
    from {
      transform: translateX(40px);
      opacity: 0;
    }
  }
  .ihead {
    height: 64px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    font-size: 20px;
  }
  .body {
    overflow-y: auto;
    padding: 0 24px 32px;
    -webkit-user-select: text;
    user-select: text;
  }
  .desc {
    display: flex;
    gap: 10px;
    font-size: 15px;
    margin: 4px 0 12px;
    color: var(--text);
  }
  .sec {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-3);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    margin: 20px 0 8px;
  }
  .row {
    display: flex;
    gap: 20px;
    padding: 10px 0;
    color: var(--text-2);
  }
  .min {
    min-width: 0;
  }
  .p {
    color: var(--text);
    font-size: 14px;
  }
  .name,
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    max-width: 280px;
  }
  .s {
    font-size: 12px;
    color: var(--text-3);
    margin-top: 2px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .chip:hover {
    background: var(--surface-2);
  }
  .link {
    color: var(--primary);
    font-size: 13px;
    font-weight: 500;
    margin-top: 4px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .actions {
    display: flex;
    gap: 16px;
  }
</style>
