<script lang="ts">
  import uPlot from "uplot";
  import { app, clock, history, view } from "../lib/state.svelte";
  import { locale, t } from "../lib/i18n.svelte";
  import { monotone, overlay, rangeWithZero, showPoints, timeValues, tooltip } from "../lib/curve";

  let host: HTMLDivElement;
  let plot: uPlot | undefined;

  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const data = () => history as unknown as uPlot.AlignedData;
  const points = $derived((app.historyRev, history[0].length));
  const poll = $derived(app.snapshot?.pollSecs || 10);
  const gap = () => 2.5 * (app.snapshot?.pollSecs || 10);
  const nowSec = () => clock.now / 1000;
  const WINDOW_S = 3600;

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
      paths: monotone(gap, () => view.t),
      points: { show: showPoints, size: 5, fill: css(color), stroke: css(color) },
    });
    plot = new uPlot(
      {
        width: host.clientWidth,
        height: 190,
        padding: [8, 8, 0, 0],
        cursor: { drag: { x: false, y: false }, points: { show: false } },
        legend: { live: false },
        scales: { x: { range: (u) => [Math.min(Math.max(nowSec() - WINDOW_S, (u.data[0][0] ?? nowSec()) - gap()), nowSec() - 2 * gap()), nowSec()] }, y: { range: rangeWithZero } },
        axes: [{ ...axis, values: timeValues }, { ...axis, size: 64, values: (_u, v) => v.map((x) => `${x} W`) }],
        series: [{}, line(t("flow.pv"), "--solar"), line(t("chart.bat"), "--charge"), line(t("chart.grid"), "--grid")],
        plugins: [
          tooltip({ gap, fmt: (_si, v) => `${Math.round(v).toLocaleString(locale())} W` }),
          overlay(() => []),
        ],
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
    clock.now;
    plot?.setData(data());
  });
</script>

<div class="chart">
  <div bind:this={host} role="img" aria-label={t("chart.aria")}></div>
  {#if points < 2}<p class="muted wait">{t("chart.wait", { poll })}</p>{/if}
  <p class="muted note">{t("chart.note", { poll, gap: 2.5 * poll })}</p>
</div>

<style>
  .chart { position: relative; min-height: 190px; }
  .wait { position: absolute; inset: 0 0 40px; display: grid; place-content: center; text-align: center; padding: 0 24px; }
  .note { margin-top: 8px; font-size: var(--text-sm); }
  .chart :global(.u-legend) { font-size: 0.8rem; color: var(--text); }
</style>
