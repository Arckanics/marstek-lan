<script lang="ts">
  import { setMode, type Direction, type ModeKind, type ModeReq } from "../lib/api";
  import { FICHE_ROWS, fiche as modeFiche, type ModeId } from "../lib/glossary";
  import { t, type Key } from "../lib/i18n.svelte";
  import { app, powerMax, setting, write } from "../lib/state.svelte";
  import { clone } from "../lib/store";
  import Confirm from "./Confirm.svelte";
  import Hint from "./Hint.svelte";

  let confirm: Confirm;
  async function toggleAuto(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    if (input.checked && !(await confirm.ask(t("modes.confirmAuto"), { title: t("modes.confirmAutoTitle"), ok: t("common.activate") }))) {
      input.checked = false;
      return;
    }
    setting("autoControl", input.checked);
  }

  const READ: Record<string, ModeId> = { Auto: "auto", AI: "ai", UPS: "ups", Passive: "passive", Manual: "manual" };
  const ORDER: ModeId[] = ["auto", "ai", "ups", "passive", "manual", "charge", "discharge", "hybrid"];

  const off = $derived(!app.connectedIp || app.busy);
  const cap = $derived(powerMax());
  const currentId = $derived(app.snapshot?.mode?.mode ? READ[app.snapshot.mode.mode] : undefined);
  let shown = $state<ModeId>();
  const id = $derived(shown ?? currentId ?? "auto");
  const fiche = $derived(modeFiche(id));

  let presets = $state(clone(app.settings.presets));
  const savePresets = () => setting("presets", clone(presets));

  let direction = $state<Direction>("charge");
  let power = $state(500);
  let minutes = $state(30);

  let rules = $state(clone(app.settings.rules));
  const saveRules = () => setting("rules", clone(rules));
  const ruleModes: ModeKind[] = ["auto", "ai", "ups"];
  const sources = { plan: "modes.source.plan", rule: "modes.source.rule", manual: "modes.source.manual", hybrid: "fiche.hybrid.name" } as const;
  const overlap = $derived(rules.low.enabled && rules.high.enabled && rules.low.soc >= rules.high.soc);

  const apply = (label: string, req: ModeReq) => write(label, () => setMode(req));
  const passive = (d: Direction, w: number, min: number): ModeReq => ({ kind: "passive", direction: d, power: Math.max(0, Math.min(cap, w)), seconds: Math.max(1, Math.round(min * 60)) });
</script>

<div class="stack">
  {#if !app.connectedIp}
    <p class="notice" role="status">{t("modes.noDevice")}</p>
  {/if}

  <section class="now" aria-label={t("modes.nowAria")}>
    <div>
      <span class="lab">{t("modes.readMode")}</span>
      <strong class="big {currentId ? modeFiche(currentId).tone : 'k-none'}">{currentId ? modeFiche(currentId).name : t("common.unknown")}</strong>
    </div>
    <div role="status">
      <span class="lab">{t("modes.drivenBy")}</span>
      <strong class="big">{app.control ? t(sources[app.control.source]) : t("common.unknown")}</strong>
      {#if app.control?.detail}<span class="muted">{app.control.detail}</span>{/if}
    </div>
  </section>

  <section class="modes" aria-label={t("modes.modesAria")}>
    <ul class="list" aria-label={t("modes.sheets")}>
      {#each ORDER as m}
        <li>
          <button class="item {modeFiche(m).tone}" aria-pressed={id === m} onclick={() => (shown = m)}>
            <span class="swatch"></span>{modeFiche(m).name}
            {#if m === currentId}<span class="cur">{t("modes.current")}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>

    <article class="fiche {fiche.tone}" aria-live="polite">
      <h2>{fiche.name}</h2>
      <dl>
        {#each FICHE_ROWS as [k, label]}<dt>{t(label)}</dt><dd>{fiche[k]}</dd>{/each}
      </dl>
      <div class="act">
        {#if id === "auto" || id === "ai" || id === "ups"}
          <button class="btn tone {fiche.tone}" disabled={off || currentId === id} onclick={() => apply(t("modes.logMode", { name: fiche.name }), { kind: id })}>
            {currentId === id ? t("modes.already", { name: fiche.name }) : t("modes.activate", { name: fiche.name })}
          </button>
        {:else if id === "passive"}
          <div class="row">
            <div class="seg" role="group" aria-label={t("confirm.direction")}>
              <button class="k-charge" aria-pressed={direction === "charge"} onclick={() => (direction = "charge")}>{t("dir.charge")}</button>
              <button class="k-discharge" aria-pressed={direction === "discharge"} onclick={() => (direction = "discharge")}>{t("dir.discharge")}</button>
            </div>
            <label class="field">{t("modes.powerMax", { cap })}<input type="number" class="num" min="0" max={cap} step="50" bind:value={power} /></label>
            <label class="field">{t("modes.duration")}<input type="number" class="num" min="1" step="5" bind:value={minutes} /></label>
            <button class="btn tone k-{direction}" disabled={off} onclick={() => apply(t("modes.logPassive", { dir: t(direction === "charge" ? "kind.charge" : "kind.discharge"), w: Math.min(cap, power), min: minutes }), passive(direction, power, minutes))}>{t("common.apply")}</button>
          </div>
          <Hint id="sign" />
        {:else}
          <button class="btn" onclick={() => (app.tab = "planning")}>{t("modes.openPlan")}</button>
        {/if}
      </div>
    </article>
  </section>

  <section class="card" aria-label={t("modes.presets")}>
    <h3>{t("modes.presets")}</h3>
    <Hint id="presets" />
    <div class="presets">
      {#each [["charge", "modes.chargeBtn"], ["discharge", "modes.dischargeBtn"]] as const as [k, label]}
        {@const name = t(label)}
        <div class="preset k-{k}">
          <strong>{name}</strong>
          <label class="field">{t("modes.powerMax", { cap })}<input type="number" class="num" min="0" max={cap} step="50" bind:value={presets[k].power} onchange={savePresets} /></label>
          <label class="field">{t("modes.duration")}<input type="number" class="num" min="1" step="5" bind:value={presets[k].minutes} onchange={savePresets} /></label>
          <button class="btn tone" disabled={off} onclick={() => apply(`${name} ${Math.min(cap, presets[k].power)} W, ${presets[k].minutes} min`, passive(k, presets[k].power, presets[k].minutes))}>{t("common.launch")}</button>
        </div>
      {/each}
      <div class="preset k-auto">
        <strong>{t("modes.backAuto")}</strong>
        <p class="muted">{t("modes.backAutoHint")}</p>
        <button class="btn tone" disabled={off} onclick={() => apply(t("modes.backAutoLog"), { kind: "auto" })}>{t("common.activate")}</button>
      </div>
    </div>
  </section>

  <section class="card" aria-label={t("modes.auto")}>
    <h3>{t("modes.auto")}</h3>
    <label class="check"><input type="checkbox" checked={app.settings.autoControl} onchange={toggleAuto} /><strong>{t(app.settings.autoControl ? "modes.autoOn" : "modes.autoOff")}</strong></label>
    <Hint id="autoControl" />
    <label class="check"><input type="checkbox" checked={app.settings.reapplyPlan} disabled={!app.settings.autoControl} onchange={(e) => setting("reapplyPlan", e.currentTarget.checked)} /><strong>{t("modes.reapply")}</strong></label>
    <p class="muted">{t("modes.reapplyNote")}</p>
    <Hint id="reapplyPlan" />
  </section>

  <section class="card" aria-label={t("modes.rules")}>
    <h3>{t("modes.rules")}</h3>
    <Hint id="rules" />
    <div class="rules">
      {#each [["low", "settings.lowSoc", "SOC ≤"], ["high", "modes.highSoc", "SOC ≥"]] as const as [k, label, cmp]}
        <div class="rule">
          <label class="check"><input type="checkbox" bind:checked={rules[k].enabled} onchange={saveRules} /><strong>{t(label)}</strong></label>
          <label class="field">{cmp} (%)<input type="number" class="num" min="0" max="100" bind:value={rules[k].soc} onchange={saveRules} /></label>
          <label class="field">{t("modes.switchTo")}
            <select bind:value={rules[k].mode} onchange={saveRules}>
              {#each ruleModes as v}<option value={v}>{v === "ai" ? t("mode.ai") : v === "ups" ? "UPS" : "Auto"}</option>{/each}
            </select>
          </label>
        </div>
      {/each}
    </div>
    {#if overlap}<p class="warn" role="alert">{t("modes.overlap")}</p>{/if}
  </section>
</div>

<Confirm bind:this={confirm} />

<style>
  .notice { padding: 12px 16px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-2); color: var(--muted); }
  .now { display: flex; flex-wrap: wrap; gap: 16px 48px; padding: 4px 2px; }
  .now > div { display: grid; gap: 2px; }
  .lab { font-size: var(--text-sm); color: var(--muted); }
  .big { font-family: var(--font-display); font-size: var(--text-xl); }
  .big[class*="k-"] { color: color-mix(in oklab, var(--k) 75%, var(--text)); }

  .modes {
    display: grid; grid-template-columns: 220px minmax(0, 1fr);
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); overflow: hidden;
  }
  .list { list-style: none; margin: 0; padding: 8px; display: grid; gap: 2px; align-content: start; background: var(--bg); border-right: 1px solid var(--border); }
  .item {
    width: 100%; display: flex; align-items: center; gap: 12px; min-height: 36px; padding: 0 12px;
    background: none; border: 0; border-radius: var(--radius-sm); cursor: pointer; text-align: left; color: var(--muted);
    transition: background-color var(--dur-micro), color var(--dur-micro);
  }
  .item:hover { background: var(--surface-2); color: var(--text); }
  .item[aria-pressed="true"] { background: var(--surface-2); color: var(--text); font-weight: 600; box-shadow: inset 3px 0 0 var(--k); }
  .item:focus-visible { outline-offset: -2px; }
  .cur { margin-left: auto; font-size: var(--text-xs); color: var(--text); padding: 1px 8px; border-radius: 999px; border: 1px solid color-mix(in oklab, var(--k) 60%, var(--border)); }

  .fiche { padding: 20px 24px; display: grid; gap: 16px; align-content: start; border-top: 3px solid var(--k); }
  .fiche h2 { font-size: var(--text-xl); color: color-mix(in oklab, var(--k) 75%, var(--text)); }
  .fiche dl { grid-template-columns: 190px minmax(0, 1fr); gap: 12px 20px; max-width: 900px; }
  .fiche dd { text-align: left; }
  .act { display: grid; gap: 12px; justify-items: start; padding-top: 4px; }

  .presets { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; margin-top: 12px; }
  .preset { display: grid; gap: 8px; align-content: start; padding: 12px; background: var(--surface-2); border-radius: var(--radius-sm); border-top: 3px solid var(--k); }
  .preset strong { color: color-mix(in oklab, var(--k) 75%, var(--text)); }
  .preset .btn { justify-self: start; }
  .rules { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; margin-top: 12px; }
  .rule { display: grid; gap: 8px; align-content: start; padding: 12px; background: var(--surface-2); border-radius: var(--radius-sm); }
  .check { display: flex; align-items: center; gap: 8px; }
  .warn { margin-top: 12px; color: var(--danger); }

  @media (max-width: 900px) {
    .modes { grid-template-columns: minmax(0, 1fr); }
    .list { border-right: 0; border-bottom: 1px solid var(--border); }
    .fiche dl { grid-template-columns: minmax(0, 1fr); }
  }
</style>
