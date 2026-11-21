<script lang="ts">
  import uPlot from "uplot";
  import { historyAverage, type AverageColumn, type AverageDay } from "../lib/api";
  import { CHARGE_SIGN } from "../lib/energy";
  import { overlay, rangeWithZero, monotone, tooltip } from "../lib/curve";
  import { errText } from "../lib/state.svelte";
  import { periodName } from "../lib/energy";
  import { t, type Key } from "../lib/i18n.svelte";

  const QUANTITIES: [AverageColumn, string][] = [["soc", "--auto"], ["pv", "--solar"], ["load", "--house"], ["bat", "--charge"], ["grid", "--grid"]];
  const quantityName = (c: AverageColumn) => (c === "soc" ? "SOC" : t(`flow.${c === "load" ? "house" : c}` as Key));
  // De la période la plus longue (pâle) à aujourd'hui (vif) : intensité de la teinte en %.
  const PERIODS: [number, number][] = [[365, 40], [90, 55], [30, 70], [7, 85], [0, 100]];
  const KINDS = { all: "common.allDays", week: "history.week", end: "soc.weekend" } as const;
  const SHARE = { all: 1, week: 5 / 7, end: 2 / 7 };
  const XS = Array.from({ length: 96 }, (_, i) => i / 4);
  const HEIGHT = 300;

  let column = $state<AverageColumn>("soc");
  let kind = $state<keyof typeof KINDS>("all");
  let shown = $state<Record<number, boolean>>({ 0: true, 7: true, 30: true, 90: true, 365: true });
  let data = $state<Record<number, AverageDay>>({});
  let error = $state("");
  let tick = $state(0);
  let theme = $state(0);
  let host: HTMLDivElement;
  let plot: uPlot | undefined;
  let seq = 0;

  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const tone = $derived(QUANTITIES.find(([c]) => c === column)![1]);
  const unit = $derived(column === "soc" ? "%" : "W");
  const color = (pct: number) => `color-mix(in oklab, ${css(tone)} ${pct}%, transparent)`;
  const insufficient = (days: number, d: AverageDay | undefined) => days > 0 && !!d && d.days < (days * SHARE[kind]) / 2;

  $effect(() => {
    tick;
    const id = ++seq;
    const weekend = kind === "all" ? null : kind === "end";
    error = "";
    Promise.all(PERIODS.map(([days]) => historyAverage(column, days, days ? weekend : null)))
      .then((r) => id === seq && (data = Object.fromEntries(PERIODS.map(([days], i) => [days, r[i]]))))
      .catch((e) => id === seq && ((error = errText(e)), (data = {})));
  });

  $effect(() => {
    const t = setInterval(() => tick++, 300_000);
    return () => clearInterval(t);
  });

  const hhmm = (h: number) => `${String(Math.floor(h)).padStart(2, "0")}:${String(Math.round((h % 1) * 60)).padStart(2, "0")}`;
  const sign = () => (column === "bat" ? CHARGE_SIGN : 1);

  function build() {
    plot?.destroy();
    plot = undefined;
    if (!Object.keys(data).length) return;
    const on = (days: number) => shown[days] && !insufficient(days, data[days]);
    const axis = { stroke: css("--muted"), grid: { stroke: css("--border"), width: 1 }, ticks: { stroke: css("--border"), width: 1 } };
    const cols = [XS, ...PERIODS.map(([days]) => data[days].avg.map((v) => (v == null ? null : v * sign())))] as uPlot.AlignedData;
    const series: uPlot.Series[] = [{}, ...PERIODS.map(([days, pct]) => {
      const c = color(pct);
      const covered = days ? ` (${t("period.days", { n: data[days].days })})` : "";
      return { label: `${days ? t("period.avg", { name: periodName(days) }) : periodName(days)}${covered}`, show: on(days), stroke: c, width: days ? 2 : 3, paths: monotone(() => 0.3), points: { show: false } };
    })];
    plot = new uPlot(
      {
        width: host.clientWidth,
        height: HEIGHT,
        padding: [8, 8, 0, 0],
        legend: { show: false },
        cursor: { drag: { x: false, y: false }, points: { show: false } },
        scales: { x: { time: false, range: [0, 24] }, y: { range: column === "soc" ? [0, 100] : rangeWithZero } },
        axes: [
          { ...axis, space: 40, splits: () => [0, 3, 6, 9, 12, 15, 18, 21, 24], values: (_u, s) => s.map((h) => `${String(h).padStart(2, "0")}:00`) },
          { ...axis, size: 62, values: (_u, v) => v.map((x) => `${x} ${unit}`) },
        ],
        series,
        plugins: [
          ...(column === "soc" ? [] : [overlay(() => [])]),
          tooltip({
            gap: () => 0.3,
            fmt: (_si, v) => `${Math.round(v)} ${unit}`,
            head: hhmm,
            kinds: { measured: t("avg.quarter"), interpolated: t("curve.interpolated") },
          }),
        ],
      },
      cols,
      host,
    );
  }

  $effect(() => {
    theme;
    data;
    shown[0], shown[7], shown[30], shown[90], shown[365];
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

<section class="card" aria-label={t("avg.section")}>
  <h3>{t("avg.section")}</h3>
  <div class="row">
    <div class="seg" role="group" aria-label={t("device.quantity")}>
      {#each QUANTITIES as [c]}<button aria-pressed={column === c} onclick={() => (column = c)}>{quantityName(c)}</button>{/each}
    </div>
    <div class="seg" role="group" aria-label={t("common.days")}>
      {#each Object.entries(KINDS) as [k, name]}<button aria-pressed={kind === k} onclick={() => (kind = k as keyof typeof KINDS)}>{t(name)}</button>{/each}
    </div>
  </div>
  <div class="row">
    {#each PERIODS as [days, pct]}
      {@const bad = insufficient(days, data[days])}
      <label class="check" class:off={bad}>
        <input type="checkbox" bind:checked={shown[days]} disabled={bad} />
        <span class="swatch" style:background={color(pct)}></span>{days ? t("period.avg", { name: periodName(days) }) : periodName(days)}
        {#if bad}<span class="muted">{t("avg.insufficient", { n: data[days].days })}</span>{/if}
      </label>
    {/each}
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  <div bind:this={host} role="img" aria-label={t("avg.aria")}></div>
  <p class="muted note">{t("avg.note")}{column === "bat" ? ` ${t("avg.noteBat")}` : ""}{column === "grid" ? ` ${t("avg.noteGrid")}` : ""}</p>
</section>

<style>
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 12px; margin-bottom: 12px; }
  .check { display: inline-flex; align-items: center; gap: 6px; }
  .check.off { color: var(--muted); }
  .swatch { width: 18px; height: 4px; border-radius: 2px; }
  .err { color: var(--danger); }
  .note { margin-top: 8px; font-size: var(--text-sm); }
</style>
