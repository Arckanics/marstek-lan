<script lang="ts">
  import type { Outcome, SyncState } from "../lib/api";
  import { app, clock, linkLog } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";
  import LinkCharts from "./LinkCharts.svelte";
  import { cloudAge, qualityLabel, downText, fmtAge, quality, spread } from "../lib/link";

  const TONES: Record<SyncState, string> = { ok: "k-ok", warn: "k-warn", bad: "k-bad", unknown: "k-none" };

  const MINI_MS = 10 * 60_000;
  let open = $state(false);
  const s = $derived(app.snapshot);
  const sync = $derived(s?.sync ?? { state: "unknown" as const, expected: "", measured: "", expectedW: null, measuredW: null, reason: null });
  const meta = $derived({ label: t(`sync.${sync.state}`), tone: TONES[sync.state] });
  const q = $derived(s ? quality(s, clock.now) : "offline");
  const loss = $derived(s?.link.total ? Math.round((1 - s.link.ok / s.link.total) * 100) : null);
  const tip = $derived(
    [meta.label, sync.expected && t("sync.tipExpected", { v: sync.expected }), sync.measured && t("sync.tipMeasured", { v: sync.measured }), sync.reason && t("sync.tipReason", { v: sync.reason })].filter(Boolean).join("\n"),
  );

  /** Tracé de la latence sur la fenêtre glissante ; un point sans réponse coupe la courbe et laisse une marque. */
  function trace(w: number, h: number) {
    app.linkRev;
    const end = clock.now;
    const pts = spread(linkLog.filter((p) => p.ts >= end - MINI_MS));
    const max = Math.max(200, ...pts.map((p) => p.rttMs ?? 0));
    const x = (ts: number) => ((ts - (end - MINI_MS)) / MINI_MS) * w;
    let d = "";
    let pen = false;
    const lost: { x: number; outcome: Outcome }[] = [];
    for (const p of pts) {
      if (p.rttMs == null) {
        lost.push({ x: x(p.x), outcome: p.outcome });
        pen = false;
        continue;
      }
      d += `${pen ? "L" : "M"}${x(p.x).toFixed(1)} ${(h - 2 - (p.rttMs / max) * (h - 6)).toFixed(1)}`;
      pen = true;
    }
    return { d, lost, max, n: pts.length };
  }
  const mini = $derived(trace(72, 20));
</script>

{#snippet spark(tr: ReturnType<typeof trace>, w: number, h: number)}
  <svg class="spark" viewBox="0 0 {w} {h}" width={w} height={h} aria-hidden="true">
    <line x1="0" x2={w} y1={h - 1} y2={h - 1} class="base" />
    <path d={tr.d} class="rtt" />
    {#each tr.lost as l}<line x1={l.x} x2={l.x} y1={h - 6} y2={h} class="lost {l.outcome}" />{/each}
  </svg>
{/snippet}

<button class="chip {meta.tone}" popovertarget="link-panel" title={tip} aria-label={t("link.chipAria", { label: meta.label, quality: qualityLabel(q) })}>
  <span class="swatch"></span>
  <span class="lbl">{meta.label}</span>
  {#if s}
    <span class="sep" aria-hidden="true"></span>
    {@render spark(mini, 72, 20)}
    <span class="num rtt-val">{s.link.rttMs != null ? `${s.link.rttMs} ms` : t("outcome.timeout")}</span>
  {/if}
</button>

<div id="link-panel" popover class="panel" ontoggle={(e) => (open = (e as ToggleEvent).newState === "open")}>
  <h2>{t("sync.title")}</h2>
  <div class="sync {meta.tone}">
    <span class="badge tone">{meta.label}</span>
    {#if sync.reason}<p>{sync.reason}</p>{/if}
  </div>
  <dl>
    <dt>{t("sync.expected")}</dt><dd>{sync.expected || "—"}</dd>
    <dt>{t("sync.measured")}</dt><dd>{sync.measured || "—"}</dd>
    {#if s?.down}<dt>{t("common.battery")}</dt><dd class="bad">{downText(s.down)}</dd>{/if}
    {#if s}<dt>{t("link.source")}</dt><dd>{s.cloud ? t("link.sourceCloud", { age: fmtAge(cloudAge(s, clock.now) ?? 0) }) : s.down ? t("link.sourceNone") : t("link.sourceLocal")}</dd>{/if}
  </dl>
  <p class="muted note">{t("sync.note")}</p>

  <h2 class="link-h">{t("link.title")}</h2>
  {#if s}
    {#if open}<LinkCharts />{/if}
    <dl class="stats">
      <dt>{t("link.median")}</dt><dd class="num">{s.link.rttMs != null ? `${s.link.rttMs} ms` : "—"}</dd>
      <dt>{t("link.responses")}</dt><dd class="num">{t("link.answers", { ok: s.link.ok, total: s.link.total })}</dd>
      <dt>{t("link.loss")}</dt><dd class="num" class:bad={loss != null && loss >= 20}>{loss != null ? `${loss} %` : "—"}</dd>
      <dt>{t("link.rssi")}</dt><dd class="num">{s.link.rssi != null ? `${s.link.rssi} dBm` : "—"}</dd>
      <dt>{t("link.quality")}</dt><dd>{qualityLabel(q)}</dd>
    </dl>
  {:else}
    <p class="muted">{t("link.none")}</p>
  {/if}
</div>

<style>
  .chip {
    display: inline-flex; align-items: center; gap: 8px; min-height: 32px; padding: 0 12px 0 12px;
    border: 1px solid color-mix(in oklab, var(--k) 55%, var(--border)); border-radius: 999px;
    background: color-mix(in oklab, var(--k) calc(var(--tint) - 8%), var(--surface)); cursor: pointer;
    font-size: var(--text-sm); font-weight: 600; white-space: nowrap;
    transition: background-color var(--dur-micro), border-color var(--dur-micro);
  }
  .chip:hover { border-color: var(--k); background: color-mix(in oklab, var(--k) var(--tint), var(--surface)); }
  .sep { width: 1px; align-self: stretch; margin: 8px 0; background: var(--border); }
  .rtt-val { font-weight: 500; color: var(--muted); min-width: 7ch; }
  .spark { display: block; overflow: visible; }
  .spark .base { stroke: var(--border); stroke-width: 1; }
  .spark .rtt { fill: none; stroke: var(--auto); stroke-width: 1.5; stroke-linejoin: round; }
  .spark .lost { stroke: var(--danger); stroke-width: 1.5; }
  .spark .lost.portClosed { stroke-dasharray: 2 2; }
  .spark .lost.refused { stroke: var(--warn); }

  .panel {
    position: fixed; inset: 56px 16px auto auto; margin: 0; width: 560px; max-width: calc(100% - 32px);
    max-height: calc(100% - 72px); overflow: auto;
    padding: 20px; background: var(--surface); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius);
  }
  .panel h2 { font-size: var(--text-md); margin-bottom: 12px; }
  .link-h { margin-top: 24px; padding-top: 20px; border-top: 1px solid var(--border); }
  .sync { display: grid; gap: 8px; margin-bottom: 12px; justify-items: start; }
  .sync p { color: var(--k); font-weight: 500; }
  .note { margin-top: 12px; font-size: var(--text-sm); }
  .stats { margin-top: 16px; }
  dd.bad { color: var(--danger); font-weight: 600; }
</style>
