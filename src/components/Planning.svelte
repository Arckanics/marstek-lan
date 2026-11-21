<script lang="ts">
  import { applyPlan, compilePlan, type Block, type Direction, type Slot } from "../lib/api";
  import { app, errText, setting, write } from "../lib/state.svelte";
  import { clone } from "../lib/store";
  import { dayNames } from "../lib/plan";
  import Confirm from "./Confirm.svelte";
  import Days from "./Days.svelte";
  import PlanGrid from "./PlanGrid.svelte";
  import SlotTest from "./SlotTest.svelte";
  import Tariff from "./Tariff.svelte";

  const MAX = 10;
  let confirm: Confirm;

  let sel = $state(0);
  const plans = $derived(app.settings.plans);
  const plan = $derived(plans[Math.min(sel, plans.length - 1)]);
  const off = $derived(!app.connectedIp || app.busy);

  let tool = $state<Direction | "erase">("charge");
  let power = $state(800);
  let from = $state(0);
  let targets = $state(0);

  let count = $state<number>();
  let error = $state("");

  const save = (next: typeof plans) => setting("plans", next);
  const uniqueName = (base: string) => {
    let n = base;
    for (let i = 2; plans.some((p) => p.name === n); i++) n = `${base} ${i}`;
    return n;
  };
  const setBlocks = (blocks: Block[]) => save(plans.map((p) => (p === plan ? { ...p, blocks } : p)));

  function add(blocks: Block[] = []) {
    const n = plans.length;
    save([...clone(plans), { name: uniqueName("Nouveau modèle"), blocks }]);
    sel = n;
  }
  function duplicate() {
    const n = plans.length;
    save([...clone(plans), { name: uniqueName(`${plan.name} (copie)`), blocks: clone(plan.blocks) }]);
    sel = n;
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
  async function remove() {
    if (!(await confirm.ask(`Supprimer le modèle « ${plan.name} » ?`, { title: "Supprimer le modèle", ok: "Supprimer" }))) return;
    save(plans.filter((p) => p !== plan));
    sel = 0;
  }
  function copyDay() {
    const src = plan.blocks.filter((b) => b.day === from);
    const rest = plan.blocks.filter((b) => !(targets & (1 << b.day)) || b.day === from);
    const copies = dayNames.flatMap((_, d) => (d !== from && targets & (1 << d) ? src.map((b) => ({ ...b, day: d })) : []));
    setBlocks([...rest, ...copies]);
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

  const slotLine = (s: Slot) => `${s.start} – ${s.end}, ${s.direction === "charge" ? "charge" : "décharge"} ${s.power} W, ${dayNames.filter((_, i) => s.days & (1 << i)).map((n) => n.slice(0, 3)).join(" ")}`;

  async function send() {
    let slots: Slot[];
    try {
      slots = await compilePlan(clone(plan));
    } catch (e) {
      error = errText(e);
      return;
    }
    const lines = slots.map(slotLine).join("\n") || "Aucun créneau : tous les créneaux seront désactivés.";
    if (!(await confirm.ask(`Le plan « ${plan.name} » sera écrit (${slots.length}/${MAX} créneaux) et l'appareil passera en mode Manuel.\n${lines}`))) return;
    const r = await write(`Envoi du plan « ${plan.name} »`, () => applyPlan(clone(plan)), (r) => r.every(Boolean));
    if (r?.every(Boolean)) app.settings.sentPlan = clone(plan);
  }
</script>

<div class="stack">
  <section class="card" aria-label="Modèles">
    <h3>Modèles</h3>
    {#if plan}
      <div class="row">
        <label class="field">Modèle
          <select bind:value={sel}>
            {#each plans as p, i}<option value={i}>{p.name}</option>{/each}
          </select>
        </label>
        {#key plan.name}
          <label class="field">Nom<input type="text" value={plan.name} onchange={rename} /></label>
        {/key}
        <button class="btn" onclick={() => add()}>Nouveau</button>
        <button class="btn" onclick={duplicate}>Dupliquer</button>
        <button class="btn danger" onclick={remove}>Supprimer</button>
      </div>
    {:else}
      <p class="muted create">Aucun modèle. Créez-en un pour dessiner votre planning hebdomadaire.</p>
      <button class="btn primary" onclick={() => add()}>Créer un modèle</button>
    {/if}
  </section>

  {#if plan}
    <section class="card" aria-label="Grille hebdomadaire">
      <h3>Grille</h3>
      <div class="row tools">
        <div class="seg" role="group" aria-label="Outil">
          <button aria-pressed={tool === "charge"} onclick={() => (tool = "charge")}>Charge</button>
          <button aria-pressed={tool === "discharge"} onclick={() => (tool = "discharge")}>Décharge</button>
          <button aria-pressed={tool === "erase"} onclick={() => (tool = "erase")}>Effacer</button>
        </div>
        <label class="field">Puissance (W)<input type="number" min="0" step="50" bind:value={power} disabled={tool === "erase"} /></label>
        <span class="legend"><i class="sw charge"></i>Charge<i class="sw discharge"></i>Décharge</span>
      </div>
      <PlanGrid blocks={plan.blocks} {tool} {power} onchange={setBlocks} />
    </section>

    <section class="card" aria-label="Copier un jour">
      <h3>Copier un jour</h3>
      <div class="row">
        <label class="field">Depuis
          <select bind:value={from}>{#each dayNames as n, i}<option value={i}>{n}</option>{/each}</select>
        </label>
        <div class="field">Vers<Days bind:days={targets} label="Jours de destination" /></div>
        <button class="btn" disabled={!(targets & ~(1 << from))} onclick={copyDay}>Copier</button>
      </div>
    </section>

    <Tariff onfill={setBlocks} />

    <section class="card" aria-label="Envoi">
      <h3>Envoi</h3>
      <div class="row send">
        <p class="muted">{app.settings.sentPlan ? `Modèle envoyé : « ${app.settings.sentPlan.name} »` : "Aucun modèle envoyé"}</p>
        <p class="count num" class:bad={error} aria-live="polite">
          {#if error}<span role="alert">{error}</span>{:else}{count ?? "…"}/{MAX} créneaux{/if}
        </p>
        <button class="btn primary" disabled={off || !!error || count == null} onclick={send}>Envoyer sur l'appareil</button>
      </div>
    </section>
  {/if}

  <SlotTest />
</div>

<Confirm bind:this={confirm} />

<style>
  .create { margin-bottom: 10px; }
  .tools { margin-bottom: 14px; }
  .legend { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); font-size: 0.85rem; min-height: 34px; }
  .sw { width: 14px; height: 14px; border-radius: 3px; display: inline-block; }
  .sw.charge { background: var(--charge); }
  .sw.discharge { background: repeating-linear-gradient(135deg, var(--discharge) 0 4px, transparent 4px 6px), var(--surface-2); margin-left: 8px; }
  .send { align-items: center; }
  .count { font-weight: 600; }
  .count.bad { color: var(--danger); }
</style>
