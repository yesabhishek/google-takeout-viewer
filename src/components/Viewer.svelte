<script lang="ts">
  import { untrack } from "svelte";
  import {
    mdiArrowLeft,
    mdiChevronLeft,
    mdiChevronRight,
    mdiInformationOutline,
    mdiStar,
    mdiStarOutline,
    mdiTrayArrowDown,
    mdiDotsVertical,
    mdiMotionPlayOutline,
    mdiOpenInNew,
    mdiFolderOpenOutline,
    mdiImageEditOutline,
    mdiAlertCircleOutline,
  } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import InfoPanel from "./InfoPanel.svelte";
  import { app, toast } from "../lib/state.svelte";
  import { call, mediaUrl, isTauri, type ItemDetails } from "../lib/api";
  import { FLAG_VIDEO, FLAG_LIVE } from "../lib/timeline";

  const v = $derived(app.viewer!);
  const t = $derived(v.timeline);
  const index = $derived(v.index);
  const id = $derived(t.ids[index]);
  const isVideo = $derived(!!(t.flags[index] & FLAG_VIDEO));
  const isLive = $derived(!!(t.flags[index] & FLAG_LIVE));

  let info = $state(loadInfoPref());
  let details = $state<ItemDetails | null>(null);
  let showOriginal = $state(false);
  let fullLoaded = $state(false);
  let fullFailed = $state(false);
  let menu = $state(false);
  let chrome = $state(true);
  let chromeTimer: ReturnType<typeof setTimeout> | undefined;

  function loadInfoPref() {
    try {
      return localStorage.getItem("viewerInfo") === "1";
    } catch {
      return false;
    }
  }
  $effect(() => {
    try {
      localStorage.setItem("viewerInfo", info ? "1" : "0");
    } catch {
      /* ignore */
    }
  });

  // details for the current item
  const cache = new Map<number, ItemDetails>();
  async function fetchDetails(i: number) {
    const hit = cache.get(i);
    if (hit) return hit;
    const d = await call<ItemDetails>("item", { id: i });
    cache.set(i, d);
    return d;
  }
  $effect(() => {
    const cur = id;
    untrack(() => {
      fullLoaded = false;
      fullFailed = false;
      showOriginal = false;
      resetZoom();
      details = cache.get(cur) ?? null;
      fetchDetails(cur).then((d) => {
        if (cur === id) details = d;
      });
      prefetch();
    });
  });

  const shownId = $derived(showOriginal && details?.originalId ? details.originalId : id);
  const fullSrc = $derived.by(() => {
    if (!details || details.id !== id) return null;
    if (!details.online) return null;
    if (isVideo) return details.playable ? mediaUrl("media", shownId) : null;
    return details.displayable ? mediaUrl("media", shownId) : mediaUrl("preview", shownId);
  });

  // neighbours: warm the cache so stepping through feels instant
  function prefetch() {
    for (const d of [1, -1, 2]) {
      const j = index + d;
      if (j < 0 || j >= t.n) continue;
      const nid = t.ids[j];
      new Image().src = mediaUrl("thumb", nid);
      if (!(t.flags[j] & FLAG_VIDEO)) {
        fetchDetails(nid).then((nd) => {
          if (nd.online && nd.displayable) new Image().src = mediaUrl("media", nid);
        });
      }
    }
  }

  function go(d: number) {
    const j = index + d;
    if (j < 0 || j >= t.n) return;
    app.viewer = { timeline: t, index: j };
  }

  function close() {
    app.revealId = id;
    app.viewer = null;
  }

  // ---- stage geometry & zoom ---------------------------------------------
  let stageW = $state(0);
  let stageH = $state(0);
  const box = $derived.by(() => {
    const w = t.w[index] || 4;
    const h = t.h[index] || 3;
    const a = w / h;
    // never upscale beyond the native size (videos may scale to fit)
    const maxW = isVideo ? stageW : Math.min(stageW, Math.max(w, 320));
    let bw = Math.min(maxW, stageH * a);
    let bh = bw / a;
    if (bh > stageH) {
      bh = stageH;
      bw = bh * a;
    }
    return { w: bw, h: bh };
  });
  let zoom = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  function resetZoom() {
    zoom = 1;
    tx = 0;
    ty = 0;
  }
  function zoomAt(nz: number, cx: number, cy: number) {
    nz = Math.min(8, Math.max(1, nz));
    // keep the point under the cursor fixed
    const k = nz / zoom;
    tx = cx - (cx - tx) * k;
    ty = cy - (cy - ty) * k;
    zoom = nz;
    if (zoom === 1) {
      tx = 0;
      ty = 0;
    }
  }
  function stageCoords(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return [e.clientX - r.left - r.width / 2, e.clientY - r.top - r.height / 2];
  }
  function onWheel(e: WheelEvent) {
    if (isVideo) return;
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      const [cx, cy] = stageCoords(e);
      zoomAt(zoom * Math.exp(-e.deltaY * 0.01), cx, cy);
    } else if (zoom > 1) {
      e.preventDefault();
      tx -= e.deltaX;
      ty -= e.deltaY;
    }
  }
  function onDbl(e: MouseEvent) {
    if (isVideo) return;
    const [cx, cy] = stageCoords(e);
    zoomAt(zoom > 1 ? 1 : 2.5, cx, cy);
  }
  let drag = $state<{ x: number; y: number; tx: number; ty: number } | null>(null);
  function onDown(e: PointerEvent) {
    if (zoom <= 1 || isVideo) return;
    drag = { x: e.clientX, y: e.clientY, tx, ty };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    poke();
    if (!drag) return;
    tx = drag.tx + e.clientX - drag.x;
    ty = drag.ty + e.clientY - drag.y;
  }

  function poke() {
    chrome = true;
    clearTimeout(chromeTimer);
    chromeTimer = setTimeout(() => (chrome = false), 2500);
  }

  // ---- Live Photos ------------------------------------------------------------
  let motionEl: HTMLVideoElement | undefined = $state();
  let motionPlaying = $state(false);
  function playMotion() {
    if (!motionEl) return;
    motionEl.currentTime = 0;
    motionEl.play().then(
      () => (motionPlaying = true),
      () => (motionPlaying = false),
    );
  }
  $effect(() => {
    if (isLive && motionEl && details?.online) {
      // play once when opened, like Google Photos
      const el = motionEl;
      if (el.readyState >= 3) playMotion();
      else el.addEventListener("canplay", () => playMotion(), { once: true });
    }
  });

  // ---- actions ----------------------------------------------------------------
  async function toggleFav() {
    if (!details) return;
    const value = !details.favorite;
    await call("set_favorite", { ids: [id], value });
    details = { ...details, favorite: value };
    cache.set(id, details);
    toast(value ? "Added to favorites" : "Removed from favorites", undefined, 2000);
  }

  async function reveal() {
    menu = false;
    await call("reveal", { id: shownId }).catch((e) => toast(String(e)));
  }
  async function openExternal() {
    menu = false;
    await call("open_external", { id: shownId }).catch((e) => toast(String(e)));
  }

  function onKey(e: KeyboardEvent) {
    if (app.exporting) return;
    const el = e.target as HTMLElement;
    if (el && el.tagName === "INPUT") return;
    if (e.key === "Escape") {
      if (menu) menu = false;
      else if (zoom > 1) resetZoom();
      else close();
    } else if (e.key === "ArrowRight") go(1);
    else if (e.key === "ArrowLeft") go(-1);
    else if (e.key === "i") info = !info;
    else if (e.key === "f" || e.key === "s") toggleFav();
    else if (e.key === "+" || e.key === "=") zoomAt(zoom * 1.5, 0, 0);
    else if (e.key === "-") zoomAt(zoom / 1.5, 0, 0);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="viewer" class:chrome onmousemove={poke} role="dialog" tabindex="-1" aria-modal="true" aria-label="Photo viewer">
  <div class="main">
    <div class="bar" data-tauri-drag-region>
      <button class="vbtn" title="Back (Esc)" onclick={close}><Icon path={mdiArrowLeft} /></button>
      <div class="flex" data-tauri-drag-region></div>
      {#if isLive}
        <button class="livechip" class:on={motionPlaying} onmouseenter={playMotion} onclick={playMotion} title="Play motion">
          <Icon path={mdiMotionPlayOutline} size={18} /> LIVE
        </button>
      {/if}
      <button class="vbtn" class:on={info} title="Info (i)" onclick={() => (info = !info)}>
        <Icon path={mdiInformationOutline} />
      </button>
      <button class="vbtn" title="Favorite (f)" onclick={toggleFav}>
        <Icon path={details?.favorite ? mdiStar : mdiStarOutline} />
      </button>
      <button class="vbtn" title="Export" onclick={() => (app.exporting = [id])}>
        <Icon path={mdiTrayArrowDown} />
      </button>
      <div class="menuwrap">
        <button class="vbtn" title="More" onclick={() => (menu = !menu)}><Icon path={mdiDotsVertical} /></button>
        {#if menu}
          <div class="menu">
            {#if isTauri}
              <button onclick={reveal}><Icon path={mdiFolderOpenOutline} size={20} /> Show in Finder</button>
              <button onclick={openExternal}><Icon path={mdiOpenInNew} size={20} /> Open with default app</button>
            {/if}
            {#if details?.originalId}
              <button
                onclick={() => {
                  showOriginal = !showOriginal;
                  fullLoaded = false;
                  menu = false;
                }}
              >
                <Icon path={mdiImageEditOutline} size={20} />
                {showOriginal ? "Show edited version" : "Show original (before edits)"}
              </button>
            {/if}
          </div>
        {/if}
      </div>
    </div>

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="stage"
      bind:clientWidth={stageW}
      bind:clientHeight={stageH}
      onwheel={onWheel}
      ondblclick={onDbl}
      onpointerdown={onDown}
      onpointermove={onMove}
      onpointerup={() => (drag = null)}
      onclick={() => (menu = false)}
    >
      <div
        class="box"
        class:zoomed={zoom > 1}
        class:dragging={drag !== null}
        style="width:{box.w}px; height:{box.h}px; transform: translate({tx}px, {ty}px) scale({zoom})"
      >
        <img class="thumb" src={mediaUrl("thumb", shownId)} alt="" draggable="false" />
        {#if isVideo}
          {#if fullSrc}
            {#key fullSrc}
              <!-- svelte-ignore a11y_media_has_caption -->
              <video class="full show" src={fullSrc} controls autoplay playsinline poster={mediaUrl("thumb", shownId)}></video>
            {/key}
          {/if}
        {:else if fullSrc}
          {#key fullSrc}
            <img
              class="full"
              class:show={fullLoaded}
              src={fullSrc}
              alt={details?.name ?? ""}
              draggable="false"
              onload={() => (fullLoaded = true)}
              onerror={() => (fullFailed = true)}
            />
          {/key}
          {#if isLive && !showOriginal}
            {#key id}
              <!-- svelte-ignore a11y_media_has_caption -->
              <video
                class="motion"
                class:show={motionPlaying}
                bind:this={motionEl}
                src={mediaUrl("motion", id)}
                muted
                playsinline
                preload="auto"
                onended={() => (motionPlaying = false)}
                onerror={() => (motionPlaying = false)}
              ></video>
            {/key}
          {/if}
        {/if}
      </div>

      {#if details && !details.online}
        <div class="notice"><Icon path={mdiAlertCircleOutline} size={20} /> The drive with the original isn’t connected — showing a preview.</div>
      {:else if isVideo && details && !details.playable}
        <div class="notice">
          <Icon path={mdiAlertCircleOutline} size={20} /> This video format ({details.ext.toUpperCase()}) can’t play here.
          {#if isTauri}<button class="link" onclick={openExternal}>Open with default app</button>{/if}
        </div>
      {:else if fullFailed}
        <div class="notice"><Icon path={mdiAlertCircleOutline} size={20} /> Couldn’t open the original file.</div>
      {/if}

      {#if index > 0}
        <button class="nav prev" title="Previous (←)" onclick={(e) => { e.stopPropagation(); go(-1); }}>
          <Icon path={mdiChevronLeft} size={32} />
        </button>
      {/if}
      {#if index < t.n - 1}
        <button class="nav next" title="Next (→)" onclick={(e) => { e.stopPropagation(); go(1); }}>
          <Icon path={mdiChevronRight} size={32} />
        </button>
      {/if}
    </div>
  </div>

  {#if info}
    <InfoPanel {details} onclose={() => (info = false)} onnavigate={close} />
  {/if}
</div>

<style>
  .viewer {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: #000;
    color: #fff;
    display: flex;
    animation: fade 0.16s ease;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .main {
    flex: 1;
    position: relative;
    min-width: 0;
  }
  .bar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 64px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 12px;
    z-index: 3;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.55), transparent);
    opacity: 0;
    transition: opacity 0.25s;
  }
  .chrome .bar,
  .bar:hover {
    opacity: 1;
  }
  .flex {
    flex: 1;
    align-self: stretch;
  }
  .vbtn {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
  }
  .vbtn:hover {
    background: rgba(255, 255, 255, 0.14);
  }
  .vbtn.on {
    background: rgba(255, 255, 255, 0.22);
  }
  .livechip {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 12px 0 8px;
    border-radius: 15px;
    background: rgba(255, 255, 255, 0.16);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    margin-right: 8px;
  }
  .livechip.on {
    background: #fff;
    color: #000;
  }
  .menuwrap {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: 48px;
    background: #2a2a2c;
    border-radius: 8px;
    padding: 8px 0;
    min-width: 260px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 10px 16px;
    text-align: left;
    color: #e8eaed;
  }
  .menu button:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .stage {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .box {
    position: relative;
    transition: transform 0.12s ease-out;
    will-change: transform;
  }
  .box.zoomed {
    cursor: grab;
  }
  .box.dragging {
    transition: none;
  }
  .box.zoomed:active {
    cursor: grabbing;
  }
  .box img,
  .box video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
  }
  .full,
  .motion {
    opacity: 0;
    transition: opacity 0.2s;
  }
  .full:global(.show),
  .motion:global(.show) {
    opacity: 1;
  }
  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 56px;
    height: 56px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: #fff;
    background: rgba(0, 0, 0, 0.25);
    opacity: 0;
    transition: opacity 0.2s, background 0.15s;
    z-index: 2;
  }
  .chrome .nav {
    opacity: 0.85;
  }
  .nav:hover {
    opacity: 1;
    background: rgba(255, 255, 255, 0.18);
  }
  .prev {
    left: 16px;
  }
  .next {
    right: 16px;
  }
  .notice {
    position: absolute;
    bottom: 32px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    background: rgba(32, 33, 36, 0.92);
    padding: 10px 16px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 2;
  }
  .link {
    color: #8ab4f8;
    font-weight: 500;
    margin-left: 4px;
  }
</style>
