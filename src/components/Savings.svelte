<script lang="ts">
  import { historySavings, type Savings } from "../lib/api";
  import { app, errText, setting } from "../lib/state.svelte";
  import { locale, t } from "../lib/i18n.svelte";
  import PriceEditor from "./PriceEditor.svelte";

  let { bounds }: { bounds?: readonly [number, number] } = $props();

  let result = $state<Savings>();
  let error = $state("");
  let editing = $state(false);
  let seq = 0;

  const price = $derived(app.settings.price);
  const cur = $derived(app.settings.currency);
  const money = (v: number) => `${v.toLocaleString(locale(), { minimumFractionDigits: 2, maximumFractionDigits: 2 })} ${cur}`;
  const kwh = (v: number) => `${v.toLocaleString(locale(), { maximumFractionDigits: 1 })} kWh`;

  $effect(() => {
    const b = bounds;
    const p = price; // dépendance : recalcule quand le tarif change
    if (!b) return;
    const id = ++seq;
    error = "";
    historySavings(b[0], b[1], p)
      .then((r) => id === seq && (result = r))
      .catch((e) => id === seq && ((error = errText(e)), (result = undefined)));
  });

</script>

<section class="card" aria-label={t("savings.title")}>
  <div class="head">
    <h3>{t("savings.title")}</h3>
    <button class="btn quiet" onclick={() => (editing = !editing)}>{editing ? t("common.done") : t("savings.editTariff")}</button>
  </div>

  {#if editing}<PriceEditor />{/if}

  {#if error}
    <p class="err" role="alert">{error}</p>
  {:else if result}
    {@const loss = result.saved < 0}
    <p class="saved" class:loss>{money(Math.abs(result.saved))}<span class="muted"> {loss ? t("savings.extraOverPeriod") : t("savings.savedOverPeriod")}</span></p>
    <dl class="grid">
      <dt>{t("savings.baseline")}</dt><dd class="num">{money(result.baseline)}</dd>
      <dt>{t("savings.cost")}</dt><dd class="num">{money(result.cost)}</dd>
      <dt>{t("savings.selfUsed")}</dt><dd class="num">{kwh(result.selfUsed)}</dd>
      <dt>{t("savings.imported")}</dt><dd class="num">{kwh(result.imported)}</dd>
      <dt>{t("savings.exported")}</dt><dd class="num">{kwh(result.exported)}</dd>
    </dl>
    <p class="muted note">{t("savings.explain")}</p>
  {/if}
</section>

<style>
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .head h3 { margin: 0; }
  .note { font-size: var(--text-sm); }
  .saved { margin: 14px 0 8px; font-size: 1.6rem; font-weight: 600; color: var(--ok, var(--accent)); font-variant-numeric: tabular-nums; }
  .saved .muted { font-size: 0.9rem; font-weight: 400; }
  .saved.loss { color: var(--danger); }
  .grid { display: grid; grid-template-columns: auto auto; gap: 4px 16px; max-width: 320px; }
  .grid dd { text-align: right; }
  .err { color: var(--danger); }
</style>
