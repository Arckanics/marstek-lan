<script lang="ts">
  import { exportCsv, historyEnergy, historyPower, type EnergyRow, type PowerSeries } from "../lib/api";
  import { withHouse } from "../lib/energy";
  import { errText } from "../lib/state.svelte";
  import HistoryChart from "./HistoryChart.svelte";

  type Span = "day" | "week" | "month";
  const spans: [Span, string][] = [["day", "Jour"], ["week", "Semaine"], ["month", "Mois"]];
  const metrics = [["pv", "Solaire"], ["gridIn", "Soutiré"], ["gridOut", "Injecté"], ["load", "Consommé"]] as const;

  const iso = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

  let span = $state<Span>("day");
  let date = $state(iso(new Date()));
  let metric = $state<(typeof metrics)[number][0]>("pv");
  let power = $state<PowerSeries>();
  let energy = $state<EnergyRow[]>();
  let error = $state("");
  let loading = $state(false);
  let exported = $state("");
  let seq = 0;

  const range = $derived.by(() => {
    const [y, m, d] = date.split("-").map(Number);
    if (!y) return undefined;
    if (span === "day") return [new Date(y, m - 1, d), new Date(y, m - 1, d + 1)];
    if (span === "week") {
      const back = (new Date(y, m - 1, d).getDay() + 6) % 7;
      return [new Date(y, m - 1, d - back), new Date(y, m - 1, d - back + 7)];
    }
    return [new Date(y, m - 1, 1), new Date(y, m, 1)];
  });
  const bounds = $derived(range && ([range[0].getTime(), range[1].getTime()] as const));
  const title = $derived(
    range &&
      (span === "month"
        ? range[0].toLocaleDateString("fr-FR", { month: "long", year: "numeric" })
        : `${range[0].toLocaleDateString("fr-FR")}${span === "week" ? ` au ${new Date(range[1].getTime() - 1).toLocaleDateString("fr-FR")}` : ""}`),
  );
  const totals = $derived(metrics.map(([k, name]) => [name, energy?.reduce((a, r) => a + (r[k] ?? 0), 0) ?? 0] as const));
  const kwh = (wh: number) => `${(wh / 1000).toLocaleString("fr-FR", { maximumFractionDigits: 2 })} kWh`;

  $effect(() => {
    if (!bounds) return;
    const [from, to] = bounds;
    const id = ++seq;
    const day = span === "day";
    loading = true;
    error = "";
    exported = "";
    (day ? historyPower(from, to) : historyEnergy(from, to, "day"))
      .then((r) => {
        if (id !== seq) return;
        power = day ? withHouse(r as PowerSeries) : undefined;
        energy = day ? undefined : (r as EnergyRow[]);
      })
      .catch((e) => id === seq && ((error = errText(e)), (power = energy = undefined)))
      .finally(() => id === seq && (loading = false));
  });

  async function doExport() {
    if (!bounds) return;
    error = "";
    try {
      exported = await exportCsv(bounds[0], bounds[1]);
    } catch (e) {
      error = errText(e);
    }
  }

  const empty = $derived(!loading && !error && (power ? power.ts.length === 0 : energy?.length === 0));
</script>

<div class="stack">
  <section class="card" aria-label="Période">
    <div class="row">
      <div class="seg" role="group" aria-label="Échelle">
        {#each spans as [v, name]}<button aria-pressed={span === v} onclick={() => (span = v)}>{name}</button>{/each}
      </div>
      <label class="field">Date<input type="date" bind:value={date} max={iso(new Date())} required /></label>
      <button class="btn" onclick={doExport} disabled={!bounds}>Exporter en CSV</button>
    </div>
    {#if exported}<p class="ok" role="status">Fichier écrit : <span class="num">{exported}</span></p>{/if}
  </section>

  <section class="card" aria-label="Historique">
    <h3>{span === "day" ? "Puissances et SOC" : "Énergie par jour"}{title ? ` · ${title}` : ""}</h3>
    {#if span !== "day"}
      <div class="seg metric" role="group" aria-label="Grandeur affichée">
        {#each metrics as [v, name]}<button aria-pressed={metric === v} onclick={() => (metric = v)}>{name}</button>{/each}
      </div>
    {/if}
    {#if error}
      <p class="err" role="alert">{error}</p>
    {:else if empty}
      <p class="muted">Aucune mesure sur cette période.</p>
    {/if}
    <div class:hide={error || empty}>
      <HistoryChart {power} {energy} {metric} label="Courbe d'historique" />
    </div>
    {#if energy?.length}
      <dl class="totals">
        {#each totals as [name, wh]}<dt>Total {name.toLowerCase()}</dt><dd class="num">{kwh(wh)}</dd>{/each}
      </dl>
    {/if}
  </section>
</div>

<style>
  .ok { margin-top: 10px; }
  .err { color: var(--danger); }
  .metric { margin-bottom: 10px; }
  .hide { display: none; }
  .totals { margin-top: 14px; max-width: 360px; }
</style>
