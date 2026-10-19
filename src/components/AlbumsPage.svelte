<script lang="ts">
  import { untrack } from "svelte";
  import { mdiImageAlbum } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { call, mediaUrl, type Album } from "../lib/api";
  import { app, navigate } from "../lib/state.svelte";
  import { plural, fmtRange } from "../lib/format";

  let albums = $state<Album[] | null>(null);
  $effect(() => {
    void app.generation;
    untrack(async () => (albums = await call<Album[]>("albums")));
  });
</script>

<div class="page">
  <h1 class="page-title">Albums</h1>
  {#if albums && albums.length === 0}
    <div class="empty">
      <div class="ring"><Icon path={mdiImageAlbum} size={48} /></div>
      <div class="et">No albums yet</div>
      <p class="muted">Album folders from your Takeout (the ones with a metadata.json) appear here.</p>
    </div>
  {:else if albums}
    <div class="grid">
      {#each albums as a (a.id)}
        <button class="card" onclick={() => navigate({ kind: "album", id: a.id, title: a.title })}>
          <div class="cover">
            {#if a.cover}
              <img src={mediaUrl("thumb", a.cover)} alt="" loading="lazy" />
            {:else}
              <Icon path={mdiImageAlbum} size={48} />
            {/if}
          </div>
          <div class="title">{a.title}</div>
          <div class="meta">{plural(a.count, "item")}{#if a.start} · {fmtRange(a.start, a.end)}{/if}</div>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 28px 20px;
  }
  .card {
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cover {
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    background: var(--placeholder);
    display: grid;
    place-items: center;
    color: var(--text-3);
    margin-bottom: 8px;
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    transition: transform 0.25s ease;
  }
  .card:hover .cover img {
    transform: scale(1.04);
  }
  .title {
    font-weight: 500;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font-size: 12px;
    color: var(--text-3);
  }
  .empty {
    max-width: 440px;
    margin: 80px auto;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .ring {
    width: 96px;
    height: 96px;
    border-radius: 50%;
    background: var(--surface-2);
    display: grid;
    place-items: center;
    color: var(--text-3);
    margin-bottom: 8px;
  }
  .et {
    font-size: 20px;
  }
</style>
