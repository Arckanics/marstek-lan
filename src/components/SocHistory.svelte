<script lang="ts">
  import uPlot from "uplot";
  import { historySoc, type SocStat } from "../lib/api";
  import { monotone, overlay, showPoints, tooltip } from "../lib/curve";
  import { app, errText } from "../lib/state.svelte";
  import { periodName } from "../lib/energy";
  import { locale, t, type Key } from "../lib/i18n.svelte";

  const PERIODS = [7, 30, 90, 365] as const;
  const DAY_MS = 86_400_000;
  const HEIGHT = 260;

  let days = $state<(typeof PERIODS)[number]>(30);
  let view = $state<"trend" | "profile">("trend");
  let split = $state(false);
  let stats = $state<SocStat[]>([]);
  let error = $state("");
  let loading = $state(false);
  let tick = $state(0);
  let theme = $state(0);
  let host: HTMLDivElement;
  let plot: uPlot | undefined;
  let seq = 0;

  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

  /** Repères : seuils des règles SOC, de la notification et cible de charge du tarif, quand ils sont actifs. */
  const marks = $derived.by(() => {
    const { rules, notify, tariff } = app.settings;
    const m: [number, string][] = [];
    if (rules.low.enabled) m.push([rules.low.soc, t("soc.ruleLow")]);
    if (rules.high.enabled) m.push([rules.high.soc, t("soc.ruleHigh")]);
    if (notify.lowSoc.enabled) m.push([notify.lowSoc.soc, t("soc.alertLow")]);
    if (tariff.chargeSoc) m.push([tariff.chargeSoc, t("soc.chargeTarget")]);
    return m;
  });

  $effect(() => {
    tick;
    const id = ++seq;
    const to = Date.now();
    loading = true;
    error = "";
    historySoc(to - days * DAY_MS, to, view === "profile")
      .then((r) => id === seq && (stats = r))
      .catch((e) => id === seq && ((error = errText(e)), (stats = [])))
      .finally(() => id === seq && (loading = false));
  });

  $effect(() => {
    const t = setInterval(() => tick++, 300_000);
    return () => clearInterval(t);
  });

  type Group = { name: string; color: string; rows: Map<number, SocStat> };

  function groups(): Group[] {
    const by = (w: boolean | undefined) => new Map(stats.filter((r) => w == null || r.weekend === w).map((r) => [view === "trend" ? r.ts / 1000 : r.hour, r] as const));
    if (view === "trend") return [{ name: "SOC", color: "--auto", rows: by(undefined) }];
    if (split) return [{ name: t("history.week"), color: "--auto", rows: by(false) }, { name: t("soc.weekend"), color: "--solar", rows: by(true) }];
    // Jours ouvrés et week-ends fusionnés : moyenne pondérée par le nombre d'échantillons.
    const all = new Map<number, SocStat>();
    for (const r of stats) {
      const a = all.get(r.hour);
      all.set(r.hour, a ? { ...a, min: Math.min(a.min, r.min), max: Math.max(a.max, r.max), avg: (a.avg * a.n + r.avg * r.n) / (a.n + r.n), n: a.n + r.n } : r);
    }
    return [{ name: t("common.allDays"), color: "--auto", rows: all }];
  }

  function build() {
    plot?.destroy();
    plot = undefined;
    if (!stats.length) return;
    const trend = view === "trend";
    const gs = groups();
    const xs = trend ? [...gs[0].rows.keys()].sort((a, b) => a - b) : Array.from({ length: 24 }, (_, h) => h);
    const gap = () => (trend ? 1.5 * 86400 : 1.5);
    const cols = [xs, ...gs.flatMap((g) => (["min", "max", "avg"] as const).map((k) => xs.map((x) => g.rows.get(x)?.[k] ?? null)))] as uPlot.AlignedData;
    const axis = { stroke: css("--muted"), grid: { stroke: css("--border"), width: 1 }, ticks: { stroke: css("--border"), width: 1 } };
    const today = new Date().setHours(0, 0, 0, 0) / 1000;
    const series: uPlot.Series[] = [{}];
    const bands: uPlot.Band[] = [];
    gs.forEach((g, i) => {
      const c = css(g.color);
      const edge = { stroke: c, width: 0, points: { show: false }, paths: uPlot.paths.linear!() };
      // Bandes min-max : valeurs agrégées par période, jamais lissées ; seule la moyenne est interpolée.
      series.push({ ...edge, label: t("soc.min", { name: g.name }) }, { ...edge, label: t("soc.max", { name: g.name }) }, { label: t("soc.avg", { name: g.name }), stroke: c, width: 2, paths: monotone(gap), points: { show: showPoints, size: 5, fill: c, stroke: c } });
      bands.push({ series: [i * 3 + 2, i * 3 + 1], fill: `color-mix(in oklab, ${c} 18%, transparent)` });
    });
    const text = css("--text");
    const lines: uPlot.Plugin = {
      hooks: {
        draw: (u) => {
          const { ctx, bbox } = u;
          const dpr = uPlot.pxRatio;
          ctx.save();
          ctx.setLineDash([4 * dpr, 4 * dpr]);
          ctx.fillStyle = text;
          ctx.strokeStyle = text;
          ctx.globalAlpha = 0.55;
          ctx.lineWidth = dpr;
          ctx.font = `${11 * dpr}px sans-serif`;
          ctx.textAlign = "right";
          for (const [v, label] of marks) {
            const y = u.valToPos(v, "y", true);
            ctx.beginPath();
            ctx.moveTo(bbox.left, y);
            ctx.lineTo(bbox.left + bbox.width, y);
            ctx.stroke();
            ctx.fillText(`${label} ${v} %`, bbox.left + bbox.width - 4 * dpr, y - 3 * dpr);
          }
          ctx.restore();
        },
      },
    };
    plot = new uPlot(
      {
        width: host.clientWidth,
        height: HEIGHT,
        padding: [8, 8, 0, 0],
        legend: { live: false },
        cursor: { drag: { x: false, y: false }, points: { show: false } },
        scales: trend ? { y: { range: [0, 100] } } : { x: { time: false, range: [0, 23] }, y: { range: [0, 100] } },
        axes: [trend ? { ...axis } : { ...axis, space: 40, values: (_u, s) => s.map((h) => `${h} h`) }, { ...axis, size: 52, values: (_u, v) => v.map((x) => `${x} %`) }],
        series,
        bands,
        plugins: [
          lines,
          tooltip({
            gap,
            fmt: (_si, v) => `${Math.round(v)} %`,
            pointsOnly: () => true,
            head: (t) => (trend ? new Date(t * 1000).toLocaleDateString(locale()) : `${Math.round(t)} h`),
            kinds: { measured: t(trend ? "soc.aggDay" : "soc.aggPeriod") },
          }),
          // Le jour en cours n'est pas complet : son segment est en pointillé et sera corrigé au rafraîchissement.
          overlay(() => {
            const n = xs.length - 1;
            if (!trend || n < 1 || xs[n] < today) return [];
            const a = cols[3][n - 1];
            const b = cols[3][n];
            return a != null && b != null ? [{ si: 3, x0: xs[n - 1], y0: a, x1: xs[n], y1: b }] : [];
          }, false),
        ],
      },
      cols,
      host,
    );
  }

  $effect(() => {
    theme;
    stats;
    marks;
    split;
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

<section class="card" aria-label={t("soc.section")}>
  <h3>{t("soc.title")}</h3>
  <div class="row">
    <div class="seg" role="group" aria-label={t("history.period")}>
      {#each PERIODS as v}<button aria-pressed={days === v} onclick={() => (days = v)}>{periodName(v)}</button>{/each}
    </div>
    <div class="seg" role="group" aria-label={t("soc.view")}>
      <button aria-pressed={view === "trend"} onclick={() => (view = "trend")}>{t("soc.trend")}</button>
      <button aria-pressed={view === "profile"} onclick={() => (view = "profile")}>{t("soc.profile")}</button>
    </div>
    {#if view === "profile"}<label class="check"><input type="checkbox" bind:checked={split} />{t("soc.split")}</label>{/if}
  </div>
  {#if error}
    <p class="err" role="alert">{error}</p>
  {:else if !loading && !stats.length}
    <p class="muted">{t("soc.none")}</p>
  {/if}
  <div bind:this={host} role="img" aria-label={t(view === "trend" ? "soc.ariaTrend" : "soc.ariaProfile")}></div>
  <p class="muted note">
    {t(view === "trend" ? "soc.noteTrend" : "soc.noteProfile")}
    {marks.length ? t("soc.marks") : t("soc.noMarks")} {t("soc.dodFloor")}
  </p>
</section>

<style>
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-bottom: 12px; }
  .check { display: inline-flex; align-items: center; gap: 6px; }
  .err { color: var(--danger); }
  .note { margin-top: 8px; font-size: var(--text-sm); }
  div :global(.u-legend) { font-size: 0.8rem; color: var(--text); }
</style>
