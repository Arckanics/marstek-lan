<script lang="ts">
  import { app, clock, view } from "../lib/state.svelte";
  import { locale, t, type Key } from "../lib/i18n.svelte";
  import { cloudFlows, flows, fmtW, mergeFlows } from "../lib/energy";
  import type { CloudReading } from "../lib/api";
  import { cloudAge, fmtAge } from "../lib/link";
  import NotConnected from "./NotConnected.svelte";
  import FlowDiagram from "./FlowDiagram.svelte";
  import SocGauge from "./SocGauge.svelte";
  import Chart from "./Chart.svelte";
  import Stale from "./Stale.svelte";

  const modeNames: Record<string, [string, string]> = { Auto: ["Auto", "k-auto"], AI: ["mode.ai", "k-ai"], Manual: ["mode.manual", "k-none"], Passive: ["mode.passive", "k-none"], UPS: ["UPS", "k-ups"] };

  const s = $derived(app.snapshot);
  const cloud = $derived(s?.cloud);
  const cloudSince = $derived(s ? cloudAge(s, clock.now) : undefined);
  const live = $derived(!cloud && view.state !== "none");
  const f = $derived(cloud ? cloudFlows(cloud) : live ? view.flows : flows(s));
  const soc = $derived(cloud ? (cloud.soc ?? undefined) : live ? view.soc : (s?.bat?.soc ?? s?.es?.batSoc));
  const age = $derived(Math.max(0, Math.round(view.age)));
  const last = $derived(`${t("flow.pv")} ${fmtW(view.last.pv)} · ${t("flow.bat")} ${fmtW(view.last.bat)} · ${t("flow.grid")} ${fmtW(view.last.grid)} · ${t("flow.house")} ${fmtW(view.last.house)}${view.lastSoc != null ? ` · ${view.lastSoc} %` : ""}`);
  const num = (v: number | undefined, unit: string) => (v == null ? "—" : `${v.toLocaleString(locale())} ${unit}`);
</script>

{#if !app.connectedIp}
  <NotConnected />
{:else if !s}
  <div class="empty" role="status"><span class="spinner" aria-hidden="true"></span><p>{t("status.connecting", { ip: app.connectedIp })}</p></div>
{:else}
  <div class="stack">
    {#if cloud}
      <p class="muted live" role="status"><span class="badge tone k-warn">{t("cloud.badge", { age: fmtAge(cloudSince ?? 0) })}</span> {t("cloud.note")}</p>
    {/if}
    {#if live}
      <p class="muted live" role="status" title={t("status.lastTitle", { last })}>
        <span class="dot" class:wait={view.state === "waiting"} aria-hidden="true"></span>
        {t("status.lag", { lag: view.lag, age })}
        {#if view.state === "waiting"}{t("status.waiting")}{:else if view.state === "held"}{t("status.held")}{/if}
        <span class="last">{t("status.last", { last })}</span>
      </p>
    {/if}
    <div class="top">
      <section class="card" aria-label={t("status.flowTitle")}>
        <h3>{t("status.flowTitle")}</h3>
        {#if !cloud}<Stale keys={["es"]} />{/if}
        <FlowDiagram {f} />
      </section>
      <section class="card side" aria-label={t("common.battery")}>
        <h3>{t("common.battery")}</h3>
        {#if !cloud}<Stale keys={["bat"]} />{/if}
        <SocGauge {soc} bat={f.bat} />
        {#if !cloud}
        <dl>
          <dt>{t("common.mode")}</dt><dd>{#if s.mode?.mode}{@const m = modeNames[s.mode.mode] ?? [s.mode.mode, "k-none"]}<span class="badge tone {m[1]}">{m[0].includes(".") ? t(m[0] as Key) : m[0]}</span>{:else}{t("common.unknown")}{/if}</dd>
          <dt>{t("status.temperature")}</dt><dd class="num">{num(s.bat?.batTemp, "°C")}</dd>
          <dt>{t("status.remaining")}</dt><dd class="num">{num(s.bat?.batCapacity ?? s.es?.batCap, "Wh")}</dd>
          <dt>{t("status.rated")}</dt><dd class="num">{num(s.bat?.ratedCapacity, "Wh")}</dd>
        </dl>
        {/if}
      </section>
    </div>
    <section class="card" aria-label={t("common.history")}>
      <h3>{t("status.lastHour")}</h3>
      <Chart />
    </section>
  </div>
{/if}

<style>
  .live { font-size: var(--text-sm); }
  .last { display: block; font-variant-numeric: tabular-nums; }
  .dot { display: inline-block; width: 8px; height: 8px; margin-right: 6px; border-radius: 50%; background: var(--charge); }
  .dot.wait { background: var(--grid); animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.25; } }
  .top { display: grid; grid-template-columns: minmax(0, 1fr) 280px; gap: 16px; }
  .side { display: grid; gap: 16px; align-content: start; }
</style>
