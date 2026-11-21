<script lang="ts">
  import type { BlockKind, SlotTest } from "../lib/api";
  import { consequence } from "../lib/glossary";
  import { t } from "../lib/i18n.svelte";
  import { isManual, kindName, parseSlot, slotTime, type Group } from "../lib/plan";
  import Days from "./Days.svelte";
  import Hint from "./Hint.svelte";

  let {
    groups,
    day,
    cap,
    verdict,
    onchange,
    onduplicate,
    ondelete,
  }: {
    groups: Group[];
    day: number;
    cap: number;
    verdict: SlotTest["verdict"] | undefined;
    onchange: (next: Group[]) => void;
    onduplicate: () => void;
    ondelete: () => void;
  } = $props();

  const KINDS: BlockKind[] = ["charge", "discharge", "auto", "ai", "ups"];

  const single = $derived(groups.length === 1 ? groups[0] : undefined);
  const common = <T,>(f: (g: Group) => T) => (groups.every((g) => f(g) === f(groups[0])) ? f(groups[0]) : undefined);
  const kind = $derived(common((g) => g.cell.kind));
  const manual = $derived(groups.every((g) => isManual(g.cell.kind)));
  const power = $derived(common((g) => g.cell.power));
  const limit = $derived(common((g) => g.cell.socLimit));
  const days = $derived(groups.reduce((m, g) => m & g.days, 127));
  const limitLabel = $derived(t(kind === "charge" ? "insp.limitCharge" : kind === "discharge" ? "insp.limitDischarge" : "insp.limitMixed"));
  const effects = $derived([...new Set(groups.map((g) => consequence(g.cell.kind, g.cell.socLimit, verdict)))]);

  const edit = (f: (g: Group) => Group) => onchange(groups.map(f));
  const clampW = (w: number) => Math.max(0, Math.min(cap, Math.round(w)));

  function setKind(k: BlockKind) {
    edit((g) => ({ ...g, cell: isManual(k) ? { kind: k, power: clampW(g.cell.power || Math.min(800, cap)), ...(g.cell.socLimit != null && { socLimit: g.cell.socLimit }) } : { kind: k, power: 0 } }));
  }
  function setPower(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).valueAsNumber;
    if (!Number.isNaN(v)) edit((g) => ({ ...g, cell: { ...g.cell, power: clampW(v) } }));
  }
  function setLimit(e: Event) {
    const v = (e.currentTarget as HTMLInputElement).valueAsNumber;
    edit((g) => {
      const { socLimit: _, ...cell } = g.cell;
      return { ...g, cell: Number.isNaN(v) ? cell : { ...cell, socLimit: Math.max(0, Math.min(100, Math.round(v))) } };
    });
  }
  function setTime(e: Event, edge: "a" | "b") {
    const input = e.currentTarget as HTMLInputElement;
    const i = parseSlot(input.value);
    const g = single!;
    const next = { ...g, [edge]: i };
    if (i == null || next.a >= next.b) {
      input.value = slotTime(g[edge]);
      return;
    }
    onchange([next]);
  }
  function setDays(v: number) {
    const added = v & ~days;
    const removed = days & ~v;
    edit((g) => ({ ...g, days: ((g.days | added) & ~removed) | (1 << day) }));
  }
</script>

<aside class="insp" aria-label={t("insp.aria")}>
  {#if !groups.length}
    <h3>{t("insp.none")}</h3>
    <p class="muted">{t("insp.noneHint")}</p>
  {:else}
    <h3>{single ? t("insp.block", { range: t("confirm.range", { start: slotTime(single.a), end: slotTime(single.b) }) }) : t("tl.selMany", { n: groups.length })}</h3>

    {#if single}
      {#key `${single.a}-${single.b}`}
        <div class="two">
          <label class="field">{t("tariff.start")}<input class="num" value={slotTime(single.a)} inputmode="numeric" pattern={"[0-9]{1,2}:[0-9]{2}"} aria-describedby="insp-step" onchange={(e) => setTime(e, "a")} /></label>
          <label class="field">{t("tariff.end")}<input class="num" value={slotTime(single.b)} inputmode="numeric" pattern={"[0-9]{1,2}:[0-9]{2}"} aria-describedby="insp-step" onchange={(e) => setTime(e, "b")} /></label>
        </div>
        <p id="insp-step" class="muted small">{t("insp.timeHint")}</p>
      {/key}
    {/if}

    <div class="field">
      {t("insp.type")}
      <div class="seg kinds" role="group" aria-label={t("insp.typeAria")}>
        {#each KINDS as k}
          <button type="button" class="k-{k}" aria-pressed={kind === k} onclick={() => setKind(k)}>{kindName(k)}</button>
        {/each}
      </div>
    </div>

    {#key `${power}-${groups.length}`}
      <div class="field">
        <span>{t("insp.power")} <span class="num">{cap} W</span></span>
        <div class="power">
          <input type="range" min="0" max={cap} step="50" value={power ?? 0} disabled={!manual} aria-label={t("common.power")} onchange={setPower} />
          <input type="number" class="num" min="0" max={cap} step="50" value={power ?? ""} placeholder={t("insp.mixed")} disabled={!manual} aria-label={t("insp.powerWatts")} onchange={setPower} />
        </div>
      </div>
      <label class="field">{limitLabel}
        <input type="number" class="num" min="0" max="100" step="1" value={limit ?? ""} placeholder={t("insp.none2")} disabled={!manual} onchange={setLimit} />
      </label>
    {/key}
    {#if manual}<Hint id="socLimit" />{/if}

    <div class="field">{t("insp.days")}<Days bind:days={() => days, setDays} label={t("insp.days")} /></div>

    <ul class="effects">
      {#each effects as e}<li>{e}</li>{/each}
    </ul>

    <div class="row">
      <button type="button" class="btn" onclick={onduplicate}>{t("common.duplicate")}</button>
      <button type="button" class="btn danger" onclick={ondelete}>{t("common.delete")}</button>
    </div>
  {/if}
</aside>

<style>
  .insp { display: grid; gap: 12px; align-content: start; padding: 16px; border-left: 1px solid var(--border); }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .two input { min-width: 0; }
  .small { font-size: var(--text-xs); margin-top: -8px; }
  .kinds { flex-wrap: wrap; }
  .kinds button { padding: 0 8px; }
  .power { display: grid; grid-template-columns: minmax(0, 1fr) 88px; gap: 8px; align-items: center; }
  .effects { margin: 0; padding: 12px 12px; list-style: none; display: grid; gap: 8px; background: var(--surface-2); border-radius: var(--radius-sm); font-size: var(--text-sm); }
</style>
