<script lang="ts">
  let { path, size = 24, class: cls = "" }: { path: string; size?: number; class?: string } = $props();
</script>

<svg class="icon {cls}" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
  <path d={path} fill="currentColor" />
</svg>

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
