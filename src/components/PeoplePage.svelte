<script lang="ts">
  import { untrack } from "svelte";
  import { mdiAccountOutline } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import { call, mediaUrl, type Person } from "../lib/api";
  import { app, navigate } from "../lib/state.svelte";
  import { plural } from "../lib/format";

  let people = $state<Person[] | null>(null);
  $effect(() => {
    void app.generation;
    untrack(async () => (people = await call<Person[]>("people")));
  });
</script>

<div class="page">
  <h1 class="page-title">People</h1>
  {#if people && people.length === 0}
    <div class="empty">
      <div class="ring"><Icon path={mdiAccountOutline} size={48} /></div>
      <div class="et">No people yet</div>
      <p class="muted">
        Google Takeout only exports the people you named in Google Photos. Once a face group has a name, its photos show
        up here.
      </p>
    </div>
  {:else if people}
    <div class="grid">
      {#each people as p (p.id)}
        <button class="person" onclick={() => navigate({ kind: "person", id: p.id, name: p.name })}>
          {#if p.cover}
            <img src={mediaUrl("thumb", p.cover)} alt="" loading="lazy" />
          {:else}
            <span class="ph"><Icon path={mdiAccountOutline} size={40} /></span>
          {/if}
          <span class="name">{p.name}</span>
          <span class="count">{plural(p.count, "item")}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(148px, 1fr));
    gap: 24px 16px;
  }
  .person {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 12px 4px;
    border-radius: 16px;
    transition: background 0.15s;
  }
  .person:hover {
    background: var(--surface-2);
  }
  .person img,
  .ph {
    width: 128px;
    height: 128px;
    border-radius: 50%;
    object-fit: cover;
    background: var(--placeholder);
    margin-bottom: 8px;
    display: grid;
    place-items: center;
    color: var(--text-3);
    transition: transform 0.2s;
  }
  .person:hover img {
    transform: scale(1.03);
  }
  .name {
    font-weight: 500;
    text-align: center;
  }
  .count {
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
