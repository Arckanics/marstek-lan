<script lang="ts">
  import { runSlotTest } from "../lib/api";
  import { app, errText, log } from "../lib/state.svelte";
  import { locale, t } from "../lib/i18n.svelte";
  import Hint from "./Hint.svelte";

  const EXPECTED = 60;
  const running = $derived(app.slotTest != null && app.slotTest.verdict === null);
  const off = $derived(!app.connectedIp || app.busy || running);
  const saved = $derived(app.settings.outsideSlots);
  const verdict = $derived(app.slotTest?.verdict ?? (running ? null : saved?.verdict));
  const last = $derived(app.slotTest?.samples.at(-1)?.batPower);

  const start = () =>
    runSlotTest().catch((e) => {
      app.slotTest = undefined;
      log(t("slotTest.error", { error: errText(e) }));
    });
</script>

<div class="slottest">
  <div class="intro"><Hint id="slotTest" /></div>
  <button class="btn" disabled={off} onclick={start}>{running ? t("slotTest.running") : t("slotTest.start")}</button>

  {#if running && app.slotTest}
    <div class="prog" role="status">
      <progress max={EXPECTED} value={app.slotTest.samples.length} aria-label={t("slotTest.progress")}></progress>
      <span class="num muted">{t("slotTest.samples", { n: app.slotTest.samples.length })}{last != null ? t("slotTest.last", { w: Math.round(last) }) : ""}</span>
    </div>
  {/if}

  {#if verdict}
    <p class="verdict" role="status">
      {t(verdict === "idle" ? "slotTest.idle" : "slotTest.follows")}
      {#if !app.slotTest?.verdict && saved?.ts}<span class="muted">{t("slotTest.savedAt", { date: new Date(saved.ts).toLocaleString(locale()) })}</span>{/if}
    </p>
  {/if}
</div>

<style>
  .intro { margin-bottom: 12px; }
  .prog { display: flex; align-items: center; gap: 12px; margin-top: 12px; }
  progress { flex: 1; accent-color: var(--auto); }
  .verdict { margin-top: 12px; padding: 8px 12px; border-left: 2px solid var(--auto); background: var(--surface-2); max-width: 80ch; }
</style>
