<script lang="ts">
  let { size = 32 }: { size?: number } = $props();
  const uid = Math.random().toString(36).slice(2, 8);
</script>

<svg width={size} height={size} viewBox="64 64 896 896" aria-hidden="true">
  <defs>
    <linearGradient id="lg-bg-{uid}" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#3b6cf6" />
      <stop offset="1" stop-color="#7a4ff0" />
    </linearGradient>
    <linearGradient id="lg-sun-{uid}" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#ffd36b" />
      <stop offset="1" stop-color="#ff9f43" />
    </linearGradient>
    <clipPath id="lg-card-{uid}"><rect x="232" y="252" width="560" height="520" rx="72" /></clipPath>
  </defs>
  <rect x="64" y="64" width="896" height="896" rx="200" fill="url(#lg-bg-{uid})" />
  <rect x="282" y="196" width="460" height="60" rx="30" fill="#fff" opacity="0.35" />
  <g clip-path="url(#lg-card-{uid})">
    <rect x="232" y="252" width="560" height="520" fill="#eaf1ff" />
    <circle cx="628" cy="400" r="64" fill="url(#lg-sun-{uid})" />
    <path d="M232 700 L420 488 L548 628 L620 556 L792 720 L792 772 L232 772 Z" fill="#3b6cf6" />
    <path d="M232 772 L232 720 L420 560 L600 772 Z" fill="#2648b8" />
  </g>
</svg>
