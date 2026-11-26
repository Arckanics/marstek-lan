<script lang="ts">
  import { exportCsv, historyEnergy, historyPower, type EnergyRow, type PowerSeries } from "../lib/api";
  import { withHouse } from "../lib/energy";
  import { errText } from "../lib/state.svelte";
  import { locale, t, type Key } from "../lib/i18n.svelte";
  import HistoryChart from "./HistoryChart.svelte";
  import AverageDay from "./AverageDay.svelte";
  import SocHistory from "./SocHistory.svelte";
  import Savings from "./Savings.svelte";

  type Span = "day" | "week" | "month";
  const spans: Span[] = ["day", "week", "month"];
  const metrics = ["pv", "gridIn", "gridOut", "load"] as const;

  const iso = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

  let span = $state<Span>("day");
  let date = $state(iso(new Date()));
  let metric = $state<(typeof metrics)[number]>("pv");
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
        ? range[0].toLocaleDateString(locale(), { month: "long", year: "numeric" })
        : span === "week"
          ? t("history.weekRange", { from: range[0].toLocaleDateString(locale()), to: new Date(range[1].getTime() - 1).toLocaleDateString(locale()) })
          : range[0].toLocaleDateString(locale())),
  );
  const totals = $derived(metrics.map((k) => [k, energy?.reduce((a, r) => a + (r[k] ?? 0), 0) ?? 0] as const));
  const kwh = (wh: number) => `${(wh / 1000).toLocaleString(locale(), { maximumFractionDigits: 2 })} kWh`;

  let tick = $state(0);
  $effect(() => {
    const t = setInterval(() => tick++, 60_000);
    return () => clearInterval(t);
  });

  $effect(() => {
    tick;
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
  <section class="card" aria-label={t("history.period")}>
    <div class="row">
      <div class="seg" role="group" aria-label={t("history.scale")}>
        {#each spans as v}<button aria-pressed={span === v} onclick={() => (span = v)}>{t(`history.${v}` as Key)}</button>{/each}
      </div>
      <label class="field">{t("common.date")}<input type="date" bind:value={date} max={iso(new Date())} required /></label>
      <button class="btn" onclick={doExport} disabled={!bounds}>{t("history.export")}</button>
    </div>
    {#if exported}<p class="ok" role="status">{t("history.exported")} <span class="num">{exported}</span></p>{/if}
  </section>

  <section class="card" aria-label={t("common.history")}>
    <h3>{span === "day" ? t("history.powersSoc") : t("history.energyDay")}{title ? `, ${title}` : ""}</h3>
    {#if span !== "day"}
      <div class="seg metric" role="group" aria-label={t("history.metric")}>
        {#each metrics as v}<button aria-pressed={metric === v} onclick={() => (metric = v)}>{t(`history.${v}` as Key)}</button>{/each}
      </div>
    {/if}
    {#if error}
      <p class="err" role="alert">{error}</p>
    {:else if empty}
      <p class="muted">{t("history.none")}</p>
    {/if}
    <div class:hide={error || empty}>
      <HistoryChart {power} {energy} {metric} label={t("history.curve")} />
    </div>
    {#if energy?.length && range && range[1].getTime() > Date.now()}
      <p class="muted">{t("history.provisional")}</p>
    {/if}
    {#if energy?.length}
      <dl class="totals">
        {#each totals as [k, wh]}<dt>{t(`history.total.${k}` as Key)}</dt><dd class="num">{kwh(wh)}</dd>{/each}
      </dl>
    {/if}
  </section>
  <Savings bounds={bounds || undefined} />
  <SocHistory />
  <AverageDay />
</div>

<style>
  .ok { margin-top: 12px; }
  .err { color: var(--danger); }
  .metric { margin-bottom: 12px; }
  .hide { display: none; }
  .totals { margin-top: 16px; max-width: 360px; }
</style>
