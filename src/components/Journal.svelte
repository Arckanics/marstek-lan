<script lang="ts">
  import { tick } from "svelte";
  import { app } from "../lib/state.svelte";

  let open = $state(false);
  let box: HTMLElement | undefined = $state();

  $effect(() => {
    app.logs.length;
    if (open) tick().then(() => box && (box.scrollTop = box.scrollHeight));
  });
</script>

<section class="journal" aria-label="Journal">
  <button class="head" aria-expanded={open} aria-controls="journal-body" onclick={() => (open = !open)}>
    <svg viewBox="0 0 24 24" aria-hidden="true" class:open><path d="m9 6 6 6-6 6" /></svg>
    Journal
    <span class="muted num">{app.logs.length} message{app.logs.length > 1 ? "s" : ""}</span>
    {#if !open && app.logs.length}<span class="last">{app.logs[app.logs.length - 1]}</span>{/if}
  </button>
  {#if open}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex : région défilante, atteignable au clavier -->
    <div id="journal-body" class="body num" bind:this={box} role="log" tabindex="0" aria-label="Messages du journal">
      {#each app.logs as l}<div>{l}</div>{:else}<div class="muted">Aucun message pour l'instant.</div>{/each}
    </div>
  {/if}
</section>

<style>
  .journal { background: var(--surface); border-top: 1px solid var(--border); }
  .head {
    display: flex; align-items: center; gap: 10px; width: 100%; min-height: 36px; padding: 0 20px;
    background: none; border: 0; cursor: pointer; text-align: left; font-weight: 500;
  }
  .head:hover { background: var(--surface-2); }
  svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; transition: transform 0.15s; }
  svg.open { transform: rotate(90deg); }
  .last { margin-left: auto; color: var(--muted); font-weight: 400; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .body {
    height: 160px; overflow: auto; padding: 8px 20px 12px; border-top: 1px solid var(--border);
    font-family: "Cascadia Mono", Consolas, monospace; font-size: 0.8rem; background: var(--bg);
  }
</style>
