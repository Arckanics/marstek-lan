<script lang="ts">
  import { tariffPlan, type Block } from "../lib/api";
  import { app, errText, setting } from "../lib/state.svelte";
  import { clone } from "../lib/store";
  import Days from "./Days.svelte";

  let { onfill }: { onfill: (blocks: Block[]) => void } = $props();

  let t = $state(clone(app.settings.tariff));
  let error = $state("");

  $effect(() => {
    const j = JSON.stringify(t);
    if (j === JSON.stringify(app.settings.tariff)) return;
    error = "";
    setting("tariff", JSON.parse(j));
  });

  async function generate() {
    error = "";
    try {
      onfill(await tariffPlan(clone(t)));
    } catch (e) {
      error = errText(e);
    }
  }
</script>

<section class="card" aria-label="Générateur tarif">
  <h3>Générateur tarif</h3>
  <p class="muted intro">Remplit la grille du modèle sélectionné : charge pendant les heures creuses, décharge le reste du temps. Le contenu actuel de la grille est remplacé.</p>

  <ul class="ranges">
    {#each t.offpeak as r, i}
      <li>
        <label class="field">Début<input type="time" bind:value={r.start} /></label>
        <label class="field">Fin<input type="time" bind:value={r.end} /></label>
        <button class="btn danger" onclick={() => t.offpeak.splice(i, 1)} aria-label="Supprimer la plage {i + 1}">Supprimer</button>
      </li>
    {:else}
      <li class="muted">Aucune plage : ajoutez au moins une plage d'heures creuses.</li>
    {/each}
  </ul>
  <button class="btn" onclick={() => t.offpeak.push({ start: "22:00", end: "06:00" })}>Ajouter une plage</button>

  <div class="row opts">
    <label class="field">Puissance de charge (W)<input type="number" min="0" step="50" bind:value={t.chargePower} /></label>
    <label class="field">Puissance de décharge (W)<input type="number" min="0" step="50" bind:value={t.dischargePower} /></label>
    <div class="field">Jours<Days bind:days={t.days} /></div>
  </div>

  <div class="row actions">
    <button class="btn" onclick={generate} disabled={!t.offpeak.length}>Remplir la grille</button>
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</section>

<style>
  .intro { margin-bottom: 14px; }
  .ranges { list-style: none; margin: 0 0 10px; padding: 0; display: grid; gap: 8px; }
  .ranges li { display: flex; align-items: end; gap: 12px; }
  .opts, .actions { margin-top: 16px; }
  .err { color: var(--danger); margin-top: 10px; }
</style>
