<script lang="ts">
  import uPlot from "uplot";
  import type { Outcome, SyncState } from "../lib/api";
  import { monotone, showPoints, tooltip } from "../lib/curve";
  import { app, clock, linkLog } from "../lib/state.svelte";
  import { outcomeLabel, spread } from "../lib/link";
  import { locale, t, type Key } from "../lib/i18n.svelte";

  const BAND: Record<SyncState, { token: string }> = { ok: { token: "--ok" }, warn: { token: "--warn" }, bad: { token: "--danger" }, unknown: { token: "--muted" } };
  const WINDOWS = [[10, "10 min"], [60, "1 h"]] as const;

  let win = $state<10 | 60>(10);
  let latHost: HTMLDivElement;
  let powHost: HTMLDivElement;
  let lat: uPlot | undefined;
  let pow: uPlot | undefined;
  let theme = $state(0);

  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const gap = () => 2.5 * (app.snapshot?.pollSecs || 10);
  const end = () => clock.now / 1000;
  const inWindow = () => linkLog.filter((p) => p.ts / 1000 >= end() - win * 60);

  function cols() {
    const pts = inWindow();
    const reqs = spread(pts);
    const only = (o: Outcome, v: (r: (typeof reqs)[number]) => number) => reqs.map((r) => (r.outcome === o ? v(r) : null));
    const x = pts.map((p) => p.ts / 1000);
    return {
      lat: [reqs.map((r) => r.x / 1000), only("ok", (r) => r.rttMs!), only("refused", (r) => r.rttMs!), only("timeout", () => 0), only("portClosed", () => 0)] as uPlot.AlignedData,
      pow: [x, pts.map((p) => p.expected), pts.map((p) => p.measured)] as uPlot.AlignedData,
    };
  }

  /** Bande d'état de la synchro en fond : chaque sondage couvre jusqu'au suivant, au plus 2,5 intervalles. */
  const bands: uPlot.Plugin = {
    hooks: {
      drawClear: (u) => {
        const pts = inWindow();
        const { ctx, bbox } = u;
        ctx.save();
        ctx.globalAlpha = 0.16;
        pts.forEach((p, i) => {
          const x0 = u.valToPos(p.ts / 1000, "x", true);
          const x1 = u.valToPos(Math.min(pts[i + 1]?.ts ?? Infinity, p.ts + gap() * 1000, clock.now) / 1000, "x", true);
          ctx.fillStyle = css(BAND[p.sync].token);
          ctx.fillRect(x0, bbox.top, Math.max(x1 - x0, 1), bbox.height);
        });
        ctx.restore();
      },
    },
  };

  function build() {
    lat?.destroy();
    pow?.destroy();
    const axis = { stroke: css("--muted"), grid: { stroke: css("--border"), width: 1 }, ticks: { stroke: css("--border"), width: 1 } };
    const xAxis = { ...axis, values: (u: uPlot, s: number[]) => s.map((t) => new Date(t * 1000).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", second: u.scales.x.max! - u.scales.x.min! < 1200 ? "2-digit" : undefined })) };
    const base = { width: latHost.clientWidth, height: 150, padding: [8, 8, 0, 0] as uPlot.Padding, legend: { live: false }, cursor: { drag: { x: false, y: false }, points: { show: false }, sync: { key: "link" } } };
    const x = { range: () => [end() - win * 60, end()] as [number, number] };
    const d = cols();
    const dots = (color: string, size: number): uPlot.Series.Points => ({ show: true, size, fill: color, stroke: color });
    const c = { auto: css("--auto"), warn: css("--warn"), danger: css("--danger"), exp: css("--text"), meas: css("--charge"), surface: css("--surface") };
    lat = new uPlot(
      {
        ...base,
        scales: { x, y: { range: (_u, _min, max) => [0, Math.max(200, (max ?? 0) * 1.1)] } },
        axes: [xAxis, { ...axis, size: 64, values: (_u, v) => v.map((n) => `${n} ms`) }],
        series: [
          {},
          { label: t("linkCharts.reqLatency"), stroke: c.auto, width: 2, paths: monotone(gap), points: { ...dots(c.auto, 5), show: showPoints } },
          { label: t("linkCharts.refused"), stroke: c.warn, width: 0, points: dots(c.warn, 9) },
          { label: t("linkCharts.noResponse"), stroke: c.danger, width: 0, points: dots(c.danger, 9) },
          { label: t("linkCharts.portClosed"), stroke: c.danger, width: 0, points: { show: true, size: 9, fill: c.surface, stroke: c.danger, width: 2 } },
        ],
        plugins: [bands, tooltip({ gap, fmt: (si, v) => (si === 3 ? t("linkCharts.noAnswer") : si === 4 ? t("outcome.portClosed") : `${Math.round(v)} ms`), pointsOnly: (si) => si > 1 })],
      },
      d.lat,
      latHost,
    );
    pow = new uPlot(
      {
        ...base,
        scales: { x, y: { range: (_u, min, max) => [Math.min(min ?? 0, 0) * 1.1, Math.max(max ?? 0, 100) * 1.1] } },
        axes: [xAxis, { ...axis, size: 64, values: (_u, v) => v.map((n) => `${n} W`) }],
        series: [
          {},
          { label: t("linkCharts.expected"), stroke: c.exp, width: 1.5, dash: [6, 4], paths: uPlot.paths.stepped!({ align: 1 }), points: { show: false } },
          { label: t("linkCharts.measured"), stroke: c.meas, width: 2, paths: monotone(gap), points: { ...dots(c.meas, 5), show: showPoints } },
        ],
        plugins: [bands, tooltip({ gap, fmt: (_si, v) => `${Math.round(v).toLocaleString(locale())} W`, pointsOnly: (si) => si === 1 })],
      },
      d.pow,
      powHost,
    );
  }

  $effect(() => {
    theme;
    win;
    build();
  });

  $effect(() => {
    app.linkRev;
    clock.now;
    const d = cols();
    lat?.setData(d.lat);
    pow?.setData(d.pow);
  });

  $effect(() => {
    const ro = new ResizeObserver(() => {
      lat?.setSize({ width: latHost.clientWidth, height: 150 });
      pow?.setSize({ width: powHost.clientWidth, height: 150 });
    });
    ro.observe(latHost);
    const mq = matchMedia("(prefers-color-scheme: dark)");
    const retheme = () => theme++;
    mq.addEventListener("change", retheme);
    return () => {
      ro.disconnect();
      mq.removeEventListener("change", retheme);
      lat?.destroy();
      pow?.destroy();
    };
  });

  const rtts = $derived((app.linkRev, clock.now, spread(inWindow()).flatMap((r) => (r.rttMs != null ? [r.rttMs] : [])).sort((a, b) => a - b)));
  const quant = (q: number) => (rtts.length ? rtts[Math.min(rtts.length - 1, Math.ceil(q * rtts.length) - 1)] : undefined);
  const lastRows = $derived((app.linkRev, spread(linkLog.slice(-10)).slice(-10).reverse()));
</script>

<div class="seg" role="group" aria-label={t("linkCharts.window")}>
  {#each WINDOWS as [v, name]}<button aria-pressed={win === v} onclick={() => (win = v)}>{name}</button>{/each}
</div>

<h3>{t("linkCharts.latency")}</h3>
<div bind:this={latHost} role="img" aria-label={t("linkCharts.latencyAria")}></div>
<h3>{t("linkCharts.powerTitle")}</h3>
<div bind:this={powHost} role="img" aria-label={t("linkCharts.powerAria")}></div>

<ul class="key" aria-label={t("linkCharts.bandAria")}>
  {#each Object.entries(BAND) as [state, b]}<li><span class="sw" style:background="var({b.token})"></span>{t(`sync.${state}` as Key)}</li>{/each}
</ul>
<dl class="q">
  <dt>{t("linkCharts.median")}</dt><dd class="num">{quant(0.5) ?? "—"} ms</dd>
  <dt>{t("linkCharts.p90")}</dt><dd class="num">{quant(0.9) ?? "—"} ms</dd>
  <dt>{t("linkCharts.polls")}</dt><dd class="num">{inWindow().length}</dd>
</dl>

<table>
  <caption class="muted">{t("linkCharts.last10")}</caption>
  <thead><tr><th>{t("linkCharts.time")}</th><th>{t("linkCharts.method")}</th><th>{t("linkCharts.latency")}</th><th>{t("linkCharts.result")}</th><th>{t("linkCharts.sync")}</th></tr></thead>
  <tbody>
    {#each lastRows as r (r.x)}
      <tr class:warn={r.outcome === "refused"} class:bad={r.outcome === "timeout" || r.outcome === "portClosed"}>
        <td class="num">{new Date(r.x).toLocaleTimeString(locale())}</td>
        <td>{r.method}</td>
        <td class="num">{r.rttMs != null ? `${r.rttMs} ms` : "—"}</td>
        <td>{outcomeLabel(r.outcome)}</td>
        <td>{t(`band.${r.poll.sync}` as Key)}</td>
      </tr>
    {/each}
  </tbody>
</table>

<style>
  h3 { margin: 16px 0 4px; font-size: var(--text-sm); }
  .key { display: flex; flex-wrap: wrap; gap: 4px 16px; margin: 8px 0; padding: 0; list-style: none; font-size: var(--text-xs); color: var(--muted); }
  .sw { display: inline-block; width: 10px; height: 10px; margin-right: 6px; border-radius: 2px; opacity: 0.6; }
  .q { margin-top: 8px; }
  table { width: 100%; margin-top: 12px; border-collapse: collapse; font-size: var(--text-sm); }
  caption { text-align: left; padding-bottom: 4px; }
  th, td { text-align: left; padding: 2px 8px 2px 0; }
  th { color: var(--muted); font-weight: 500; }
  tr.warn td:nth-child(4) { color: var(--warn); font-weight: 600; }
  tr.bad td:nth-child(4) { color: var(--danger); font-weight: 600; }
</style>
