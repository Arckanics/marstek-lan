<script lang="ts">
  import { applyPlan, compilePlan, type Block, type BlockKind } from "../lib/api";
  import { app, clock, errText, powerMax, setting, write } from "../lib/state.svelte";
  import { clone } from "../lib/store";
  import { dayName, dayNameLower, locale, t } from "../lib/i18n.svelte";
  import { limits, slotCount } from "../lib/limits";
  import { ALL_DAYS, STEPS, WEEKDAYS, WEEKEND, diffRuns, fromCells, groupAt, isManual, kindName, rewrite, tariffBands, toCells, type Cell, type Group } from "../lib/plan";
  import BlockInspector from "./BlockInspector.svelte";
  import Confirm from "./Confirm.svelte";
  import DayTimeline from "./DayTimeline.svelte";
  import Days from "./Days.svelte";
  import Hint from "./Hint.svelte";
  import PriceEditor from "./PriceEditor.svelte";
  import Icon from "./Icon.svelte";
  import SlotTest from "./SlotTest.svelte";
  import Tariff from "./Tariff.svelte";
  import WeekOverview from "./WeekOverview.svelte";

  let confirm: Confirm;

  const TOOLS: (BlockKind | "erase")[] = ["charge", "discharge", "auto", "ai", "ups", "erase"];
  const toolName = (k: BlockKind | "erase") => (k === "erase" ? t("plan.erase") : kindName(k));
  const DAYS = [0, 1, 2, 3, 4, 5, 6];
  const today = () => (new Date(clock.now).getDay() + 6) % 7;

  let modelIdx = $state(0);
  const plans = $derived(app.settings.plans);
  const plan = $derived(plans[Math.min(modelIdx, plans.length - 1)]);
  const off = $derived(!app.connectedIp || app.busy);
  const max = $derived(slotCount(limits(app.snapshot)));
  const cap = $derived(powerMax());

  const savedTargets = app.settings.planTargets;
  const savedDay = app.settings.planDay;
  const initTargets = savedTargets && savedTargets <= ALL_DAYS ? savedTargets : 1 << (savedDay ?? today());
  let targets = $state(initTargets);
  let open = $state(savedDay != null && initTargets & (1 << savedDay) ? savedDay : DAYS.findIndex((d) => initTargets & (1 << d)));
  let picked = $state<{ a: number; b: number }[]>([]);

  let tool = $state<BlockKind | "erase">("charge");
  let power = $state(800);
  let limit = $state<number | null>(null);

  let count = $state<number>();
  let error = $state("");
  let editTariff = $state(false);

  const cells = $derived(plan ? toCells(plan.blocks) : []);
  const groups = $derived(picked.map((r) => groupAt(cells, open, r.a, r.b)).filter((g): g is Group => g != null));
  const manualTool = $derived(tool !== "erase" && isManual(tool));
  const watts = $derived(Math.max(0, Math.min(cap, Math.round(power || 0))));
  const paint: Cell = $derived(
    tool === "erase" ? null : manualTool ? { kind: tool, power: watts, ...(limit != null && { socLimit: Math.round(limit) }) } : { kind: tool, power: 0 },
  );
  const verdict = $derived(app.settings.outsideSlots?.verdict);
  const piloted = $derived(verdict !== "follows");
  const hasModes = $derived(!!plan?.blocks.some((b) => !isManual(b.kind)));
  const hasLimits = $derived(!!plan?.blocks.some((b) => b.socLimit != null));
  const needsApp = $derived((hasModes && piloted) || hasLimits);
  const nowSlot = $derived(open === today() ? ((clock.now - new Date(clock.now).setHours(0, 0, 0, 0)) / 900_000) : null);

  // Fond heures creuses/pleines : bandes de prix du tarif derrière la frise, pour y caler les blocs.
  const tariff = $derived(tariffBands(app.settings.price));
  const cur = $derived(app.settings.currency);
  const money = (v: number) => `${v.toLocaleString(locale(), { maximumFractionDigits: 2 })} ${cur}`;
  const tierPrices = $derived([...new Set(tariff.bands.map((b) => b.price))].sort((a, b) => a - b));

  const sent = $derived(app.settings.sentPlan);
  const diff = $derived(plan && sent ? diffRuns(plan.blocks, sent.blocks) : null);
  const sync = $derived(app.snapshot?.sync);
  const bar = $derived(sync?.state === "bad" ? "bad" : diff === 0 ? "ok" : "dirty");

  const save = (next: typeof plans) => setting("plans", next);
  const setBlocks = (blocks: Block[]) => save(plans.map((p) => (p === plan ? { ...p, blocks } : p)));
  const commit = (next: Cell[][]) => setBlocks(fromCells(next));
  const uniqueName = (base: string) => {
    let n = base;
    for (let i = 2; plans.some((p) => p.name === n); i++) n = `${base} ${i}`;
    return n;
  };
  const firstDay = (mask: number) => DAYS.findIndex((d) => mask & (1 << d));
  const single = $derived((targets & (targets - 1)) === 0);
  // Un seul jour coché : on peut ouvrir n'importe lequel ; sinon seulement ceux du groupe.
  const list = $derived(DAYS.filter((d) => single || targets & (1 << d)));
  const title = $derived(
    targets === ALL_DAYS ? t("common.allDays") : targets === WEEKDAYS ? `${dayName(0)} → ${dayName(4)}` : targets === WEEKEND ? t("plan.weekend") : single ? dayName(open) : list.map((d) => dayName(d).slice(0, 3)).join(" "),
  );
  const spread = (gs: Group[]) => gs.map((g) => ({ ...g, days: g.days | targets }));
  const identical = $derived(cells.every((c) => JSON.stringify(c) === JSON.stringify(cells[open])));

  function openDay(d: number) {
    open = d;
    if (!(targets & (1 << d))) targets = 1 << d;
    picked = [];
    setting("planDay", open);
    setting("planTargets", targets);
  }
  const stepDay = (s: number) => openDay(list[(list.indexOf(open) + s + list.length) % list.length]);
  function setTargets(mask: number) {
    if (!mask) return;
    targets = mask;
    if (!(mask & (1 << open))) open = firstDay(mask);
    picked = [];
    setting("planDay", open);
    setting("planTargets", targets);
  }
  function pickModel(e: Event) {
    modelIdx = Number((e.currentTarget as HTMLSelectElement).value);
    picked = [];
  }

  function paintRange(a: number, b: number, value: Cell) {
    const next = cells.map((c) => [...c]);
    for (let d = 0; d < 7; d++) if (targets & (1 << d) || d === open) for (let i = a; i <= b; i++) next[d][i] = value;
    commit(next);
    picked = [];
  }
  function select(r: { a: number; b: number } | null, additive: boolean) {
    if (!r) picked = [];
    else if (!additive) picked = [r];
    else picked = picked.some((x) => x.a === r.a) ? picked.filter((x) => x.a !== r.a) : [...picked, r];
  }
  function edit(next: Group[]) {
    // Une modification touche tous les jours choisis, pas seulement ceux où le bloc est déjà identique ; un changement de jours demandé dans l'inspecteur est respecté.
    const to = next.map((n, i) => (n.days === groups[i].days ? { ...n, days: n.days | targets } : n));
    commit(rewrite(cells, spread(groups), to));
    picked = next.filter((g) => g.days & (1 << open)).map(({ a, b }) => ({ a, b }));
  }
  function moveBy(d: number) {
    const dd = d > 0 ? Math.min(d, ...groups.map((g) => STEPS - g.b)) : Math.max(d, ...groups.map((g) => -g.a));
    if (dd) edit(groups.map((g) => ({ ...g, a: g.a + dd, b: g.b + dd })));
  }
  const resizeBy = (d: number) => edit(groups.map((g) => ({ ...g, b: Math.max(g.a + 1, Math.min(STEPS, g.b + d)) })));
  function removePicked() {
    commit(rewrite(cells, spread(groups), []));
    picked = [];
  }
  function duplicate() {
    const copies = spread(groups).flatMap((g) => {
      const len = g.b - g.a;
      return g.b + len <= STEPS ? [{ ...g, a: g.b, b: g.b + len }] : g.a - len >= 0 ? [{ ...g, a: g.a - len, b: g.a }] : [];
    });
    commit(rewrite(cells, [], copies));
    picked = copies.map(({ a, b }) => ({ a, b }));
  }
  function copyDay() {
    commit(cells.map((col, d) => (d !== open && targets & (1 << d) ? [...cells[open]] : col)));
  }

  function add(blocks: Block[] = []) {
    const n = plans.length;
    save([...clone(plans), { name: uniqueName(t("plan.newTemplate")), blocks }]);
    modelIdx = n;
  }
  function duplicateModel() {
    const n = plans.length;
    save([...clone(plans), { name: uniqueName(t("plan.copyName", { name: plan.name })), blocks: clone(plan.blocks) }]);
    modelIdx = n;
  }
  function rename(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const name = input.value.trim();
    if (!name || plans.some((p) => p !== plan && p.name === name)) {
      input.value = plan.name;
      return;
    }
    save(plans.map((p) => (p === plan ? { ...p, name } : p)));
  }
  async function removeModel() {
    if (!(await confirm.ask(t("plan.removeAsk", { name: plan.name }), { title: t("plan.removeTitle"), ok: t("common.delete") }))) return;
    save(plans.filter((p) => p !== plan));
    modelIdx = 0;
  }

  $effect(() => {
    if (!plan) return;
    const p = clone(plan);
    const timer = setTimeout(async () => {
      try {
        count = (await compilePlan(p)).length;
        error = "";
      } catch (e) {
        count = undefined;
        error = errText(e);
      }
    }, 250);
    return () => clearTimeout(timer);
  });

  async function send() {
    if (!plan || off || error || count == null) return;
    let slots;
    try {
      slots = await compilePlan(clone(plan));
    } catch (e) {
      error = errText(e);
      return;
    }
    const ok = await confirm.ask(t("plan.sendAsk", { name: plan.name, n: slots.length, max }), {
      title: t("plan.apply"),
      ok: t("plan.apply"),
      slots,
      note: needsApp ? t("plan.sendNote") : "",
    });
    if (!ok) return;
    const r = await write(t("plan.sendLog", { name: plan.name }), () => applyPlan(clone(plan)), (r) => r.every(Boolean));
    if (r?.every(Boolean)) app.settings.sentPlan = clone(plan);
  }

  async function revert() {
    if (!sent || !diff) return;
    if (!(await confirm.ask(t("plan.revertAsk", { n: diff, name: sent.name }), { title: t("plan.undo"), ok: t("plan.undo") }))) return;
    setBlocks(clone(sent.blocks));
    picked = [];
  }

  function keys(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      send();
    }
  }
  function toolKey(e: KeyboardEvent) {
    if (e.ctrlKey || e.altKey || e.metaKey || (e.target as HTMLElement).closest("input, select, textarea")) return;
    const i = Number(e.key) - 1;
    if (i >= 0 && i < TOOLS.length) tool = TOOLS[i];
  }
</script>

<svelte:window onkeydown={keys} />

<div class="planning">
  {#if !plan}
    <div class="empty">
      <strong>{t("plan.emptyTitle")}</strong>
      <p>{t("plan.emptyHint")}</p>
      <button class="btn primary" onclick={() => add()}>{t("plan.create")}</button>
    </div>
  {:else}
    <div class="top">
      <label class="field model">{t("plan.template")}
        <select value={Math.min(modelIdx, plans.length - 1)} onchange={pickModel}>
          {#each plans as p, i}<option value={i}>{p.name}</option>{/each}
        </select>
      </label>
      <div class="apply">
        <span class="lab">{t("plan.applyTo")}</span>
        <div class="seg" role="group" aria-label={t("plan.targetsAria")}>
          <button aria-pressed={targets === 1 << open} onclick={() => setTargets(1 << open)}>{dayName(open)}</button>
          <button aria-pressed={targets === WEEKDAYS} onclick={() => setTargets(WEEKDAYS)}>{t("plan.monFri")}</button>
          <button aria-pressed={targets === WEEKEND} onclick={() => setTargets(WEEKEND)}>{t("soc.weekend")}</button>
          <button aria-pressed={targets === ALL_DAYS} onclick={() => setTargets(ALL_DAYS)}>{t("common.allDays")}</button>
        </div>
        {#if targets !== ALL_DAYS}
          <Days bind:days={() => targets, setTargets} label={t("plan.chosenDays")} />
          <button class="btn quiet" disabled={!(targets & ~(1 << open))} onclick={copyDay}>{t("plan.copyDay", { day: dayNameLower(open) })}</button>
        {/if}
      </div>
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <section class="editor" aria-label={t("plan.editor")} onkeydown={toolKey}>
      <div class="ed-main">
        <div class="ed-head">
          <div class="daynav">
            {#if targets !== ALL_DAYS}
              <button class="btn quiet" aria-label={t("plan.prevDay")} disabled={list.length < 2} onclick={() => stepDay(-1)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 6-6 6 6 6" /></svg></button>
              <label class="field day">
                <span class:visually-hidden={single}>{t("plan.dayShown")}</span>
                <select value={open} onchange={(e) => openDay(Number(e.currentTarget.value))}>
                  {#each list as d}<option value={d}>{dayName(d)}</option>{/each}
                </select>
              </label>
              <button class="btn quiet" aria-label={t("plan.nextDay")} disabled={list.length < 2} onclick={() => stepDay(1)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m9 6 6 6-6 6" /></svg></button>
            {/if}
            {#if !single}<h2>{title}</h2>{/if}
          </div>
          <div class="tools">
            <div class="seg" role="group" aria-label={t("plan.tool")}>
              {#each TOOLS as k, i}
                <button class={k === "erase" ? "" : `k-${k}`} aria-pressed={tool === k} aria-keyshortcuts={String(i + 1)} title={t("plan.shortcut", { n: i + 1 })} onclick={() => (tool = k)}>
                  <Icon name={k} size={15} />{toolName(k)}
                </button>
              {/each}
            </div>
            <label class="field inline">W
              <input type="number" class="num" min="0" max={cap} step="50" bind:value={power} disabled={!manualTool} aria-label={t("plan.toolPower", { cap })} />
            </label>
            <label class="field inline">{t(tool === "discharge" ? "plan.floorPct" : "plan.targetPct")}
              <input type="number" class="num" min="0" max="100" step="1" placeholder={t("plan.noLimit")} bind:value={limit} disabled={!manualTool} aria-label={t("plan.toolLimit")} />
            </label>
          </div>
        </div>
        <DayTimeline
          cells={cells[open]}
          {paint}
          {piloted}
          day={single ? dayNameLower(open) : title}
          now={nowSlot}
          selected={picked}
          bands={tariff.bands}
          tiers={tariff.tiers}
          onpaint={paintRange}
          onselect={select}
          onmove={moveBy}
          onresize={resizeBy}
          ondelete={removePicked} />
        <div class="tariff-foot">
          {#if tariff.tiers > 1}
            <p class="tariff-leg">
              <span class="lab">{t("plan.tariffLegend")}</span>
              {#each tierPrices as p, k}
                <span class="tl-item"><span class="tl-sw" style:--r={(tariff.tiers > 1 ? k / (tariff.tiers - 1) : 0).toFixed(3)}></span>{money(p)}/kWh</span>
              {/each}
              <span class="muted hint">{t("plan.tariffHint")}</span>
            </p>
          {/if}
          <button class="btn quiet edit-tariff" onclick={() => (editTariff = !editTariff)}>{editTariff ? t("common.done") : t("savings.editTariff")}</button>
        </div>
        {#if editTariff}<PriceEditor />{/if}
      </div>
      <BlockInspector {groups} day={open} {cap} {verdict} onchange={edit} onduplicate={duplicate} ondelete={removePicked} />
    </section>

    <section class="week" aria-label={t("history.week")}>
      <div class="week-head">
        <h3>{targets === ALL_DAYS && identical ? t("plan.legend") : t("history.week")}</h3>
        <p class="legend">
          {#each TOOLS.slice(0, 5) as k}<span class="k-{k}"><Icon name={k} size={14} />{toolName(k)}</span>{/each}
          <span><span class="swatch hatch"></span>{t("plan.pcOn")}</span>
        </p>
      </div>
      {#if targets === ALL_DAYS && identical}
        <p class="pc-note">{t("plan.identical")}</p>
      {:else}
        <WeekOverview {cells} {open} {targets} {piloted} onopen={openDay} />
      {/if}
      {#if needsApp}
        <p class="pc-note">
          {t("plan.hatched", { parts: [hasModes && piloted && t("plan.needsModes"), hasLimits && t("plan.needsLimits")].filter(Boolean).join(t("plan.needsAnd")) })}
        </p>
      {/if}
    </section>

    <div class="secondary">
      <details class="more">
        <summary>{t("plan.templates")} <span class="muted">{t("plan.templatesHint")}</span></summary>
        <div class="row">
          {#key plan.name}
            <label class="field">{t("common.name")}<input type="text" value={plan.name} onchange={rename} /></label>
          {/key}
          <button class="btn" onclick={() => add()}>{t("common.new")}</button>
          <button class="btn" onclick={duplicateModel}>{t("common.duplicate")}</button>
          <button class="btn danger" onclick={removeModel}>{t("common.delete")}</button>
        </div>
      </details>
      <details class="more">
        <summary>{t("plan.tariffGen")} <span class="muted">{t("plan.tariffGenHint")}</span></summary>
        <Tariff onfill={setBlocks} />
      </details>
      <details class="more">
        <summary>{t("plan.slotTest")} <span class="muted">{t(verdict === "idle" ? "plan.slotTestIdle" : verdict === "follows" ? "plan.slotTestFollows" : "plan.slotTestNever")}</span></summary>
        <SlotTest />
      </details>
    </div>

    <div class="sendbar k-{bar === 'bad' ? 'bad' : bar === 'dirty' ? 'warn' : 'none'}" role="region" aria-label={t("plan.sendAria")}>
      <div class="state" aria-live="polite">
        <strong>{t(bar === "bad" ? "plan.stateBad" : bar === "ok" ? "plan.stateOk" : "plan.stateDirty")}</strong>
        <span class="sub">
          {#if bar === "bad" && sync?.reason}{sync.reason}{:else if diff == null}{t("plan.neverSent")}{:else if diff}{t(diff > 1 ? "plan.diffMany" : "plan.diffOne", { n: diff })}{:else}{t("plan.sentName", { name: sent?.name ?? "" })}{/if}
        </span>
      </div>
      <p class="count num" class:err={error}>
        {#if error}<span role="alert">{error}</span>{:else}{t("plan.slotsCount", { n: count ?? "…", max })}{/if}
      </p>
      <div class="actions">
        <button class="btn quiet" disabled={!sent || !diff} onclick={revert}>{t("plan.undo")}</button>
        <button class="btn primary" class:hot={bar !== "ok"} disabled={off || !!error || count == null} onclick={send} aria-keyshortcuts="Control+S">
          {t("plan.apply")} <kbd>Ctrl S</kbd>
        </button>
      </div>
    </div>
  {/if}
</div>

<Confirm bind:this={confirm} />

<style>
  .planning { display: grid; gap: 16px; }
  .top { display: flex; flex-wrap: wrap; align-items: end; gap: 12px 24px; }
  .model select { min-width: 200px; font-weight: 600; }
  .apply { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .lab { font-size: var(--text-sm); color: var(--muted); }

  .editor {
    display: grid; grid-template-columns: minmax(0, 1fr) 300px;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius);
  }
  .ed-main { padding: 16px; min-width: 0; }
  .ed-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
  .daynav { display: flex; align-items: center; gap: 4px; }
  .daynav h2 { font-size: var(--text-xl); margin-left: 8px; }
  .daynav .btn { min-width: 34px; padding: 0; }
  .day select { min-width: 150px; font-family: var(--font-display); font-size: var(--text-lg); font-weight: 600; }
  .daynav svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .tools { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .tools .seg button { display: inline-flex; align-items: center; gap: 8px; }
  .tools .seg button :global(.icon), .legend :global(.icon) { color: var(--k, var(--muted)); }
  .tools .seg button[aria-pressed="true"] { box-shadow: inset 0 -2px 0 var(--k, var(--text)), inset 0 -14px 12px -10px color-mix(in oklab, var(--k, var(--text)) var(--glow), transparent); }
  .inline { grid-auto-flow: column; align-items: center; gap: 8px; }
  .inline input { width: 82px; }
  .tariff-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-top: 10px; }
  .edit-tariff { margin-left: auto; }
  .tariff-leg { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 14px; font-size: var(--text-sm); color: var(--muted); }
  .tl-item { display: inline-flex; align-items: center; gap: 6px; color: var(--text); font-variant-numeric: tabular-nums; }
  .tl-sw { width: 12px; height: 12px; border-radius: 3px; background: color-mix(in oklab, var(--discharge) calc(var(--r) * 100%), var(--charge)); }
  .tariff-leg .hint { flex-basis: 100%; }

  .week { padding: 16px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
  .week-head { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: baseline; gap: 8px; margin-bottom: 12px; }
  .legend { display: flex; flex-wrap: wrap; gap: 4px 16px; font-size: var(--text-sm); color: var(--muted); }
  .legend > span { display: inline-flex; align-items: center; gap: 8px; }
  .hatch { background: repeating-linear-gradient(135deg, var(--muted) 0 2px, transparent 2px 5px); }
  .pc-note { margin-top: 12px; font-size: var(--text-sm); color: var(--muted); max-width: 90ch; }

  .secondary { display: grid; }

  .sendbar {
    position: sticky; bottom: 0; z-index: 5; margin: 0 -20px -20px;
    display: flex; flex-wrap: wrap; align-items: center; gap: 12px 20px; padding: 12px 20px;
    background: color-mix(in oklab, var(--k) 12%, var(--surface));
    border-top: 2px solid var(--k);
  }
  .state { display: grid; gap: 2px; min-width: 0; flex: 1; }
  .state strong { color: color-mix(in oklab, var(--k) 70%, var(--text)); font-family: var(--font-display); font-size: var(--text-md); }
  .sendbar.k-none .state strong { color: var(--text); }
  .sub { font-size: var(--text-sm); color: var(--muted); }
  .count { font-weight: 600; }
  .count.err { color: var(--danger); }
  .actions { display: flex; gap: 8px; }
  .btn.primary.hot:not(:disabled) { animation: charged 2.4s ease-in-out infinite; }
  @keyframes charged {
    0%, 100% { box-shadow: 0 0 0 1px var(--focus), 0 0 6px color-mix(in oklab, var(--focus) var(--glow), transparent); }
    50% { box-shadow: 0 0 0 1px var(--focus), 0 0 16px 2px color-mix(in oklab, var(--focus) var(--glow), transparent); }
  }
  .primary kbd { border-color: color-mix(in oklab, var(--on-primary) 30%, transparent); opacity: 0.75; }

  @media (max-width: 1100px) {
    .editor { grid-template-columns: minmax(0, 1fr); }
    .editor :global(.insp) { border-left: 0; border-top: 1px solid var(--border); }
  }
</style>
