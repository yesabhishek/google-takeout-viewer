<script lang="ts">
  // The app icon (src-tauri/icons/icon-source.svg) without its drop shadow:
  // your photos rising out of an opened Takeout box.
  let { size = 32 }: { size?: number } = $props();
  const uid = Math.random().toString(36).slice(2, 8);
</script>

<svg width={size} height={size} viewBox="100 100 824 824" aria-hidden="true">
  <defs>
    <linearGradient id="lg-bg-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#21468C" />
      <stop offset="1" stop-color="#0C1F44" />
    </linearGradient>
    <radialGradient id="lg-glow-{uid}" cx="0.3" cy="0.2" r="0.8">
      <stop offset="0" stop-color="#5B8CFF" stop-opacity="0.45" />
      <stop offset="1" stop-color="#5B8CFF" stop-opacity="0" />
    </radialGradient>
    <linearGradient id="lg-box-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FFB347" />
      <stop offset="1" stop-color="#F2711C" />
    </linearGradient>
    <linearGradient id="lg-flap-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FFD08A" />
      <stop offset="1" stop-color="#FFB04A" />
    </linearGradient>
    <linearGradient id="lg-sky-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#6CC4FF" />
      <stop offset="1" stop-color="#D7F0FF" />
    </linearGradient>
    <linearGradient id="lg-dusk-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FF9A8B" />
      <stop offset="1" stop-color="#FFD3A5" />
    </linearGradient>
    <clipPath id="lg-body-{uid}"><rect x="100" y="100" width="824" height="824" rx="186" /></clipPath>
  </defs>
  <g clip-path="url(#lg-body-{uid})">
    <rect x="100" y="100" width="824" height="824" fill="url(#lg-bg-{uid})" />
    <rect x="100" y="100" width="824" height="824" fill="url(#lg-glow-{uid})" />
    <rect x="250" y="548" width="524" height="80" rx="24" fill="#B4500F" />
    <g transform="rotate(-13 420 430)">
      <rect x="276" y="276" width="288" height="320" rx="22" fill="#F4F7FB" />
      <rect x="300" y="300" width="240" height="220" rx="10" fill="url(#lg-dusk-{uid})" />
      <path d="M300 470 L370 410 L430 455 L480 420 L540 470 L540 520 L300 520 Z" fill="#E8590C" />
    </g>
    <g transform="rotate(9 580 420)">
      <rect x="420" y="236" width="320" height="360" rx="24" fill="#FFFFFF" />
      <rect x="446" y="262" width="268" height="250" rx="12" fill="url(#lg-sky-{uid})" />
      <circle cx="652" cy="322" r="34" fill="#FFD43B" />
      <path d="M446 470 L530 380 L600 450 L640 412 L714 480 L714 512 L446 512 Z" fill="#2F9E44" />
      <path d="M446 512 L446 488 L540 430 L640 512 Z" fill="#237A35" />
    </g>
    <path d="M236 600 L420 600 L372 512 L170 512 Z" fill="url(#lg-flap-{uid})" />
    <path d="M788 600 L604 600 L652 512 L854 512 Z" fill="url(#lg-flap-{uid})" />
    <rect x="236" y="584" width="552" height="236" rx="44" fill="url(#lg-box-{uid})" />
    <rect x="236" y="584" width="552" height="22" rx="11" fill="#FFFFFF" opacity="0.25" />
  </g>
</svg>
