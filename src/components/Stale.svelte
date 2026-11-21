<script lang="ts">
  import { app, clock } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";
  import { ageOf, fmtAge, staleAfter } from "../lib/link";
  import type { Section } from "../lib/api";

  let { keys }: { keys: Section[] } = $props();
  const age = $derived(app.snapshot ? ageOf(app.snapshot, keys, clock.now) : undefined);
</script>

{#if age != null && app.snapshot && age > staleAfter(app.snapshot)}
  <span class="badge tone k-warn" role="status">{t("stale.badge", { age: fmtAge(age) })}</span>
{/if}
