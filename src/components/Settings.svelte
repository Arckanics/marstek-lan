<script lang="ts">
  import { onMount } from "svelte";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { app, errText, setting } from "../lib/state.svelte";
  import { clone } from "../lib/store";

  let notify = $state(clone(app.settings.notify));
  const save = () => setting("notify", clone(notify));

  let autostart = $state(false);
  let error = $state("");

  onMount(() => {
    isEnabled().then((v) => (autostart = v), (e) => (error = errText(e)));
  });

  async function toggle() {
    error = "";
    try {
      await (autostart ? enable() : disable());
    } catch (e) {
      autostart = !autostart;
      error = errText(e);
    }
  }
</script>

<div class="stack">
  <section class="card" aria-label="Notifications">
    <h3>Notifications</h3>
    <div class="list">
      <div class="item">
        <label class="check"><input type="checkbox" bind:checked={notify.lowSoc.enabled} onchange={save} />SOC bas</label>
        <label class="field">Seuil (%)<input type="number" min="0" max="100" bind:value={notify.lowSoc.soc} disabled={!notify.lowSoc.enabled} onchange={save} /></label>
      </div>
      <label class="check"><input type="checkbox" bind:checked={notify.offline} onchange={save} />Batterie injoignable depuis plus de 2 minutes</label>
      <label class="check"><input type="checkbox" bind:checked={notify.modeChanged} onchange={save} />Mode changé par le plan ou une règle</label>
      <label class="check"><input type="checkbox" bind:checked={notify.writeFailed} onchange={save} />Écriture échouée</label>
    </div>
  </section>

  <section class="card" aria-label="Démarrage">
    <h3>Démarrage</h3>
    <label class="check"><input type="checkbox" bind:checked={autostart} onchange={toggle} />Lancer Marstek Control au démarrage de Windows</label>
    <p class="muted hint">L'application démarre alors réduite dans la zone de notification.</p>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </section>
</div>

<style>
  .list { display: grid; gap: 12px; }
  .item { display: flex; align-items: end; gap: 16px; }
  .check { display: flex; align-items: center; gap: 8px; min-height: 34px; }
  .hint { margin-top: 6px; }
  .err { margin-top: 8px; color: var(--danger); }
</style>
