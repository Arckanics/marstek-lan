<script lang="ts">
  import uPlot from "uplot";
  import type { EnergyRow, PowerSeries } from "../lib/api";
  import { monotone, overlay, rangeWithZero, showPoints, timeValues, tooltip } from "../lib/curve";
  import { app } from "../lib/state.svelte";
  import { locale, t, type Key } from "../lib/i18n.svelte";

  let { power, energy, metric = "pv", label = "" }: { power?: PowerSeries; energy?: EnergyRow[]; metric?: "pv" | "gridIn" | "gridOut" | "load"; label?: string } = $props();

  let host: HTMLDivElement;
  let plot: uPlot | undefined;
  let theme = $state(0);

  const HEIGHT = 260;
  /** Une minute enregistrée sans voisine à moins de 2,5 sondages est isolée ; mini 150 s car un point ne vaut qu'une minute. */
  const gap = () => Math.max(150, 2.5 * (app.snapshot?.pollSecs || 10));
  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const metricColor = { pv: "--solar", gridIn: "--grid", gridOut: "--charge", load: "--house" };

  function build() {
    plot?.destroy();
    plot = undefined;
    const axis = { stroke: css("--muted"), grid: { stroke: css("--border"), width: 1 }, ticks: { stroke: css("--border"), width: 1 } };
    const base = { width: host.clientWidth, height: HEIGHT, padding: [8, 8, 0, 0] as uPlot.Padding, cursor: { drag: { x: false, y: false } } };
    const line = (label: string, color: string, extra: Partial<uPlot.Series> = {}): uPlot.Series => ({ label, stroke: css(color), width: 2, spanGaps: false, paths: monotone(gap), points: { show: showPoints, size: 5, fill: css(color), stroke: css(color) }, ...extra });

    if (power) {
      const cols = [power.ts.map((t) => t / 1000), power.pv, power.bat, power.grid, power.load, power.soc];
      const cl = power.cloud;
      // La ligne pleine saute les points cloud (coupure locale) : l'overlay les trace en pointillé par-dessus le trou.
      const solid = cols.map((c, ci) => (ci === 0 ? c : (c as (number | null)[]).map((v, i) => (cl[i] ? null : v))));
      // La minute en cours n'est qu'une moyenne partielle : elle est tracée en pointillé, puis corrigée au rafraîchissement.
      const partial = cols[0].length > 1 && cols[0].at(-1)! * 1000 > Date.now() - 60_000;
      const d = (partial ? solid.map((c) => c.slice(0, -1)) : solid) as uPlot.AlignedData;
      plot = new uPlot(
        {
          ...base,
          legend: { live: false },
          cursor: { ...base.cursor, points: { show: false } },
          scales: { y: { range: rangeWithZero }, soc: { range: [0, 100] } },
          plugins: [
            tooltip({ gap, fmt: (si, v) => (si === 5 ? `${Math.round(v)} %` : `${Math.round(v).toLocaleString(locale())} W`) }),
            overlay(() => {
              const xs = cols[0] as number[];
              // Segment pointillé de i-1 à i pour une série, s'il a deux valeurs : minute partielle et points cloud.
              const seg = (si: number, i: number) => {
                const y0 = (cols[si] as (number | null)[])[i - 1], y1 = (cols[si] as (number | null)[])[i];
                return i > 0 && y0 != null && y1 != null ? [{ si, x0: xs[i - 1], y0, x1: xs[i], y1 }] : [];
              };
              const partialSegs = partial ? [1, 2, 3, 4, 5].flatMap((si) => seg(si, xs.length - 1)) : [];
              // Pointillé continu à travers la zone cloud, raccordé à la ligne pleine à chaque bord.
              const cloudSegs = cl.flatMap((c, i) => (c || cl[i - 1] ? [1, 2, 3, 4, 5].flatMap((si) => seg(si, i)) : []));
              return [...partialSegs, ...cloudSegs];
            }),
          ],
          axes: [{ ...axis, values: timeValues }, { ...axis, size: 64, values: (_u, v) => v.map((x) => `${x} W`) }, { ...axis, scale: "soc", side: 1, size: 52, grid: { show: false }, values: (_u, v) => v.map((x) => `${x} %`) }],
          series: [{}, line(t("flow.pv"), "--solar"), line(t("flow.bat"), "--charge"), line(t("flow.grid"), "--grid"), line(t("flow.house"), "--house"), line("SOC", "--text", { scale: "soc", dash: [6, 4], width: 1.5 })],
        },
        d,
        host,
      );
    } else if (energy) {
      const d: uPlot.AlignedData = [energy.map((r) => r.ts / 1000), energy.map((r) => (r[metric] == null ? null : r[metric] / 1000))];
      const color = css(metricColor[metric]);
      plot = new uPlot(
        {
          ...base,
          scales: { x: { range: (_u, min, max) => [min - 43200, max + 43200] } },
          axes: [{ ...axis }, { ...axis, size: 64, values: (_u, v) => v.map((x) => `${x} kWh`) }],
          series: [{}, { label: `${t(`history.${metric}` as Key)} (kWh)`, value: (_u, v) => (v == null ? "—" : `${v.toLocaleString(locale(), { maximumFractionDigits: 2 })} kWh`), fill: color, stroke: color, paths: uPlot.paths.bars!({ size: [0.7, 60] }), points: { show: false } }],
        },
        d,
        host,
      );
    }
  }

  $effect(() => {
    theme;
    power;
    energy;
    metric;
    build();
  });

  $effect(() => {
    const ro = new ResizeObserver(() => plot?.setSize({ width: host.clientWidth, height: HEIGHT }));
    ro.observe(host);
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const retheme = () => theme++;
    mq.addEventListener("change", retheme);
    return () => {
      ro.disconnect();
      mq.removeEventListener("change", retheme);
      plot?.destroy();
    };
  });
</script>

<div bind:this={host} role="img" aria-label={label}></div>

<style>
  div :global(.u-legend) { font-size: 0.8rem; color: var(--text); }
</style>
