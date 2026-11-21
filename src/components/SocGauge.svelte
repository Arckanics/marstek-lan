<script lang="ts">
  let { soc, bat }: { soc: number | undefined; bat: number | null } = $props();

  const R = 52;
  const C = 2 * Math.PI * R;
  const pct = $derived(Math.max(0, Math.min(100, soc ?? 0)));
  const state = $derived(bat == null || Math.abs(bat) < 10 ? "En veille" : bat > 0 ? "En charge" : "En décharge");
  const color = $derived(bat == null || Math.abs(bat) < 10 ? "var(--muted)" : bat > 0 ? "var(--charge)" : "var(--discharge)");
</script>

<div class="gauge" role="meter" aria-label="Niveau de charge" aria-valuemin="0" aria-valuemax="100" aria-valuenow={soc} aria-valuetext={soc == null ? "inconnu" : `${pct} %`}>
  <svg viewBox="0 0 120 120" aria-hidden="true">
    <circle cx="60" cy="60" r={R} class="track" />
    <circle cx="60" cy="60" r={R} class="arc" stroke={color} stroke-dasharray="{(C * pct) / 100} {C}" transform="rotate(-90 60 60)" />
  </svg>
  <div class="txt">
    <strong class="num">{soc == null ? "—" : Math.round(pct)}<small>%</small></strong>
    <span class="muted">{state}</span>
  </div>
</div>

<style>
  .gauge { position: relative; width: 140px; height: 140px; margin: 0 auto; }
  svg { width: 100%; height: 100%; }
  .track { fill: none; stroke: var(--surface-2); stroke-width: 10; }
  .arc { fill: none; stroke-width: 10; stroke-linecap: round; transition: stroke-dasharray 0.6s ease, stroke 0.3s; }
  .txt { position: absolute; inset: 0; display: grid; place-content: center; text-align: center; }
  strong { font-size: 2rem; line-height: 1; }
  small { font-size: 1rem; font-weight: 500; margin-left: 2px; }
  .txt span { font-size: 0.8rem; margin-top: 4px; }
</style>
