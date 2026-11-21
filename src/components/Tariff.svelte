<script lang="ts">
  import { tariffPlan, type Block } from "../lib/api";
  import { app, errText, powerMax, setting } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";
  import { clone } from "../lib/store";
  import Days from "./Days.svelte";
  import Hint from "./Hint.svelte";

  let { onfill }: { onfill: (blocks: Block[]) => void } = $props();

  let tariff = $state(clone(app.settings.tariff));
  let error = $state("");
  const cap = $derived(powerMax());

  $effect(() => {
    const j = JSON.stringify(tariff);
    if (j === JSON.stringify(app.settings.tariff)) return;
    error = "";
    setting("tariff", JSON.parse(j));
  });

  async function generate() {
    error = "";
    try {
      onfill(await tariffPlan({ ...clone(tariff), chargePower: Math.min(cap, tariff.chargePower), dischargePower: Math.min(cap, tariff.dischargePower) }));
    } catch (e) {
      error = errText(e);
    }
  }
</script>

<div class="tariff">
  <div class="intro"><Hint id="tariff" /></div>

  <ul class="ranges">
    {#each tariff.offpeak as r, i}
      <li>
        <label class="field">{t("tariff.start")}<input type="time" bind:value={r.start} /></label>
        <label class="field">{t("tariff.end")}<input type="time" bind:value={r.end} /></label>
        <button class="btn danger" onclick={() => tariff.offpeak.splice(i, 1)} aria-label={t("tariff.removeRange", { n: i + 1 })}>{t("common.delete")}</button>
      </li>
    {:else}
      <li class="muted">{t("tariff.noRange")}</li>
    {/each}
  </ul>
  <button class="btn" onclick={() => tariff.offpeak.push({ start: "22:00", end: "06:00" })}>{t("tariff.addRange")}</button>

  <div class="row opts">
    <label class="field">{t("tariff.chargePower", { cap })}<input type="number" class="num" min="0" max={cap} step="50" bind:value={tariff.chargePower} /></label>
    <label class="field">{t("tariff.chargeSoc")}<input type="number" min="1" max="100" step="1" bind:value={tariff.chargeSoc} /></label>
    <label class="field">{t("tariff.peak")}
      <select bind:value={tariff.peak}>
        <option value="auto">Auto</option>
        <option value="discharge">{t("dir.discharge")}</option>
      </select>
    </label>
    {#if tariff.peak === "discharge"}
      <label class="field">{t("tariff.dischargePower", { cap })}<input type="number" class="num" min="0" max={cap} step="50" bind:value={tariff.dischargePower} /></label>
    {/if}
    <div class="field">{t("common.days")}<Days bind:days={tariff.days} /></div>
  </div>

  <div class="row actions">
    <button class="btn" onclick={generate} disabled={!tariff.offpeak.length}>{t("tariff.fill")}</button>
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>

<style>
  .intro { margin-bottom: 16px; }
  .ranges { list-style: none; margin: 0 0 12px; padding: 0; display: grid; gap: 8px; }
  .ranges li { display: flex; align-items: end; gap: 12px; }
  .opts, .actions { margin-top: 16px; }
  .err { color: var(--danger); margin-top: 12px; }
</style>
