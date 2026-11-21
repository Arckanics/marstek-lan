<script lang="ts">
  import { app, clock } from "../lib/state.svelte";
  import { ageOf, fmtAge, STALE_S } from "../lib/link";
  import type { Section } from "../lib/api";

  let { keys }: { keys: Section[] } = $props();
  const age = $derived(app.snapshot ? ageOf(app.snapshot, keys, clock.now) : undefined);
</script>

{#if age != null && age > STALE_S}
  <span class="badge" role="status"><span class="dot poor"></span>Données d'il y a {fmtAge(age)}</span>
{/if}
