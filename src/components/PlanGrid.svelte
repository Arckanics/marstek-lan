<script lang="ts">
  import type { Block } from "../lib/api";
  import { STEPS, dayNames, describeCell, fromCells, isManual, slotTime, toCells, type Cell } from "../lib/plan";

  /** `paint` : ce que la souris pose (null = effacer). `piloted` : les blocs Auto, IA et UPS demandent l'application. */
  let { blocks, paint, piloted, onchange }: { blocks: Block[]; paint: Cell; piloted: boolean; onchange: (b: Block[]) => void } = $props();

  const cells = $derived(toCells(blocks));
  const letters = { auto: "A", ai: "IA", ups: "UPS" } as const;

  let drag = $state<{ day: number; a: number; b: number }>();
  let cursor = $state({ day: 0, slot: 0 });
  let anchor = $state<number>();

  const inDrag = (d: number, i: number) => drag?.day === d && i >= Math.min(drag.a, drag.b) && i <= Math.max(drag.a, drag.b);
  const view = (d: number, i: number) => (inDrag(d, i) ? paint : cells[d][i]);

  function apply(day: number, a: number, b: number) {
    const next = cells.map((c) => [...c]);
    for (let i = Math.min(a, b); i <= Math.max(a, b); i++) next[day][i] = paint;
    onchange(fromCells(next));
  }

  function end() {
    if (!drag) return;
    apply(drag.day, drag.a, drag.b);
    drag = undefined;
  }

  const down = (e: PointerEvent, day: number, i: number) => {
    if (e.button !== 0) return;
    drag = { day, a: i, b: i };
    cursor = { day, slot: i };
  };
  const over = (day: number, i: number) => {
    if (drag && drag.day === day) drag.b = i;
  };

  function key(e: KeyboardEvent) {
    const { day, slot } = cursor;
    const move: Record<string, [number, number]> = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0], PageUp: [0, -4], PageDown: [0, 4] };
    const m = move[e.key];
    if (m) {
      e.preventDefault();
      cursor = { day: Math.min(6, Math.max(0, day + m[0])), slot: Math.min(STEPS - 1, Math.max(0, slot + m[1])) };
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      if (anchor == null) anchor = slot;
      else {
        apply(day, anchor, slot);
        anchor = undefined;
      }
    } else if (e.key === "Escape") anchor = undefined;
    else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      const next = cells.map((c) => [...c]);
      next[day][slot] = null;
      onchange(fromCells(next));
    }
  }

  const describe = (d: number, i: number) => {
    const c = cells[d][i];
    return `${dayNames[d]} ${slotTime(i)} : ${describeCell(c)}${c && !isManual(c.kind) && piloted ? " (PC allumé requis)" : ""}`;
  };
  const status = $derived(`${describe(cursor.day, cursor.slot)}${anchor != null ? `. Début de sélection à ${slotTime(anchor)}` : ""}`);
</script>

<svelte:window onpointerup={end} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="grid"
  role="application"
  tabindex="0"
  aria-label="Grille hebdomadaire, pas de 15 minutes"
  aria-describedby="grid-help"
  onkeydown={key}>
  <div class="hours" aria-hidden="true">
    {#each Array(STEPS) as _, i}<div class="h">{i % 4 === 0 ? slotTime(i) : ""}</div>{/each}
  </div>
  {#each dayNames as name, d}
    <div class="col">
      <div class="dn" aria-hidden="true">{name.slice(0, 3)}</div>
      {#each Array(STEPS) as _, i}
        {@const c = view(d, i)}
        <div
          class="cell"
          class:charge={c?.kind === "charge"}
          class:discharge={c?.kind === "discharge"}
          class:mode={c && !isManual(c.kind)}
          class:piloted={c && !isManual(c.kind) && piloted}
          class:hour={i % 4 === 0}
          class:cur={cursor.day === d && cursor.slot === i}
          class:anchor={d === cursor.day && i === anchor}
          title={describe(d, i)}
          role="presentation"
          onpointerdown={(e) => down(e, d, i)}
          onpointerenter={() => over(d, i)}>
          {#if c && c.kind in letters && view(d, i - 1)?.kind !== c.kind}{letters[c.kind as keyof typeof letters]}{/if}
        </div>
      {/each}
    </div>
  {/each}
</div>
<p id="grid-help" class="muted help">
  Glissez à la souris pour peindre avec l'outil choisi. Au clavier : flèches pour se déplacer, Espace pour marquer le début puis la fin d'un bloc, Suppr pour vider une case, Échap pour annuler.
</p>
<p class="sr" aria-live="polite">{status}</p>

<style>
  .grid { display: grid; grid-template-columns: 44px repeat(7, minmax(0, 1fr)); gap: 0 2px; user-select: none; touch-action: none; border-radius: 8px; }
  .hours, .col { display: grid; grid-template-rows: 22px repeat(96, 11px); }
  .hours::before { content: ""; }
  .h { font-size: 0.68rem; color: var(--muted); line-height: 11px; text-align: right; padding-right: 6px; transform: translateY(-5px); }
  .dn { font-size: 0.78rem; font-weight: 600; text-align: center; color: var(--muted); }
  .cell { background: var(--surface-2); cursor: crosshair; border-top: 1px solid transparent; }
  .cell.hour { border-top-color: var(--border); }
  .cell.charge { background: var(--charge); }
  .cell.discharge { background: repeating-linear-gradient(135deg, var(--discharge) 0 4px, transparent 4px 6px), var(--surface-2); }
  .cell.mode { background: color-mix(in srgb, var(--grid) 28%, var(--surface-2)); font-size: 0.6rem; line-height: 11px; font-weight: 700; color: var(--text); overflow: hidden; white-space: nowrap; }
  .cell.piloted { background: repeating-linear-gradient(90deg, var(--grid) 0 3px, transparent 3px 6px), var(--surface-2); }
  .cell.cur { outline: 2px solid var(--focus); outline-offset: -2px; position: relative; }
  .cell.anchor { box-shadow: inset 0 0 0 2px var(--text); }
  .grid:not(:focus-visible) .cell.cur { outline: 0; }
  .help { margin-top: 8px; font-size: 0.85rem; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
</style>
