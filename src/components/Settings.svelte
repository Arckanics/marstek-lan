<script lang="ts">
  import { onMount } from "svelte";
  import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
  import { app, errText, setting } from "../lib/state.svelte";
  import { LANGS, setLang, t, type Lang } from "../lib/i18n.svelte";
  import { clone } from "../lib/store";
  import Account from "./Account.svelte";
  import Hint from "./Hint.svelte";

  let notify = $state(clone(app.settings.notify));
  const save = () => setting("notify", clone(notify));

  const pollNote = $derived(
    app.settings.pollSeconds != null
      ? t("settings.pollChosen", { s: app.snapshot?.pollSecs ?? app.settings.pollSeconds })
      : t("settings.pollDefault", { s: app.snapshot?.pollSecs || 240 }),
  );

  function savePoll(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const v = input.valueAsNumber;
    const secs = Number.isNaN(v) ? undefined : Math.min(600, Math.max(30, Math.round(v)));
    input.value = secs == null ? "" : String(secs);
    setting("pollSeconds", secs);
  }

  function toggleTune(e: Event) {
    const on = (e.currentTarget as HTMLInputElement).checked;
    setting("autoTune", on);
    // Repartir de zéro à la prochaine activation : on oublie la cadence apprise.
    if (!on) setting("tunedPoll", undefined);
  }

  function chooseLang(lang: Lang) {
    setLang(lang);
    setting("lang", lang);
  }

  let autostart = $state(false);
  let error = $state("");

  onMount(() => {
    isEnabled().then((v) => (autostart = v), (e) => (error = errText(e)));
  });

  async function toggle() {
    error = "";
    try {
      await (autostart ? enable() : disable());
    } catch (e) {
      autostart = !autostart;
      error = errText(e);
    }
  }
</script>

<div class="stack">
  <Account />
  <section class="card" aria-label={t("settings.language")}>
    <h3>{t("settings.language")}</h3>
    <select value={app.settings.lang} aria-label={t("settings.language")} onchange={(e) => chooseLang(e.currentTarget.value as Lang)}>
      {#each LANGS as l}<option value={l.code}>{l.name}</option>{/each}
    </select>
  </section>

  <section class="card" aria-label={t("settings.notifications")}>
    <h3>{t("settings.notifications")}</h3>
    <div class="intro"><Hint id="notify" /></div>
    <div class="list">
      <div class="item">
        <label class="check"><input type="checkbox" bind:checked={notify.lowSoc.enabled} onchange={save} />{t("settings.lowSoc")}</label>
        <label class="field">{t("settings.threshold")}<input type="number" min="0" max="100" bind:value={notify.lowSoc.soc} disabled={!notify.lowSoc.enabled} onchange={save} /></label>
      </div>
      <label class="check"><input type="checkbox" bind:checked={notify.offline} onchange={save} />{t("settings.offline")}</label>
      <label class="check"><input type="checkbox" bind:checked={notify.modeChanged} onchange={save} />{t("settings.modeChanged")}</label>
      <label class="check"><input type="checkbox" bind:checked={notify.writeFailed} onchange={save} />{t("settings.writeFailed")}</label>
    </div>
  </section>

  <section class="card" aria-label={t("settings.polling")}>
    <h3>{t("settings.polling")}</h3>
    <label class="field">{t("settings.cycle")}<input type="number" min="30" max="600" step="1" placeholder={t("common.auto")} value={app.settings.pollSeconds ?? ""} onchange={savePoll} disabled={app.settings.autoTune} /></label>
    <p class="note">{app.settings.autoTune ? t("settings.autoTuneNote", { s: app.settings.tunedPoll ?? app.snapshot?.pollSecs ?? 240 }) : pollNote}</p>
    <label class="check"><input type="checkbox" checked={app.settings.autoTune} onchange={toggleTune} />{t("settings.autoTune")}</label>
    <div class="hint"><Hint id="poll" /></div>
  </section>

  <section class="card" aria-label={t("settings.startup")}>
    <h3>{t("settings.startup")}</h3>
    <label class="check"><input type="checkbox" bind:checked={autostart} onchange={toggle} />{t("settings.autostart")}</label>
    <div class="hint"><Hint id="autostart" /></div>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </section>
</div>

<style>
  .list { display: grid; gap: 12px; }
  .intro { margin-bottom: 12px; }
  .item { display: flex; align-items: end; gap: 16px; }
  .check { display: flex; align-items: center; gap: 8px; min-height: 34px; }
  .hint { margin-top: 8px; }
  .note { margin-top: 8px; font-size: var(--text-sm); }
  .err { margin-top: 8px; color: var(--danger); }
</style>
