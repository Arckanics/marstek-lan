<script lang="ts">
  let { days = $bindable(), label = "Jours" }: { days: number; label?: string } = $props();

  const names = ["Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche"];
</script>

<div class="seg days" role="group" aria-label={label}>
  {#each names as n, i}
    <button type="button" aria-pressed={(days & (1 << i)) !== 0} aria-label={n} title={n} onclick={() => (days ^= 1 << i)}>{n[0]}</button>
  {/each}
</div>

<style>
  .days button { min-width: 30px; padding: 0 6px; }
</style>
