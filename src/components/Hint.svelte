<script lang="ts">
  import type { HelpId } from "../lib/glossary";
  import { t, type Key } from "../lib/i18n.svelte";

  let { id }: { id: HelpId } = $props();
  let open = $state(false);
  const uid = $props.id();
</script>

<div class="hint">
  <p>
    {t(`help.${id}.short` as Key)}
    <button type="button" class="q" aria-expanded={open} aria-controls="{uid}-d" aria-label={t("hint.more")} onclick={() => (open = !open)}>?</button>
  </p>
  {#if open}<p id="{uid}-d" class="detail">{t(`help.${id}.detail` as Key)}</p>{/if}
</div>

<style>
  .hint { font-size: var(--text-sm); color: var(--muted); }
  .q {
    display: inline-grid; place-items: center; width: 20px; height: 20px; margin-left: 4px; padding: 0; vertical-align: 1px;
    border: 1px solid var(--border); border-radius: 50%; background: var(--surface-2); color: var(--text);
    font-size: var(--text-xs); font-weight: 700; cursor: pointer;
    transition: background-color var(--dur-micro), border-color var(--dur-micro);
  }
  .q:hover { background: var(--surface-3); border-color: var(--border-strong); }
  .q[aria-expanded="true"] { background: var(--text); color: var(--bg); border-color: var(--text); }
  .detail { margin-top: 8px; padding: 8px 12px; border-left: 2px solid var(--border-strong); background: var(--surface-2); color: var(--text); max-width: 70ch; }
</style>
