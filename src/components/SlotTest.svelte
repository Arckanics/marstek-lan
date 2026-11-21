<script lang="ts">
  import { runSlotTest } from "../lib/api";
  import { app, errText, log } from "../lib/state.svelte";

  const EXPECTED = 60;
  const running = $derived(app.slotTest != null && app.slotTest.verdict === null);
  const off = $derived(!app.connectedIp || app.busy || running);
  const saved = $derived(app.settings.outsideSlots);
  const verdict = $derived(app.slotTest?.verdict ?? (running ? null : saved?.verdict));
  const last = $derived(app.slotTest?.samples.at(-1)?.batPower);

  const start = () =>
    runSlotTest().catch((e) => {
      app.slotTest = undefined;
      log(`Test hors créneaux : ${errText(e)}`);
    });
</script>

<section class="card" aria-label="Test hors créneaux">
  <h3>Test hors créneaux</h3>
  <p class="muted intro">Écrit un créneau de charge de 200 W pendant 5 minutes, relève la puissance de la batterie jusqu'à 5 minutes après sa fin, puis restaure le plan actif. Il vérifie que la batterie reste au repos hors des créneaux programmés (environ 10 minutes).</p>
  <button class="btn" disabled={off} onclick={start}>{running ? "Test en cours…" : "Lancer le test"}</button>

  {#if running && app.slotTest}
    <div class="prog" role="status">
      <progress max={EXPECTED} value={app.slotTest.samples.length} aria-label="Progression du test"></progress>
      <span class="num muted">{app.slotTest.samples.length} mesure(s){last != null ? `, dernière : ${Math.round(last)} W` : ""}</span>
    </div>
  {/if}

  {#if verdict}
    <p class="verdict" class:ok={verdict === "idle"} class:ko={verdict === "follows"} role="status">
      {verdict === "idle" ? "Conforme : la batterie est restée au repos après la fin du créneau. Les blocs Auto, IA et UPS du planning sont pilotés par l'application (PC allumé requis)." : "Non conforme : la batterie a continué à charger ou décharger après la fin du créneau. Les blocs Auto, IA et UPS du planning n'ont alors pas besoin de l'application."}
      {#if !app.slotTest?.verdict && saved?.ts}<span class="muted"> Résultat du {new Date(saved.ts).toLocaleString("fr-FR")}.</span>{/if}
    </p>
  {/if}
</section>

<style>
  .intro { margin-bottom: 12px; }
  .prog { display: flex; align-items: center; gap: 12px; margin-top: 12px; }
  progress { flex: 1; accent-color: var(--primary); }
  .verdict { margin-top: 12px; font-weight: 500; }
  .ok { color: var(--charge-ink); }
  .ko { color: var(--danger); }
</style>
