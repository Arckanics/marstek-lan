<script lang="ts" module>
  const PATHS = {
    charge: "M13 2 4 14h7l-1 8 9-12h-7z",
    discharge: "M13 2 4 14h7l-1 8 9-12h-7z M3 3l18 18",
    auto: "M20 12a8 8 0 0 0-14-5.3L4 9 M4 4v5h5 M4 12a8 8 0 0 0 14 5.3L20 15 M20 20v-5h-5",
    ai: "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z M19 16v4 M17 18h4",
    ups: "M12 3 4 6v6c0 4.5 3.2 8 8 9 4.8-1 8-4.5 8-9V6z M9 12l2 2 4-4",
    erase: "m7 21-4.3-4.3a1 1 0 0 1 0-1.4l9.6-9.6a1 1 0 0 1 1.4 0l5.6 5.6a1 1 0 0 1 0 1.4L13 21 M22 21H7 M5 11l9 9",
    solar: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8z M12 2v2 M12 20v2 M2 12h2 M20 12h2 M4.9 4.9l1.4 1.4 M17.7 17.7l1.4 1.4 M4.9 19.1l1.4-1.4 M17.7 6.3l1.4-1.4",
    grid: "M12 2 7 22 M12 2l5 20 M8.5 9h7 M7.3 15h9.4 M5 6h14",
    plug: "M9 2v5 M15 2v5 M6 7h12v4a6 6 0 0 1-12 0z M12 17v5",
    house: "M3 11.5 12 4l9 7.5 M5 10v10h14V10",
    battery: "M4 7h13a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z M22 10v4",
    devices: "M5 12.5a10 10 0 0 1 14 0 M8.5 16a5 5 0 0 1 7 0 M12 19.5h.01",
    status: "M3 12h4l3-8 4 16 3-8h4",
    modes: "M12 3 3 8l9 5 9-5z M3 13l9 5 9-5",
    planning: "M5 5h14a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z M4 10h16 M8 3v4 M16 3v4",
    history: "M3 3v18h18 M7 15l4-4 3 3 5-6",
    device: "M7 7h10v10H7z M10 10h4v4h-4z M9 2v3 M15 2v3 M9 19v3 M15 19v3 M2 9h3 M2 15h3 M19 9h3 M19 15h3",
    settings: "M4 6h9 M17 6h3 M15 4v4 M4 12h3 M11 12h9 M9 10v4 M4 18h11 M19 18h1 M17 16v4",
  };

  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  /** Icônes décoratives (le texte voisin porte le sens) ; `level` remplit la batterie (0 à 100). */
  let { name, size = 16, level }: { name: IconName; size?: number; level?: number } = $props();
</script>

<svg class="icon" viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d={PATHS[name]} />
  {#if name === "battery" && level != null}<rect x="4.25" y="9.25" width={Math.max(0, Math.min(100, level)) * 0.125} height="5.5" rx="0.5" fill="currentColor" stroke="none" />{/if}
</svg>

<style>
  .icon { flex: none; }
</style>
