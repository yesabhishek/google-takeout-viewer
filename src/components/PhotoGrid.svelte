<script lang="ts">
  import type { Snippet } from "svelte";
  import { untrack } from "svelte";
  import { mdiCheckCircle, mdiCheck, mdiStar, mdiMotionPlayOutline, mdiPlay, mdiImageBrokenVariant } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { callBin, mediaUrl } from "../lib/api";
  import {
    decode,
    layout,
    firstVisible,
    lastVisible,
    HEADER_H,
    FLAG_VIDEO,
    FLAG_LIVE,
    FLAG_FAV,
    type Timeline,
    type Layout,
  } from "../lib/timeline";
  import { app, setRowHeight } from "../lib/state.svelte";
  import { dayLabel, fmtDuration, monthYear } from "../lib/format";

  let {
    filter,
    emptyTitle = "No photos here yet",
    emptyText = "",
    children,
  }: { filter: Record<string, unknown>; emptyTitle?: string; emptyText?: string; children?: Snippet } = $props();

  const PAD_L = 16;
  const PAD_R = 60;
  const PAD_TOP = 4;

  let scroller: HTMLDivElement | undefined = $state();
  let clientW = $state(0);
  let viewH = $state(0);
  let headH = $state(0);
  let scrollTop = $state(0);
  let tl = $state.raw<Timeline | null>(null);
  let loading = $state(true);
  let error = $state("");

  // Re-layout at most every ~90ms while the window is being resized: a full
  // justified layout of 100k+ items costs tens of milliseconds.
  let layoutW = $state(0);
  let resizeTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const w = clientW;
    if (layoutW === 0 || !tl || tl.n < 5000) {
      layoutW = w;
      return;
    }
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => (layoutW = w), 90);
  });

  const gridTop = $derived(headH + PAD_TOP);
  const rowH = $derived(Math.max(90, Math.min(app.rowHeight, (layoutW - PAD_L - PAD_R) / 2.2)));
  const L = $derived.by<Layout | null>(() => {
    if (!tl || layoutW <= 0) return null;
    return layout(tl, layoutW - PAD_L - PAD_R, rowH);
  });
  const totalH = $derived(gridTop + (L?.height ?? 0) + 48);
  const maxScroll = $derived(Math.max(1, totalH - viewH));

  // ---- data -------------------------------------------------------------
  let seq = 0;
  async function load() {
    const my = ++seq;
    try {
      const buf = await callBin("timeline", { filter });
      if (my !== seq) return;
      const t = decode(buf);
      tl = t;
      app.current = t;
      error = "";
      // drop selected ids that vanished
      for (const id of app.selection) if (!t.index.has(id)) app.selection.delete(id);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void app.generation;
    untrack(load);
  });

  // ---- scroll anchoring: keep the top item steady across relayouts -------
  let anchor: { id: number; ratio: number } | null = null;
  function remember() {
    if (!L || !tl || !scroller || scroller.scrollTop < 4) {
      anchor = null;
      return;
    }
    const top = scroller.scrollTop - gridTop;
    const i = firstVisible(L, top);
    anchor = i < tl.n ? { id: tl.ids[i], ratio: (top - L.y[i]) / L.th[i] } : null;
  }
  $effect(() => {
    const l = L;
    const t = tl;
    untrack(() => {
      if (!l || !t || !scroller || !anchor) return;
      const i = t.index.get(anchor.id);
      if (i !== undefined) scroller.scrollTop = l.y[i] + gridTop + anchor.ratio * l.th[i];
    });
  });

  let raf = 0;
  let scrolling = $state(false);
  let scrollTimer: ReturnType<typeof setTimeout> | undefined;
  function onScroll() {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      if (!scroller) return;
      scrollTop = scroller.scrollTop;
      remember();
      scrolling = true;
      clearTimeout(scrollTimer);
      scrollTimer = setTimeout(() => (scrolling = false), 900);
    });
  }

  // ---- virtualization ------------------------------------------------------
  const range = $derived.by(() => {
    if (!L) return [0, 0];
    const top = scrollTop - gridTop - viewH * 0.75;
    const bottom = scrollTop - gridTop + viewH * 1.75;
    return [firstVisible(L, top), lastVisible(L, bottom)];
  });
  const visible = $derived.by(() => {
    const [a, b] = range;
    const out: number[] = [];
    for (let i = a; i < b; i++) out.push(i);
    return out;
  });
  const visibleHeaders = $derived.by(() => {
    if (!L) return [];
    const top = scrollTop - gridTop - viewH * 0.75;
    const bottom = scrollTop - gridTop + viewH * 1.75;
    return L.headers.filter((h) => h.y + HEADER_H + rowH * 1.6 >= top && h.y <= bottom);
  });

  // ---- reveal the item the viewer was showing ------------------------------
  $effect(() => {
    const id = app.revealId;
    if (id === null || !L || !tl || !scroller) return;
    untrack(() => {
      const i = tl!.index.get(id);
      app.revealId = null;
      if (i === undefined) return;
      const y = L!.y[i] + gridTop;
      if (y < scroller!.scrollTop || y + L!.th[i] > scroller!.scrollTop + viewH) {
        scroller!.scrollTop = y - viewH / 3;
      }
    });
  });

  // ---- interaction -----------------------------------------------------------
  let lastClicked = -1;
  function toggle(i: number, shift: boolean) {
    if (!tl) return;
    const id = tl.ids[i];
    if (shift && lastClicked >= 0) {
      const [a, b] = lastClicked < i ? [lastClicked, i] : [i, lastClicked];
      for (let k = a; k <= b; k++) app.selection.add(tl.ids[k]);
    } else if (app.selection.has(id)) {
      app.selection.delete(id);
    } else {
      app.selection.add(id);
    }
    lastClicked = i;
  }

  function onClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (!tl) return;
    const head = target.closest<HTMLElement>("[data-first]");
    if (head && target.closest(".hcheck")) {
      const a = Number(head.dataset.first);
      const b = Number(head.dataset.last);
      let all = true;
      for (let k = a; k < b; k++) if (!app.selection.has(tl.ids[k])) all = false;
      for (let k = a; k < b; k++) {
        if (all) app.selection.delete(tl.ids[k]);
        else app.selection.add(tl.ids[k]);
      }
      return;
    }
    const tile = target.closest<HTMLElement>("[data-i]");
    if (!tile) return;
    const i = Number(tile.dataset.i);
    if (target.closest(".check") || app.selection.size > 0 || e.metaKey || e.ctrlKey) {
      toggle(i, e.shiftKey);
      return;
    }
    lastClicked = i;
    app.viewer = { timeline: tl, index: i };
  }

  function dayAllSelected(a: number, b: number): boolean {
    if (!tl || app.selection.size === 0) return false;
    for (let k = a; k < b; k++) if (!app.selection.has(tl.ids[k])) return false;
    return true;
  }

  function onKey(e: KeyboardEvent) {
    if (app.viewer || app.exporting) return;
    const el = e.target as HTMLElement;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
    if (e.key === "Escape" && app.selection.size) {
      app.selection.clear();
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "a" && tl) {
      e.preventDefault();
      for (let i = 0; i < tl.n; i++) app.selection.add(tl.ids[i]);
    } else if (e.key === "+" || e.key === "=") {
      setRowHeight(app.rowHeight * 1.2);
    } else if (e.key === "-") {
      setRowHeight(app.rowHeight / 1.2);
    }
  }

  function onWheel(e: WheelEvent) {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      setRowHeight(app.rowHeight * Math.exp(-e.deltaY * 0.01));
    }
  }

  function imgLoaded(e: Event) {
    (e.currentTarget as HTMLImageElement).classList.add("ok");
  }
  function imgError(e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    const n = Number(img.dataset.retry || 0);
    if (n >= 5) {
      img.classList.add("broken");
      return;
    }
    img.dataset.retry = String(n + 1);
    setTimeout(() => {
      if (img.isConnected) img.src = mediaUrl("thumb", Number(img.dataset.id), `r${n + 1}`);
    }, 250 * (n + 1) * (n + 1));
  }

  // ---- scrubber ------------------------------------------------------------
  const showScrubber = $derived(!!L && totalH > viewH * 2.5);
  const yearMarks = $derived.by(() => {
    if (!L || viewH <= 0) return [];
    const marks: { year: number; top: number }[] = [];
    let last = -100;
    for (const y of L.years) {
      const top = Math.max(12, Math.min(viewH - 16, ((y.y + gridTop) / maxScroll) * viewH));
      if (top - last >= 20) {
        marks.push({ year: y.year, top });
        last = top;
      }
    }
    return marks;
  });
  const thumbTop = $derived(Math.min(viewH - 2, (scrollTop / maxScroll) * viewH));
  let hoverY = $state<number | null>(null);
  let dragging = $state(false);

  function labelAt(scrollPos: number): string {
    if (!L || !tl || tl.n === 0) return "";
    const i = Math.min(tl.n - 1, firstVisible(L, scrollPos - gridTop + 1));
    return monthYear(tl.ts[i]);
  }
  const hoverLabel = $derived(hoverY === null ? "" : labelAt((hoverY / viewH) * maxScroll));
  const currentLabel = $derived(labelAt(scrollTop));

  function scrubTo(e: PointerEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const y = Math.max(0, Math.min(r.height, e.clientY - r.top));
    hoverY = y;
    if (scroller) scroller.scrollTop = (y / r.height) * maxScroll;
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="wrap">
  <div
    class="scroller"
    class:selecting={app.selection.size > 0}
    bind:this={scroller}
    bind:clientWidth={clientW}
    bind:clientHeight={viewH}
    onscroll={onScroll}
    onwheel={onWheel}
  >
    <div class="head" bind:offsetHeight={headH}>
      {@render children?.()}
    </div>

    {#if error}
      <div class="empty"><div class="et">Couldn’t load this view</div><div class="muted">{error}</div></div>
    {:else if tl && tl.n === 0 && !loading}
      <div class="empty">
        <div class="et">{emptyTitle}</div>
        {#if emptyText}<div class="muted">{emptyText}</div>{/if}
      </div>
    {/if}

    {#if L && tl}
      {@const t = tl}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="canvas" style="height:{L.height + 48}px; margin-left:{PAD_L}px; width:{L.width}px" onclick={onClick}>
        {#each visibleHeaders as h (h.first)}
          {@const all = dayAllSelected(h.first, h.last)}
          <div class="dh" class:all style="left:{h.x}px; top:{h.y}px; max-width:{Math.max(h.width, 160)}px" data-first={h.first} data-last={h.last}>
            <button class="hcheck" title="Select day">
              <Icon path={all ? mdiCheckCircle : mdiCheck} size={all ? 22 : 16} />
            </button>
            <span>{dayLabel(h.day)}</span>
          </div>
        {/each}
        {#each visible as i (t.ids[i])}
          {@const id = t.ids[i]}
          {@const f = t.flags[i]}
          {@const sel = app.selection.has(id)}
          <div
            class="tile"
            class:sel
            data-i={i}
            style="transform:translate({L.x[i]}px,{L.y[i]}px); width:{L.tw[i]}px; height:{L.th[i]}px"
          >
            <img
              src={mediaUrl("thumb", id)}
              alt=""
              draggable="false"
              decoding="async"
              data-id={id}
              onload={imgLoaded}
              onerror={imgError}
            />
            <span class="brokenmark"><Icon path={mdiImageBrokenVariant} size={28} /></span>
            {#if f & FLAG_VIDEO}
              <span class="badge">{fmtDuration(t.dur[i])}<Icon path={mdiPlay} size={18} /></span>
            {:else if f & FLAG_LIVE}
              <span class="badge"><Icon path={mdiMotionPlayOutline} size={18} /></span>
            {/if}
            {#if f & FLAG_FAV}
              <span class="fav"><Icon path={mdiStar} size={18} /></span>
            {/if}
            <button class="check" title="Select" tabindex="-1">
              <Icon path={sel ? mdiCheckCircle : mdiCheck} size={sel ? 24 : 16} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  {#if showScrubber}
    <div
      class="scrub"
      class:active={dragging || hoverY !== null}
      role="scrollbar"
      aria-controls="grid"
      aria-valuenow={Math.round((scrollTop / maxScroll) * 100)}
      tabindex="-1"
      onpointerdown={(e) => {
        dragging = true;
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        scrubTo(e);
      }}
      onpointermove={(e) => {
        if (dragging) scrubTo(e);
        else {
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          hoverY = e.clientY - r.top;
        }
      }}
      onpointerup={() => (dragging = false)}
      onpointerleave={() => {
        if (!dragging) hoverY = null;
      }}
    >
      {#each yearMarks as m (m.year)}
        <div class="yr" style="top:{m.top}px">{m.year}</div>
      {/each}
      <div class="pos" style="top:{thumbTop}px"></div>
      {#if hoverY !== null}
        <div class="hline" style="top:{hoverY}px"></div>
        <div class="bubble" style="top:{hoverY}px">{hoverLabel}</div>
      {:else if scrolling && currentLabel}
        <div class="bubble soft" style="top:{thumbTop}px">{currentLabel}</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    height: 100%;
  }
  .scroller {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    padding-top: 4px;
    scrollbar-width: none;
  }
  .scroller::-webkit-scrollbar {
    display: none;
  }
  .canvas {
    position: relative;
    contain: strict;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 120px 24px;
    text-align: center;
  }
  .et {
    font-size: 20px;
    color: var(--text-2);
  }

  .dh {
    position: absolute;
    height: 52px;
    display: flex;
    align-items: center;
    font-size: 14px;
    font-weight: 500;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    padding-top: 8px;
  }
  .hcheck {
    width: 0;
    height: 24px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    color: var(--text-3);
    overflow: hidden;
    transition: width 0.15s, margin 0.15s;
    flex: none;
  }
  .dh:hover .hcheck,
  .selecting .hcheck,
  .dh.all .hcheck {
    width: 24px;
    margin-right: 8px;
    border: 1.5px solid var(--text-3);
  }
  .dh.all .hcheck {
    border: 0;
    color: var(--primary);
  }

  .tile {
    position: absolute;
    left: 0;
    top: 0;
    overflow: hidden;
    background: var(--placeholder);
    cursor: pointer;
    will-change: transform;
  }
  .tile img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    opacity: 0;
    transition:
      opacity 0.22s ease,
      transform 0.16s ease,
      border-radius 0.16s ease;
  }
  .tile img:global(.ok) {
    opacity: 1;
  }
  .brokenmark {
    position: absolute;
    inset: 0;
    display: none;
    place-items: center;
    color: var(--text-3);
  }
  .tile img:global(.broken) + .brokenmark {
    display: grid;
  }
  .tile::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(to bottom, rgba(0, 0, 0, 0.38), transparent 34%);
    opacity: 0;
    transition: opacity 0.15s;
    pointer-events: none;
  }
  .tile:hover::after,
  .selecting .tile::after {
    opacity: 1;
  }
  .tile.sel {
    background: var(--primary-container);
  }
  .tile.sel img {
    transform: scale(0.86);
    border-radius: 6px;
  }
  .tile.sel::after {
    opacity: 0;
  }
  .check {
    position: absolute;
    top: 6px;
    left: 6px;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1.5px solid rgba(255, 255, 255, 0.92);
    color: rgba(255, 255, 255, 0.95);
    display: grid;
    place-items: center;
    opacity: 0;
    transition: opacity 0.12s, background 0.12s;
    z-index: 2;
  }
  .check:hover {
    background: rgba(255, 255, 255, 0.25);
  }
  .tile:hover .check,
  .selecting .check {
    opacity: 1;
  }
  .tile.sel .check {
    opacity: 1;
    border: 0;
    color: var(--primary);
    background: var(--bg);
  }
  .badge {
    position: absolute;
    top: 6px;
    right: 6px;
    display: flex;
    align-items: center;
    gap: 2px;
    color: #fff;
    font-size: 13px;
    font-weight: 500;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.6);
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.5));
    z-index: 1;
    pointer-events: none;
  }
  .fav {
    position: absolute;
    left: 6px;
    bottom: 6px;
    color: #fff;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
    z-index: 1;
    pointer-events: none;
  }

  .scrub {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 56px;
    cursor: row-resize;
    touch-action: none;
    z-index: 5;
  }
  .yr {
    position: absolute;
    right: 10px;
    font-size: 11px;
    color: var(--text-3);
    transform: translateY(-50%);
    pointer-events: none;
    opacity: 0.75;
    transition: opacity 0.15s;
  }
  .scrub.active .yr {
    opacity: 1;
  }
  .pos {
    position: absolute;
    right: 6px;
    width: 26px;
    height: 2px;
    background: var(--primary);
    border-radius: 1px;
    pointer-events: none;
  }
  .hline {
    position: absolute;
    right: 0;
    width: 56px;
    height: 1px;
    background: var(--text-3);
    pointer-events: none;
  }
  .bubble {
    position: absolute;
    right: 60px;
    transform: translateY(-50%);
    background: var(--bg);
    color: var(--text);
    padding: 6px 12px;
    border-radius: 16px;
    box-shadow: var(--shadow);
    white-space: nowrap;
    font-weight: 500;
    font-size: 13px;
    pointer-events: none;
  }
  .bubble.soft {
    background: var(--primary-container);
    color: var(--on-primary-container);
  }
</style>
