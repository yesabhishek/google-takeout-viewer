<script lang="ts">
  import {
    mdiMenu,
    mdiMagnify,
    mdiClose,
    mdiArrowLeft,
    mdiStarOutline,
    mdiStar,
    mdiTrayArrowDown,
    mdiFolderPlusOutline,
    mdiMagnifyMinusOutline,
    mdiMagnifyPlusOutline,
    mdiCogOutline,
    mdiAccountOutline,
    mdiImageAlbum,
  } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import Logo from "./Logo.svelte";
  import { app, navigate, back, setRowHeight, toast } from "../lib/state.svelte";
  import { call, isTauri, isMac, mediaUrl, type Person, type Album } from "../lib/api";
  import { FLAG_FAV } from "../lib/timeline";
  import { addFolder } from "../lib/actions";
  import { plural } from "../lib/format";

  let focused = $state(false);
  let input: HTMLInputElement | undefined = $state();
  let people = $state<Person[]>([]);
  let albums = $state<Album[]>([]);
  let loadedGen = -1;
  let debounce: ReturnType<typeof setTimeout> | undefined;

  async function loadLists() {
    if (loadedGen === app.generation) return;
    loadedGen = app.generation;
    [people, albums] = await Promise.all([call<Person[]>("people"), call<Album[]>("albums")]);
  }

  const q = $derived(app.query.trim().toLowerCase());
  const peopleHits = $derived(q ? people.filter((p) => p.name.toLowerCase().includes(q)).slice(0, 4) : people.slice(0, 6));
  const albumHits = $derived(q ? albums.filter((a) => a.title.toLowerCase().includes(q)).slice(0, 4) : []);

  function onInput() {
    clearTimeout(debounce);
    debounce = setTimeout(() => {
      const v = app.query.trim();
      if (v) navigate({ kind: "search", q: v });
      else if (app.view.kind === "search") back();
    }, 280);
  }

  function submit(e: Event) {
    e.preventDefault();
    clearTimeout(debounce);
    const v = app.query.trim();
    if (v) navigate({ kind: "search", q: v });
    focused = false;
    input?.blur();
  }

  function clearSearch() {
    app.query = "";
    if (app.view.kind === "search") back();
    input?.focus();
  }

  const selCount = $derived(app.selection.size);
  const allFav = $derived.by(() => {
    const t = app.current;
    if (!t || selCount === 0) return false;
    for (const id of app.selection) {
      const i = t.index.get(id);
      if (i === undefined || !(t.flags[i] & FLAG_FAV)) return false;
    }
    return true;
  });

  async function toggleFav() {
    const ids = [...app.selection];
    await call("set_favorite", { ids, value: !allFav });
    toast(allFav ? `Removed ${plural(ids.length, "item")} from favorites` : `Added ${plural(ids.length, "item")} to favorites`);
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      input?.focus();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<header class="topbar" class:inset={isTauri && isMac} class:selecting={selCount > 0} data-tauri-drag-region>
  {#if selCount > 0}
    <button class="icon-btn" title="Clear selection (Esc)" onclick={() => app.selection.clear()}>
      <Icon path={mdiClose} />
    </button>
    <div class="sel-count" data-tauri-drag-region>{selCount} selected</div>
    <div class="spacer" data-tauri-drag-region></div>
    <button class="icon-btn" title={allFav ? "Unfavorite" : "Favorite"} onclick={toggleFav}>
      <Icon path={allFav ? mdiStar : mdiStarOutline} />
    </button>
    <button class="btn tonal" onclick={() => (app.exporting = [...app.selection])}>
      <Icon path={mdiTrayArrowDown} size={20} /> Export
    </button>
  {:else}
    <div class="left" data-tauri-drag-region>
      <button class="icon-btn" title="Menu" onclick={() => (app.sidebar = !app.sidebar)}>
        <Icon path={mdiMenu} />
      </button>
      <button class="brand" onclick={() => navigate({ kind: "photos" })}>
        <Logo size={30} />
        <span><b>Takeout</b> Viewer</span>
      </button>
    </div>

    <form class="search" class:focused onsubmit={submit} role="search">
      {#if app.view.kind === "search"}
        <button type="button" class="icon-btn small" title="Back" onclick={clearSearch}>
          <Icon path={mdiArrowLeft} size={22} />
        </button>
      {:else}
        <span class="lead"><Icon path={mdiMagnify} size={22} /></span>
      {/if}
      <input
        bind:this={input}
        bind:value={app.query}
        oninput={onInput}
        onfocus={() => {
          focused = true;
          loadLists();
        }}
        onblur={() => setTimeout(() => (focused = false), 150)}
        placeholder="Search your photos, people, albums, years"
        spellcheck="false"
        autocomplete="off"
      />
      {#if app.query}
        <button type="button" class="icon-btn small" title="Clear" onclick={clearSearch}>
          <Icon path={mdiClose} size={20} />
        </button>
      {/if}

      {#if focused && (peopleHits.length || albumHits.length)}
        <div class="suggest">
          {#if peopleHits.length}
            <div class="sg-title">People</div>
            <div class="sg-people">
              {#each peopleHits as p (p.id)}
                <button class="sg-person" onmousedown={() => navigate({ kind: "person", id: p.id, name: p.name })}>
                  {#if p.cover}
                    <img src={mediaUrl("thumb", p.cover)} alt="" />
                  {:else}
                    <span class="ph"><Icon path={mdiAccountOutline} /></span>
                  {/if}
                  <span>{p.name}</span>
                </button>
              {/each}
            </div>
          {/if}
          {#if albumHits.length}
            <div class="sg-title">Albums</div>
            {#each albumHits as a (a.id)}
              <button class="sg-row" onmousedown={() => navigate({ kind: "album", id: a.id, title: a.title })}>
                {#if a.cover}<img src={mediaUrl("thumb", a.cover)} alt="" />{:else}<Icon path={mdiImageAlbum} />{/if}
                <span>{a.title}</span>
                <span class="muted">{plural(a.count, "item")}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/if}
    </form>

    <div class="right" data-tauri-drag-region>
      <button class="icon-btn" title="Smaller thumbnails (−)" onclick={() => setRowHeight(app.rowHeight / 1.2)}>
        <Icon path={mdiMagnifyMinusOutline} />
      </button>
      <button class="icon-btn" title="Larger thumbnails (+)" onclick={() => setRowHeight(app.rowHeight * 1.2)}>
        <Icon path={mdiMagnifyPlusOutline} />
      </button>
      <button class="icon-btn" title="Add a Takeout folder" onclick={addFolder}>
        <Icon path={mdiFolderPlusOutline} />
      </button>
      <button class="icon-btn" title="Library & settings" onclick={() => navigate({ kind: "library" })}>
        <Icon path={mdiCogOutline} />
      </button>
    </div>
  {/if}
</header>

<style>
  .topbar {
    height: var(--topbar-h);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-bottom: 1px solid var(--outline-soft);
    background: var(--bg);
    position: relative;
    z-index: 20;
  }
  .topbar.inset {
    padding-left: 80px;
  }
  .topbar.selecting {
    background: var(--primary-container);
    color: var(--on-primary-container);
    border-bottom-color: transparent;
  }
  .topbar.selecting .icon-btn {
    color: var(--on-primary-container);
  }
  .sel-count {
    font-size: 18px;
    font-weight: 500;
    margin-left: 4px;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .left {
    display: flex;
    align-items: center;
    gap: 4px;
    width: calc(var(--sidebar-w) - 24px);
    flex: none;
    align-self: stretch;
  }
  .inset .left {
    width: calc(var(--sidebar-w) - 92px);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 20px;
    color: var(--text-2);
    padding: 4px 8px;
    border-radius: 8px;
    white-space: nowrap;
  }
  .brand b {
    font-weight: 600;
    color: var(--text);
  }
  .search {
    flex: 1;
    max-width: 720px;
    height: 48px;
    border-radius: 24px;
    background: var(--surface-2);
    display: flex;
    align-items: center;
    padding: 0 4px 0 12px;
    gap: 4px;
    position: relative;
    transition: background 0.15s, box-shadow 0.15s;
  }
  .search.focused {
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  .search .lead {
    color: var(--text-2);
    display: grid;
    place-items: center;
    width: 32px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    font-size: 16px;
    height: 100%;
    -webkit-user-select: text;
    user-select: text;
  }
  .search input::placeholder {
    color: var(--text-3);
  }
  .icon-btn.small {
    width: 36px;
    height: 36px;
  }
  .right {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 2px;
    padding-left: 16px;
    align-self: stretch;
  }
  .suggest {
    position: absolute;
    top: 52px;
    left: 0;
    right: 0;
    background: var(--bg);
    border-radius: 16px;
    box-shadow: var(--shadow-lg);
    padding: 8px 0 12px;
    z-index: 40;
  }
  .sg-title {
    padding: 10px 20px 6px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .sg-people {
    display: flex;
    gap: 8px;
    padding: 4px 16px;
    overflow-x: auto;
  }
  .sg-person {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 84px;
    padding: 6px 2px;
    border-radius: 12px;
    font-size: 12px;
    text-align: center;
  }
  .sg-person:hover,
  .sg-row:hover {
    background: var(--surface-2);
  }
  .sg-person img,
  .sg-person .ph {
    width: 56px;
    height: 56px;
    border-radius: 50%;
    object-fit: cover;
    background: var(--placeholder);
    display: grid;
    place-items: center;
    color: var(--text-3);
  }
  .sg-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 6px 20px;
    text-align: left;
  }
  .sg-row img {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    object-fit: cover;
  }
  .sg-row span:nth-child(2) {
    flex: 1;
  }
</style>
