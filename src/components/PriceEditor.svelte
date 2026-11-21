<script lang="ts">
  import type { Price } from "../lib/api";
  import { app, setting } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";

  const price = $derived(app.settings.price);
  const cur = $derived(app.settings.currency);

  const update = (p: Price) => setting("price", p);
  const addSlot = () => update({ ...price, slots: [...price.slots, { start: "08:00", end: "20:00", price: price.default }] });
  const removeSlot = (i: number) => update({ ...price, slots: price.slots.filter((_, j) => j !== i) });
  function setSlot(i: number, field: "start" | "end" | "price", v: string) {
    const slots = price.slots.map((s, j) => (j === i ? { ...s, [field]: field === "price" ? Number(v) || 0 : v } : s));
    update({ ...price, slots });
  }
  const num = (v: string) => Number(v) || 0;
</script>

<div class="editor">
  <p class="muted note">{t("savings.editHint")}</p>
  <div class="slots">
    {#each price.slots as s, i}
      <div class="slot">
        <input type="time" value={s.start} aria-label={t("savings.from")} onchange={(e) => setSlot(i, "start", e.currentTarget.value)} />
        <span>→</span>
        <input type="time" value={s.end} aria-label={t("savings.to")} onchange={(e) => setSlot(i, "end", e.currentTarget.value)} />
        <input class="price" type="number" min="0" step="0.01" value={s.price} aria-label={t("savings.price")} onchange={(e) => setSlot(i, "price", e.currentTarget.value)} />
        <span class="cur">{cur}/kWh</span>
        <button class="btn quiet del" aria-label={t("savings.removeSlot")} onclick={() => removeSlot(i)}>×</button>
      </div>
    {/each}
  </div>
  <button class="btn" onclick={addSlot}>{t("savings.addSlot")}</button>
  <div class="row fields">
    <label class="field">{t("savings.default")}<input type="number" min="0" step="0.01" value={price.default} onchange={(e) => update({ ...price, default: num(e.currentTarget.value) })} /></label>
    <label class="field">{t("savings.feedIn")}<input type="number" min="0" step="0.01" placeholder={t("savings.noResale")} value={price.feedIn ?? ""} onchange={(e) => update({ ...price, feedIn: e.currentTarget.value === "" ? null : num(e.currentTarget.value) })} /></label>
    <label class="field">{t("savings.currency")}<input type="text" maxlength="3" value={cur} onchange={(e) => setting("currency", e.currentTarget.value.trim() || "€")} /></label>
  </div>
</div>

<style>
  .editor { margin: 12px 0 4px; display: grid; gap: 12px; }
  .note { font-size: var(--text-sm); }
  .slots { display: grid; gap: 8px; }
  .slot { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .slot input { min-height: 34px; padding: 0 10px; background: var(--bg); border: 1px solid var(--border); border-radius: var(--radius-sm); }
  .slot .price { max-width: 90px; }
  .slot .cur { color: var(--muted); font-size: var(--text-sm); }
  .del { padding: 2px 10px; }
  .fields { display: flex; gap: 16px; flex-wrap: wrap; }
</style>
