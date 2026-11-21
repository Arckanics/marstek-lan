<script lang="ts">
  import uPlot from "uplot";
  import { app, history, HISTORY_POINTS } from "../lib/state.svelte";

  let host: HTMLDivElement;
  let plot: uPlot | undefined;

  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const data = () => history as unknown as uPlot.AlignedData;
  const points = $derived((app.historyRev, history[0].length));

  function build() {
    plot?.destroy();
    const axis = {
      stroke: css("--muted"),
      grid: { stroke: css("--border"), width: 1 },
      ticks: { stroke: css("--border"), width: 1 },
    };
    const line = (label: string, color: string): uPlot.Series => ({
      label,
      stroke: css(color),
      width: 2,
      spanGaps: false,
      points: { show: false },
    });
    plot = new uPlot(
      {
        width: host.clientWidth,
        height: 190,
        padding: [8, 8, 0, 0],
        cursor: { drag: { x: false, y: false } },
        axes: [{ ...axis }, { ...axis, size: 64, values: (_u, v) => v.map((x) => `${x} W`) }],
        series: [{}, line("Solaire", "--solar"), line("Batterie (charge +)", "--charge"), line("Réseau (soutirage +)", "--grid")],
      },
      data(),
      host,
    );
  }

  $effect(() => {
    build();
    const ro = new ResizeObserver(() => plot?.setSize({ width: host.clientWidth, height: 190 }));
    ro.observe(host);
    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", build);
    return () => {
      ro.disconnect();
      mq.removeEventListener("change", build);
      plot?.destroy();
    };
  });

  $effect(() => {
    app.historyRev;
    plot?.setData(data());
  });
</script>

<div class="chart">
  <div bind:this={host} aria-hidden="true"></div>
  {#if points < 2}<p class="muted wait">Les premières mesures arrivent toutes les 10 secondes. Historique conservé : 1 heure ({HISTORY_POINTS} points).</p>{/if}
</div>

<style>
  .chart { position: relative; min-height: 190px; }
  .wait { position: absolute; inset: 0 0 40px; display: grid; place-content: center; text-align: center; padding: 0 24px; }
  .chart :global(.u-legend) { font-size: 0.8rem; color: var(--text); }
</style>
