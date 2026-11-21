<script lang="ts">
  import { setMode, type Direction, type ModeKind, type ModeReq } from "../lib/api";
  import { app, setting, write } from "../lib/state.svelte";
  import { clone } from "../lib/store";

  const modeNames: Record<string, string> = { Auto: "Auto", AI: "IA", Manual: "Manuel", Passive: "Passif", UPS: "UPS" };

  const off = $derived(!app.connectedIp || app.busy);
  const current = $derived(app.snapshot?.mode?.mode);

  let presets = $state(clone(app.settings.presets));
  const savePresets = () => setting("presets", clone(presets));

  let direction = $state<Direction>("charge");
  let power = $state(500);
  let minutes = $state(30);

  let rules = $state(clone(app.settings.rules));
  const saveRules = () => setting("rules", clone(rules));
  const ruleModes: [ModeKind, string][] = [["auto", "Auto"], ["ai", "IA"], ["ups", "UPS"]];
  const sources = { plan: "Plan", rule: "Règle SOC", manual: "Action manuelle" };
  const overlap = $derived(rules.low.enabled && rules.high.enabled && rules.low.soc >= rules.high.soc);

  const apply = (label: string, req: ModeReq) => write(label, () => setMode(req));
  const passive = (d: Direction, w: number, min: number): ModeReq => ({ kind: "passive", direction: d, power: Math.max(0, w), seconds: Math.max(1, Math.round(min * 60)) });
  const dirName = (d: Direction) => (d === "charge" ? "Charge" : "Décharge");
</script>

<div class="stack">
  {#if !app.connectedIp}
    <p class="notice" role="status">Aucun appareil connecté : les commandes sont désactivées. Choisissez une batterie dans l'onglet Appareils.</p>
  {/if}

  <section class="card" aria-label="Préréglages">
    <h3>Préréglages</h3>
    <div class="presets">
      {#each [["charge", "Charger"], ["discharge", "Décharger"]] as const as [k, name]}
        <div class="preset">
          <strong class={k}>{name}</strong>
          <label class="field">Puissance (W)<input type="number" min="0" step="50" bind:value={presets[k].power} onchange={savePresets} /></label>
          <label class="field">Durée (min)<input type="number" min="1" step="5" bind:value={presets[k].minutes} onchange={savePresets} /></label>
          <button class="btn primary" disabled={off} onclick={() => apply(`${name} ${presets[k].power} W, ${presets[k].minutes} min`, passive(k, presets[k].power, presets[k].minutes))}>Lancer</button>
        </div>
      {/each}
      <div class="preset">
        <strong>Retour Auto</strong>
        <p class="muted">Rend le pilotage à la batterie.</p>
        <button class="btn primary" disabled={off} onclick={() => apply("Retour en mode Auto", { kind: "auto" })}>Activer</button>
      </div>
    </div>
  </section>

  <section class="card" aria-label="Modes de fonctionnement">
    <h3>Mode de fonctionnement</h3>
    <p class="cur">Mode actuel : <strong>{current ? (modeNames[current] ?? current) : "inconnu"}</strong></p>
    <div class="modes">
      <div class="mode" class:active={current === "Auto"}>
        <strong>Auto</strong>
        <p class="muted">La batterie suit la consommation mesurée par le compteur CT.</p>
        <button class="btn" disabled={off} onclick={() => apply("Mode Auto", { kind: "auto" })}>Activer</button>
      </div>
      <div class="mode" class:active={current === "AI"}>
        <strong>IA</strong>
        <p class="muted">Pilotage automatique optimisé par l'appareil.</p>
        <button class="btn" disabled={off} onclick={() => apply("Mode IA", { kind: "ai" })}>Activer</button>
      </div>
      <div class="mode" class:active={current === "UPS"}>
        <strong>UPS</strong>
        <p class="muted">Priorité à l'alimentation de secours de la sortie hors réseau.</p>
        <button class="btn" disabled={off} onclick={() => apply("Mode UPS", { kind: "ups" })}>Activer</button>
      </div>
      <div class="mode passive" class:active={current === "Passive"}>
        <strong>Passif</strong>
        <div class="seg" role="group" aria-label="Direction">
          <button aria-pressed={direction === "charge"} onclick={() => (direction = "charge")}>Charge</button>
          <button aria-pressed={direction === "discharge"} onclick={() => (direction = "discharge")}>Décharge</button>
        </div>
        <div class="two">
          <label class="field">Puissance (W)<input type="number" min="0" step="50" bind:value={power} /></label>
          <label class="field">Durée (min)<input type="number" min="1" step="5" bind:value={minutes} /></label>
        </div>
        <button class="btn" disabled={off} onclick={() => apply(`Mode Passif, ${dirName(direction).toLowerCase()} ${power} W, ${minutes} min`, passive(direction, power, minutes))}>Appliquer</button>
      </div>
    </div>
  </section>

  <section class="card" aria-label="Règles SOC">
    <h3>Règles SOC</h3>
    <p class="cur" role="status">Piloté par : <strong>{app.control ? sources[app.control.source] : "inconnu"}</strong>{#if app.control?.detail}<span class="muted"> · {app.control.detail}</span>{/if}</p>
    <p class="muted intro">Une règle active prime sur le plan et sur les actions manuelles. Elle est relâchée avec 5 points d'hystérésis.</p>
    <div class="rules">
      {#each [["low", "SOC bas", "SOC ≤"], ["high", "SOC haut", "SOC ≥"]] as const as [k, name, cmp]}
        <div class="rule">
          <label class="check"><input type="checkbox" bind:checked={rules[k].enabled} onchange={saveRules} /><strong>{name}</strong></label>
          <label class="field">{cmp} (%)<input type="number" min="0" max="100" bind:value={rules[k].soc} onchange={saveRules} /></label>
          <label class="field">Passer en mode
            <select bind:value={rules[k].mode} onchange={saveRules}>
              {#each ruleModes as [v, label]}<option value={v}>{label}</option>{/each}
            </select>
          </label>
        </div>
      {/each}
    </div>
    {#if overlap}<p class="warn" role="alert">Le seuil bas doit rester inférieur au seuil haut.</p>{/if}
  </section>
</div>

<style>
  .notice { padding: 10px 14px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface-2); color: var(--muted); }
  .presets { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .preset { display: grid; gap: 8px; align-content: start; padding: 12px; border: 1px solid var(--border); border-radius: 8px; }
  .preset .btn { justify-self: start; }
  .charge { color: var(--charge-ink); }
  .discharge { color: var(--discharge-ink); }
  .cur { margin-bottom: 12px; }
  .modes { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
  .mode { display: grid; gap: 8px; align-content: start; padding: 12px; border: 1px solid var(--border); border-radius: 8px; }
  .mode .btn, .mode .seg { justify-self: start; }
  .mode.active { border-color: var(--text); box-shadow: inset 0 0 0 1px var(--text); }
  .intro { margin-bottom: 12px; }
  .rules { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
  .rule { display: grid; gap: 8px; align-content: start; padding: 12px; border: 1px solid var(--border); border-radius: 8px; }
  .check { display: flex; align-items: center; gap: 8px; }
  .warn { margin-top: 10px; color: var(--danger); }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .two input { min-width: 0; }
</style>
