<script lang="ts">
  let { name, size = 20 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    Profiles: 'M3 7h7l2 2h9v11H3ZM3 7V4h7l2 3',
    Repositories: 'M4 4h16v6H4ZM4 14h16v6H4ZM7 7h.01M7 17h.01',
    Snapshots: 'M3 11a9 9 0 1 1 2 7M3 4v7h7M12 7v6l4 2',
    Settings: 'M4 7h16M4 17h16M8 4v6M16 14v6',
    shield: 'M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6ZM8 12l3 3 5-6',
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.65"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name] ?? paths.shield} />
</svg>
