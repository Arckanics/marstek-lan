<script lang="ts">
  import uPlot from "uplot";
  import type { EnergyRow, PowerSeries } from "../lib/api";

  let { power, energy, metric = "pv", label = "" }: { power?: PowerSeries; energy?: EnergyRow[]; metric?: "pv" | "gridIn" | "gridOut" | "load"; label?: string } = $props();

  let host: HTMLDivElement;
  let plot: uPlot | undefined;
  let theme = $state(0);

  const HEIGHT = 260;
  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const metricColor = { pv: "--solar", gridIn: "--grid", gridOut: "--charge", load: "--house" };
  const metricName = { pv: "Solaire", gridIn: "Soutiré", gridOut: "Injecté", load: "Consommé" };

  function build() {
    plot?.destroy();
    plot = undefined;
    const axis = { stroke: css("--muted"), grid: { stroke: css("--border"), width: 1 }, ticks: { stroke: css("--border"), width: 1 } };
    const base = { width: host.clientWidth, height: HEIGHT, padding: [8, 8, 0, 0] as uPlot.Padding, cursor: { drag: { x: false, y: false } } };
    const line = (label: string, color: string, extra: Partial<uPlot.Series> = {}): uPlot.Series => ({ label, stroke: css(color), width: 2, spanGaps: false, points: { show: false }, ...extra });

    if (power) {
      const d: uPlot.AlignedData = [power.ts.map((t) => t / 1000), power.pv, power.bat, power.grid, power.load, power.soc];
      plot = new uPlot(
        {
          ...base,
          scales: { soc: { range: [0, 100] } },
          axes: [{ ...axis }, { ...axis, size: 64, values: (_u, v) => v.map((x) => `${x} W`) }, { ...axis, scale: "soc", side: 1, size: 52, grid: { show: false }, values: (_u, v) => v.map((x) => `${x} %`) }],
          series: [{}, line("Solaire", "--solar"), line("Batterie", "--charge"), line("Réseau", "--grid"), line("Maison", "--house"), line("SOC", "--text", { scale: "soc", dash: [6, 4], width: 1.5 })],
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
          series: [{}, { label: `${metricName[metric]} (kWh)`, fill: color, stroke: color, paths: uPlot.paths.bars!({ size: [0.7, 60] }), points: { show: false } }],
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
