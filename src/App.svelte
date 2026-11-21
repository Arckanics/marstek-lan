<script lang="ts">
  import { onMount } from "svelte";
  import { app, clock, start } from "./lib/state.svelte";
  import Devices from "./components/Devices.svelte";
  import Status from "./components/Status.svelte";
  import Modes from "./components/Modes.svelte";
  import Planning from "./components/Planning.svelte";
  import History from "./components/History.svelte";
  import Settings from "./components/Settings.svelte";
  import DevicePanel from "./components/DevicePanel.svelte";
  import Journal from "./components/Journal.svelte";
  import { QUALITY_LABEL, linkTip, quality } from "./lib/link";

  const tabs = [
    { id: "devices", label: "Appareils" },
    { id: "status", label: "État" },
    { id: "modes", label: "Modes" },
    { id: "planning", label: "Planning" },
    { id: "history", label: "Historique" },
    { id: "device", label: "Appareil" },
    { id: "settings", label: "Réglages" },
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
</script>

<div class="shell">
  <header>
    <h1>Marstek Control</h1>
    <div class="tabs" role="tablist" aria-label="Sections" tabindex="-1" onkeydown={onKey}>
      {#each tabs as t}
        <button
          role="tab"
          id="tab-{t.id}"
          aria-selected={app.tab === t.id}
          aria-controls="panel"
          tabindex={app.tab === t.id ? 0 : -1}
          onclick={() => (app.tab = t.id)}>{t.label}</button>
      {/each}
    </div>
    <div class="conn" aria-live="polite">
      {#if app.busy}
        <span class="badge"><span class="spinner" aria-hidden="true"></span>Écriture en cours{app.progress ? ` · ${app.progress}` : ""}</span>
      {/if}
      {#if app.connectedIp && app.snapshot}
        {@const q = quality(app.snapshot, clock.now)}
        <span class="badge" title={linkTip(app.snapshot)}><span class="dot {q}"></span>Liaison {QUALITY_LABEL[q]}</span>
      {/if}
      <span class="badge">
        <span class="dot" class:on={app.connectedIp}></span>
        {app.connectedIp ? `${device?.name ?? "Batterie"} · ${app.connectedIp}` : "Non connecté"}
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
  .shell { height: 100%; display: grid; grid-template-rows: auto minmax(0, 1fr) auto; }
  header {
    display: flex; align-items: center; gap: 24px; padding: 10px 20px;
    background: var(--surface); border-bottom: 1px solid var(--border);
  }
  h1 { font-size: 1rem; letter-spacing: 0.01em; white-space: nowrap; }
  .tabs { display: flex; gap: 4px; flex: 1; }
  .tabs button {
    min-height: 34px; padding: 0 14px; background: none; border: 0; border-radius: 8px;
    cursor: pointer; color: var(--muted); font-weight: 500;
  }
  .tabs button:hover { background: var(--surface-2); color: var(--text); }
  .tabs button[aria-selected="true"] { background: var(--surface-2); color: var(--text); box-shadow: inset 0 -2px 0 var(--text); }
  .conn { display: flex; gap: 8px; }
  main { overflow: auto; padding: 20px; }
</style>
