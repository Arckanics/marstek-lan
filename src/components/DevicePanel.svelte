<script lang="ts">
  import { setBle, setDod, setLed } from "../lib/api";
  import { app, clock, powerMax, setting, write } from "../lib/state.svelte";
  import { fmtKwh, fmtW } from "../lib/energy";
  import { cloudAge, fmtAge } from "../lib/link";
  import { locale, t } from "../lib/i18n.svelte";
  import { DOD_RANGE, UNKNOWN_MAX, limits, slotCount } from "../lib/limits";
  import Hint from "./Hint.svelte";
  import NotConnected from "./NotConnected.svelte";

  const s = $derived(app.snapshot);
  // API locale coupée : le cloud ne fournit que le solaire total (ni tension/courant par entrée, ni CT, ni compteurs).
  const cloud = $derived(s?.cloud);
  const cloudSince = $derived(s ? cloudAge(s, clock.now) : undefined);
  const off = $derived(app.busy);
  let dod = $state(80);
  const lim = $derived(limits(s));
  const noSys = $derived(off || lim.sys === false);
  const cap = $derived(powerMax());
  const modelMax = $derived(lim.model?.max ?? undefined);

  function saveMax(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const v = input.valueAsNumber;
    if (Number.isNaN(v) || v <= 0) {
      input.value = String(cap);
      return;
    }
    const w = Math.min(Math.round(v), modelMax ?? Infinity);
    input.value = String(w);
    setting("maxPower", w);
  }

  const rssiText = (r: number | undefined) => (r == null ? "—" : `${r} dBm, ${t(r >= -60 ? "device.rssi.good" : r >= -75 ? "device.rssi.fair" : "device.rssi.poor")}`);
  const v = (x: number | undefined, unit: string, digits = 0) =>
    x == null ? "—" : `${x.toLocaleString(locale(), { maximumFractionDigits: digits })} ${unit}`;

  const pvRows = $derived(
    ([1, 2, 3, 4] as const).map((n) => ({
      n,
      p: s?.pv?.[`pv${n}Power`],
      u: s?.pv?.[`pv${n}Voltage`],
      i: s?.pv?.[`pv${n}Current`],
    })),
  );
</script>

<div class="stack">
{#if !app.connectedIp}
  <NotConnected />
{:else}
  {#if cloud}
    <p class="muted cloudnote" role="status"><span class="badge tone k-warn">{t("cloud.badge", { age: fmtAge(cloudSince ?? 0) })}</span> {t("device.cloudNote")}</p>
  {/if}
  <div class="grid2">
    <section class="card" aria-label={t("device.wireless")}>
      <h3>{t("device.connectivity")}</h3>
      <dl>
        <dt>Wi-Fi (SSID)</dt><dd>{s?.wifi?.ssid ?? "—"}</dd>
        <dt>{t("device.wifiSignal")}</dt><dd class="num">{rssiText(s?.wifi?.rssi)}</dd>
        <dt>{t("device.ip")}</dt><dd class="num">{s?.wifi?.staIp ?? app.connectedIp}</dd>
        <dt>{t("device.wifiMac")}</dt><dd class="num">{s?.wifi?.wifiMac ?? s?.device?.wifiMac ?? "—"}</dd>
        <dt>Bluetooth</dt><dd>{s?.ble?.state ?? "—"}</dd>
        <dt>{t("device.bleMac")}</dt><dd class="num">{s?.ble?.bleMac ?? s?.device?.bleMac ?? "—"}</dd>
      </dl>
    </section>

    <section class="card" aria-label={t("device.ct")}>
      <h3>{t("device.ct")}</h3>
      {#if s?.em?.ctState === 1}
        <dl>
          <dt>{t("common.state")}</dt><dd><span class="badge tone k-ok">{t("devices.connected")}</span></dd>
          <dt>{t("device.phase", { p: "A" })}</dt><dd class="num">{v(s.em.aPower, "W")}</dd>
          <dt>{t("device.phase", { p: "B" })}</dt><dd class="num">{v(s.em.bPower, "W")}</dd>
          <dt>{t("device.phase", { p: "C" })}</dt><dd class="num">{v(s.em.cPower, "W")}</dd>
          <dt>{t("common.total")}</dt><dd class="num"><strong>{v(s.em.totalPower, "W")}</strong></dd>
        </dl>
      {:else}
        <p class="muted">{s?.em ? t("device.noCt") : t("device.waitFirst")}</p>
      {/if}
    </section>

    <section class="card" aria-label={t("device.pvInputs")}>
      <h3>{t("device.panels")}</h3>
      {#if cloud}
        <p class="muted cloudpv">{t("device.cloudTotal")} <strong class="num">{fmtW(cloud.pvW)}</strong></p>
      {/if}
      <table>
        <thead><tr><th>{t("device.input")}</th><th>{t("common.power")}</th><th>{t("device.voltage")}</th><th>{t("device.current")}</th></tr></thead>
        <tbody>
          {#each pvRows as r}
            <tr><td>PV {r.n}</td><td class="num">{v(r.p, "W")}</td><td class="num">{v(r.u, "V", 1)}</td><td class="num">{v(r.i, "A", 2)}</td></tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section class="card" aria-label={t("device.counters")}>
      <h3>{t("device.counters")}</h3>
      <dl>
        <dt>{t("device.pvProduced")}</dt><dd class="num">{fmtKwh(s?.es?.totalPvEnergy, 100)}</dd>
        <dt>{t("device.gridIn")}</dt><dd class="num">{fmtKwh(s?.es?.totalGridInputEnergy, 1000)}</dd>
        <dt>{t("device.gridOut")}</dt><dd class="num">{fmtKwh(s?.es?.totalGridOutputEnergy, 1000)}</dd>
        <dt>{t("device.loadTotal")}</dt><dd class="num">{fmtKwh(s?.es?.totalLoadEnergy, 1000)}</dd>
        <dt>{t("device.ctIn")}</dt><dd class="num">{fmtKwh(s?.em?.inputEnergy, 10000)}</dd>
        <dt>{t("device.ctOut")}</dt><dd class="num">{fmtKwh(s?.em?.outputEnergy, 10000)}</dd>
      </dl>
    </section>

    <section class="card wide" aria-label={t("device.limits")}>
      <h3>{t("device.limits")}</h3>
      <div class="limits">
        <table>
          <thead><tr><th>{t("device.quantity")}</th><th>{t("device.value")}</th><th>{t("device.source")}</th></tr></thead>
          <tbody>
            <tr><td>{t("device.model")}</td><td>{lim.model?.name ?? lim.device ?? t("common.unknown")}</td><td class="src">{lim.device ? t("device.read") : "—"}</td></tr>
            <tr><td>Firmware</td><td class="num">{lim.firmware ?? "—"}</td><td class="src">{lim.firmware != null ? t("device.read") : "—"}</td></tr>
            <tr><td>{t("status.rated")}</td><td class="num">{v(lim.rated, "Wh")}</td><td class="src">{lim.rated != null ? t("device.read") : "—"}</td></tr>
            <tr><td>{t("status.remaining")}</td><td class="num">{v(lim.remaining, "Wh")}</td><td class="src">{lim.remaining != null ? t("device.read") : "—"}</td></tr>
            <tr><td>{t("device.modelMax")}</td><td class="num">{modelMax != null ? `${modelMax} W` : t("device.unknownF")}</td><td class="src ded">{t("device.deducedModel")}</td></tr>
            <tr><td>{t("device.slots")}</td><td class="num">{slotCount(lim)}</td><td class="src ded">{t("device.deducedModel")}</td></tr>
            <tr><td>{t("device.dodRange")}</td><td class="num">{t("device.dodRangeValue", { lo: DOD_RANGE[0], hi: DOD_RANGE[1] })}</td><td class="src ded">{t("device.deducedModel")}</td></tr>
            <tr><td>{t("device.sys")}</td><td>{lim.sys == null ? "—" : lim.sys ? t("device.sysYes") : t("device.sysNo")}</td><td class="src ded">{t("device.deducedFirmware")}</td></tr>
          </tbody>
        </table>
        <div class="item">
          <label class="field" for="maxp">{t("device.maxLabel")}</label>
          <input id="maxp" class="num" type="number" min="50" max={modelMax} step="50" value={cap} onchange={saveMax} />
          <p class="muted">{modelMax != null ? t("device.maxKnown", { max: modelMax }) : t("device.maxUnknown", { max: UNKNOWN_MAX })} {t("device.maxApplies")}</p>
          <Hint id="maxPower" />
        </div>
      </div>
    </section>

    <section class="card wide" aria-label={t("device.settings")}>
      <h3>{t("tab.settings")}</h3>
      {#if lim.sys === false}<p class="muted">{t("device.fwWarn", { fw: lim.firmware ?? "" })}</p>{/if}
      <div class="settings">
        <div class="item">
          <label class="field" for="dod">{t("device.dod")} <strong class="num">{dod} %</strong></label>
          <input id="dod" type="range" min="30" max="88" bind:value={dod} />
          <Hint id="dod" />
          <button class="btn" disabled={noSys} onclick={() => write(`DOD ${dod} %`, () => setDod(dod))}>{t("common.apply")}</button>
        </div>
        <div class="item">
          <span class="lab">{t("device.led")}</span>
          <div class="seg" role="group" aria-label={t("device.led")}>
            <button disabled={noSys} onclick={() => write(t("device.ledOnLog"), () => setLed(true))}>{t("device.ledOn")}</button>
            <button disabled={noSys} onclick={() => write(t("device.ledOffLog"), () => setLed(false))}>{t("device.ledOff")}</button>
          </div>
          <Hint id="led" />
        </div>
        <div class="item">
          <span class="lab">{t("device.ble")}</span>
          <div class="seg" role="group" aria-label={t("device.ble")}>
            <button disabled={noSys} onclick={() => write(t("device.bleOnLog"), () => setBle(true))}>{t("device.bleOn")}</button>
            <button disabled={noSys} onclick={() => write(t("device.bleOffLog"), () => setBle(false))}>{t("device.bleOff")}</button>
          </div>
          <Hint id="ble" />
        </div>
      </div>
    </section>
  </div>
{/if}
</div>

<style>
  .wide { grid-column: 1 / -1; }
  .cloudnote { font-size: var(--text-sm); margin-bottom: 4px; }
  .cloudpv { font-size: var(--text-sm); margin: 0 0 10px; }
  .settings { display: grid; grid-template-columns: 2fr 1fr 1fr; gap: 24px; }
  .item { display: grid; gap: 8px; align-content: start; }
  .item .btn, .item .seg { justify-self: start; }
  .lab { font-size: 0.85rem; color: var(--muted); }
  .field strong { color: var(--text); }
  .limits { display: grid; grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr); gap: 24px; align-items: start; }
  .limits input { min-height: 34px; padding: 0 12px; max-width: 160px; background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius-sm); }
  .src { color: var(--muted); font-size: var(--text-sm); }
  .src.ded { font-style: italic; }
  @media (max-width: 900px) { .limits, .settings { grid-template-columns: minmax(0, 1fr); } }
</style>
