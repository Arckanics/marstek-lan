<script lang="ts">
  import { onMount } from "svelte";
  import { app, start, view } from "./lib/state.svelte";
  import { closeWin, minimizeWin, toggleMaximize, startResize } from "./lib/window";
  import { t } from "./lib/i18n.svelte";

  // Poignées de redimensionnement (la fenêtre sans décorations n'a plus celles du système).
  const handles = [
    ["n", "North"], ["s", "South"], ["e", "East"], ["w", "West"],
    ["ne", "NorthEast"], ["nw", "NorthWest"], ["se", "SouthEast"], ["sw", "SouthWest"],
  ] as const;
  import Devices from "./components/Devices.svelte";
  import Status from "./components/Status.svelte";
  import Modes from "./components/Modes.svelte";
  import Planning from "./components/Planning.svelte";
  import History from "./components/History.svelte";
  import Settings from "./components/Settings.svelte";
  import DevicePanel from "./components/DevicePanel.svelte";
  import Journal from "./components/Journal.svelte";
  import LinkMonitor from "./components/LinkMonitor.svelte";
  import Icon from "./components/Icon.svelte";

  const tabs = [
    { id: "devices", label: "tab.devices", icon: "devices" },
    { id: "status", label: "tab.status", icon: "status" },
    { id: "modes", label: "tab.modes", icon: "modes" },
    { id: "planning", label: "tab.planning", icon: "planning" },
    { id: "history", label: "tab.history", icon: "history" },
    { id: "device", label: "tab.device", icon: "device" },
    { id: "settings", label: "tab.settings", icon: "settings" },
  ] as const;

  onMount(start);

  function onKey(e: KeyboardEvent) {
    const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!d) return;
    const i = tabs.findIndex((t) => t.id === app.tab);
    app.tab = tabs[(i + d + tabs.length) % tabs.length].id;
    queueMicrotask(() => document.getElementById(`tab-${app.tab}`)?.focus());
  }

  const device = $derived(app.devices.find((d) => d.ip === app.connectedIp));
  const raw = $derived(view.state !== "none" ? view.soc : (app.snapshot?.bat?.soc ?? app.snapshot?.es?.batSoc));
  const soc = $derived(raw == null ? raw : Math.round(raw));
</script>

<div class="shell">
  {#each handles as [cls, dir]}<div class="rz {cls}" onmousedown={startResize(dir)} role="presentation"></div>{/each}
  <header data-tauri-drag-region>
    <div class="lights">
      <button class="light close" onclick={closeWin} aria-label={t("win.close")}></button>
      <button class="light min" onclick={minimizeWin} aria-label={t("win.minimize")}></button>
      <button class="light max" onclick={toggleMaximize} aria-label={t("win.maximize")}></button>
    </div>
    <h1 data-tauri-drag-region>Marstek LAN</h1>
    <div class="tabs" role="tablist" aria-label={t("app.sections")} tabindex="-1" onkeydown={onKey} data-tauri-drag-region>
      {#each tabs as tab}
        <button
          role="tab"
          id="tab-{tab.id}"
          aria-selected={app.tab === tab.id}
          aria-controls="panel"
          aria-label={t(tab.label)}
          title={t(tab.label)}
          tabindex={app.tab === tab.id ? 0 : -1}
          onclick={() => (app.tab = tab.id)}><Icon name={tab.icon} /></button>
      {/each}
    </div>
    <div class="conn" data-tauri-drag-region>
      {#if app.busy}
        <span class="badge" role="status"><span class="spinner" aria-hidden="true"></span>{t("app.writing")}{#if app.progress}<span class="muted prog">{app.progress}</span>{/if}</span>
      {/if}
      <LinkMonitor />
      <span class="dev" class:off={!app.connectedIp}>
        {#if app.connectedIp}
          <strong>{device?.name ?? t("common.battery")}</strong>
          <span class="muted num">{app.connectedIp}</span>
          {#if soc != null}<span class="num soc">{soc} %</span>{/if}
        {:else}{t("app.notConnected")}{/if}
      </span>
    </div>
  </header>

  <main>
    {#if app.ready}
      <div id="panel" role="tabpanel" aria-labelledby="tab-{app.tab}">
        {#if app.tab === "devices"}<Devices />
        {:else if app.tab === "status"}<Status />
        {:else if app.tab === "modes"}<Modes />
        {:else if app.tab === "planning"}<Planning />
        {:else if app.tab === "history"}<History />
        {:else if app.tab === "device"}<DevicePanel />
        {:else}<Settings />{/if}
      </div>
    {/if}
  </main>

  <Journal />
</div>

<style>
  .shell { position: relative; height: 100%; display: grid; grid-template-rows: auto minmax(0, 1fr) auto; }
  /* Feux macOS de la barre de titre. */
  .lights { display: flex; gap: 8px; align-items: center; padding-right: 4px; }
  .light { width: 19px; height: 19px; padding: 0; border: none; border-radius: 50%; cursor: pointer; display: grid; place-content: center; font-size: 14px; line-height: 1; color: rgb(0 0 0 / 0.55); }
  .light.close { background: #ff5f57; }
  .light.min { background: #febc2e; }
  .light.max { background: #28c840; }
  .light::before { opacity: 0; transition: opacity var(--dur-micro); }
  .lights:hover .light::before { opacity: 1; }
  .light.close::before { content: "\00d7"; }
  .light.min::before { content: "\2013"; }
  .light.max::before { content: "+"; }
  .light:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
  /* Poignées de redimensionnement (fenêtre sans décorations). */
  .rz { position: absolute; z-index: 20; }
  .rz.n { top: 0; left: 8px; right: 8px; height: 5px; cursor: ns-resize; }
  .rz.s { bottom: 0; left: 8px; right: 8px; height: 5px; cursor: ns-resize; }
  .rz.e { top: 8px; bottom: 8px; right: 0; width: 5px; cursor: ew-resize; }
  .rz.w { top: 8px; bottom: 8px; left: 0; width: 5px; cursor: ew-resize; }
  .rz.ne { top: 0; right: 0; width: 10px; height: 10px; cursor: nesw-resize; }
  .rz.nw { top: 0; left: 0; width: 10px; height: 10px; cursor: nwse-resize; }
  .rz.se { bottom: 0; right: 0; width: 10px; height: 10px; cursor: nwse-resize; }
  .rz.sw { bottom: 0; left: 0; width: 10px; height: 10px; cursor: nesw-resize; }
  header {
    display: flex; align-items: center; gap: 20px; padding: 0 16px 0 20px; min-height: 52px;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }
  h1 { font-size: var(--text-md); white-space: nowrap; }
  .tabs { display: flex; gap: 2px; flex: 1; align-self: stretch; min-width: 0; overflow-x: auto; }
  .tabs button {
    display: inline-flex; align-items: center; gap: 8px; padding: 0 12px; background: none; border: 0; cursor: pointer; color: var(--muted); font-weight: 500;
    box-shadow: inset 0 -2px 0 transparent; white-space: nowrap;
    transition: color var(--dur-micro), box-shadow var(--dur-micro);
  }
  .tabs button:hover { color: var(--text); box-shadow: inset 0 -2px 0 var(--border); }
  .tabs button[aria-selected="true"] {
    color: var(--text); font-weight: 600; box-shadow: inset 0 -2px 0 var(--text);
    background: linear-gradient(to top, color-mix(in oklab, var(--focus) var(--glow-soft), transparent), transparent 75%);
  }
  .tabs button:focus-visible { outline-offset: -2px; }
  .conn { display: flex; align-items: center; gap: 12px; }
  .prog { max-width: 28ch; overflow: hidden; text-overflow: ellipsis; }
  .dev { display: inline-flex; align-items: baseline; gap: 8px; white-space: nowrap; font-size: var(--text-sm); }
  .dev.off { color: var(--muted); }
  .soc { font-weight: 600; }
  main { overflow: auto; padding: 20px; }
</style>
