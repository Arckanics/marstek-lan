<script lang="ts">
  import { app, scan, stopScan, connectTo, portCount, setting } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";
  import Confirm from "./Confirm.svelte";
  import Hint from "./Hint.svelte";

  let confirm: Confirm;
  /** Au plus 50 paquets par seconde et par adresse. */
  const minutes = (ports: number) => Math.max(1, Math.round(ports / 50 / 60));
  const ask = (message: string) => confirm.ask(`${message} ${t("devices.risk")}`, { title: t("devices.manyPorts"), ok: t("common.launch") });
  const askAuto = () =>
    ask(t("devices.autoWarn"));

  let range = $state(app.settings.scanRange);
  let portsText = $state(app.settings.ports);
  // Un port trouvé par la recherche automatique complète le réglage.
  $effect(() => {
    portsText = app.settings.ports;
  });

  async function run(e: SubmitEvent) {
    e.preventDefault();
    const ports = portsText.trim();
    const n = portCount(ports);
    // Sans plage ni ports saisis, c'est la recherche automatique.
    const ok = n > 20 ? await ask(t("devices.bigScan", { n, min: minutes(n) })) : !range.trim() && !ports ? await askAuto() : true;
    if (!ok) return;
    setting("ports", ports);
    setting("scanRange", range.trim());
    scan(range.trim());
  }

  async function auto() {
    if (await askAuto()) scan("", true);
  }
</script>

<div class="stack">
  <form class="card row" onsubmit={run}>
    <label class="field grow">
      {t("devices.range")}
      <input bind:value={range} placeholder="192.168.1.1-254 ou 192.168.1.0/24" spellcheck="false" aria-describedby="scan-hint" />
    </label>
    <label class="field port">
      {t("devices.ports")}
      <input bind:value={portsText} placeholder="30000, 49152-65535" spellcheck="false" aria-invalid={app.scanError !== ""} aria-describedby="scan-hint" />
    </label>
    <button class="btn primary" disabled={app.scanning}>
      {#if app.scanning}<span class="spinner" aria-hidden="true"></span>{t("devices.scanning")}{:else}{t("devices.scan")}{/if}
    </button>
    <button type="button" class="btn" disabled={app.scanning} onclick={auto}>{t("devices.auto")}</button>
    {#if app.scanning}<button type="button" class="btn" onclick={stopScan}>{t("common.cancel")}</button>{/if}
    {#if app.scanProgress}
      <div class="full" role="status">
        <progress value={app.scanProgress.sent} max={app.scanProgress.total || 1}></progress>
        <span class="muted">{t("devices.progress", { stage: app.scanProgress.stage, sent: app.scanProgress.sent, total: app.scanProgress.total, found: app.scanProgress.found })}</span>
      </div>
    {/if}
    {#if app.scanError}<p class="full" role="alert">{app.scanError}</p>{/if}
    <div id="scan-hint" class="full"><Hint id="scan" /></div>
  </form>

  {#if app.devices.length}
    <ul class="list" aria-label={t("devices.found")}>
      {#each app.devices as d (`${d.ip}:${d.port}`)}
        <li>
          <button class="card dev" onclick={() => connectTo(d.ip, d.port)} aria-current={d.ip === app.connectedIp}>
            <span class="name">{d.name ?? t("devices.marstek")}</span>
            <span class="num muted">{d.ip}{`:${d.port}`}</span>
            <span class="muted">{d.version ? t("devices.version", { v: d.version }) : t("devices.versionUnknown")}</span>
            {#if d.ip === app.connectedIp}<span class="badge tone k-ok">{t("devices.connected")}</span>
            {:else}<span class="muted act">{t("devices.connect")}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {:else if app.scanning}
    <div class="empty" role="status"><span class="spinner" aria-hidden="true"></span><p>{t("devices.searching")}</p></div>
  {:else}
    <div class="empty">
      <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" /><path d="M6.3 6.3a8 8 0 0 0 0 11.4M17.7 6.3a8 8 0 0 1 0 11.4M3.5 3.5a12 12 0 0 0 0 17M20.5 3.5a12 12 0 0 1 0 17" /></svg>
      <strong>{t("devices.none")}</strong>
      <p>{t("devices.noneHint")}</p>
    </div>
  {/if}
</div>

<Confirm bind:this={confirm} />

<style>
  .grow { flex: 1; min-width: 260px; }
  .port { width: 220px; }
  progress { width: 100%; }
  .full { flex-basis: 100%; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
  .dev {
    width: 100%; display: grid; gap: 4px; text-align: left; cursor: pointer;
    transition: border-color 0.12s, background-color 0.12s;
  }
  .dev:hover { border-color: var(--muted); background: var(--surface-2); }
  .dev[aria-current="true"] { border-color: var(--ok); box-shadow: inset 3px 0 0 var(--ok); }
  .name { font-weight: 600; font-size: 1.05rem; }
  .act, .dev .badge { margin-top: 8px; justify-self: start; }
</style>
