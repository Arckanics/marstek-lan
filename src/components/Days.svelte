<script lang="ts">
  import { dayName, t } from "../lib/i18n.svelte";

  let { days = $bindable(), label }: { days: number; label?: string } = $props();
</script>

<div class="seg days" role="group" aria-label={label ?? t("common.days")}>
  {#each { length: 7 } as _, i}
    {@const n = dayName(i)}
    <button type="button" aria-pressed={(days & (1 << i)) !== 0} aria-label={n} title={n} onclick={() => (days ^= 1 << i)}>{n[0]}</button>
  {/each}
</div>

<style>
  .days button { min-width: 30px; padding: 0 8px; }
</style>
