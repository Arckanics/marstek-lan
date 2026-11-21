<script lang="ts">
  import { app, scan, connectTo, setting } from "../lib/state.svelte";

  let range = $state(app.settings.scanRange);

  function run(e: SubmitEvent) {
    e.preventDefault();
    setting("scanRange", range.trim());
    scan(range.trim());
  }
</script>

<div class="stack">
  <form class="card row" onsubmit={run}>
    <label class="field grow">
      Plage d'adresses (facultatif)
      <input bind:value={range} placeholder="Automatique, ou 192.168.1.1-254, 192.168.1.0/24" spellcheck="false" />
    </label>
    <button class="btn primary" disabled={app.scanning}>
      {#if app.scanning}<span class="spinner" aria-hidden="true"></span>Balayage en cours{:else}Balayer le réseau{/if}
    </button>
  </form>

  {#if app.devices.length}
    <ul class="list" aria-label="Appareils trouvés">
      {#each app.devices as d (d.ip)}
        <li>
          <button class="card dev" onclick={() => connectTo(d.ip)} aria-current={d.ip === app.connectedIp}>
            <span class="name">{d.name ?? "Appareil Marstek"}</span>
            <span class="num muted">{d.ip}{d.port !== 30000 ? `:${d.port}` : ""}</span>
            <span class="muted">{d.version ? `Version ${d.version}` : "Version inconnue"}</span>
            {#if d.ip === app.connectedIp}<span class="badge"><span class="dot on"></span>Connecté</span>
            {:else}<span class="muted act">Se connecter</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {:else if app.scanning}
    <div class="empty" role="status"><span class="spinner" aria-hidden="true"></span><p>Recherche des batteries sur le réseau local…</p></div>
  {:else}
    <div class="empty">
      <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" /><path d="M6.3 6.3a8 8 0 0 0 0 11.4M17.7 6.3a8 8 0 0 1 0 11.4M3.5 3.5a12 12 0 0 0 0 17M20.5 3.5a12 12 0 0 1 0 17" /></svg>
      <strong>Aucun appareil trouvé</strong>
      <p>Vérifiez que la batterie est sur le même réseau et que l'API locale est activée, ou indiquez une plage d'adresses.</p>
    </div>
  {/if}
</div>

<style>
  .grow { flex: 1; min-width: 260px; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
  .dev {
    width: 100%; display: grid; gap: 4px; text-align: left; cursor: pointer;
    transition: border-color 0.12s, background-color 0.12s;
  }
  .dev:hover { border-color: var(--muted); background: var(--surface-2); }
  .dev[aria-current="true"] { border-color: var(--text); }
  .name { font-weight: 600; font-size: 1.05rem; }
  .act, .dev .badge { margin-top: 8px; justify-self: start; }
</style>
