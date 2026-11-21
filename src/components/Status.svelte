<script lang="ts">
  import { app } from "../lib/state.svelte";
  import { flows } from "../lib/energy";
  import NotConnected from "./NotConnected.svelte";
  import FlowDiagram from "./FlowDiagram.svelte";
  import SocGauge from "./SocGauge.svelte";
  import Chart from "./Chart.svelte";

  const modeNames: Record<string, string> = { Auto: "Auto", AI: "IA", Manual: "Manuel", Passive: "Passif", UPS: "UPS" };

  const s = $derived(app.snapshot);
  const f = $derived(flows(s));
  const soc = $derived(s?.bat?.soc ?? s?.es?.batSoc);
  const num = (v: number | undefined, unit: string) => (v == null ? "—" : `${v.toLocaleString("fr-FR")} ${unit}`);
</script>

{#if !app.connectedIp}
  <NotConnected />
{:else if !s}
  <div class="empty" role="status"><span class="spinner" aria-hidden="true"></span><p>Connexion à {app.connectedIp}, première mesure en cours…</p></div>
{:else}
  <div class="stack">
    <div class="top">
      <section class="card" aria-label="Flux d'énergie">
        <h3>Flux d'énergie</h3>
        <FlowDiagram {f} />
      </section>
      <section class="card side" aria-label="Batterie">
        <h3>Batterie</h3>
        <SocGauge {soc} bat={f.bat} />
        <dl>
          <dt>Mode</dt><dd>{s.mode?.mode ? (modeNames[s.mode.mode] ?? s.mode.mode) : "—"}</dd>
          <dt>Température</dt><dd class="num">{num(s.bat?.batTemp, "°C")}</dd>
          <dt>Énergie restante</dt><dd class="num">{num(s.bat?.batCapacity ?? s.es?.batCap, "Wh")}</dd>
          <dt>Capacité nominale</dt><dd class="num">{num(s.bat?.ratedCapacity, "Wh")}</dd>
        </dl>
      </section>
    </div>
    <section class="card" aria-label="Historique">
      <h3>Dernière heure</h3>
      <Chart />
    </section>
  </div>
{/if}

<style>
  .top { display: grid; grid-template-columns: minmax(0, 1fr) 280px; gap: 16px; }
  .side { display: grid; gap: 16px; align-content: start; }
</style>
