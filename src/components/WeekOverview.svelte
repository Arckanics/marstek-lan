<script lang="ts">
  import { dayName, t } from "../lib/i18n.svelte";
  import { STEPS, describeCell, isManual, runs, slotTime, type Cell } from "../lib/plan";

  let { cells, open, targets, piloted, onopen }: { cells: Cell[][]; open: number; targets: number; piloted: boolean; onopen: (day: number) => void } = $props();

  const pct = (i: number) => `${(i / STEPS) * 100}%`;
  const summary = (d: number) => runs(cells[d]).map((r) => `${t("confirm.range", { start: slotTime(r.a), end: slotTime(r.b) })} ${describeCell(r.cell)}`).join(", ") || t("plan.empty");
</script>

<ol class="week">
  {#each { length: 7 } as _, d}
    {@const name = dayName(d)}
    <li>
      <button
        type="button"
        class="day"
        class:open={d === open}
        class:target={d !== open && targets & (1 << d)}
        aria-current={d === open ? "date" : undefined}
        aria-label={t("week.open", { name, summary: summary(d) })}
        onclick={() => onopen(d)}>
        <span class="dn">{name.slice(0, 3)}</span>
        <span class="mini">
          {#each runs(cells[d]) as r (r.a)}
            <span class="b k-{r.cell.kind}" class:pc={(!isManual(r.cell.kind) && piloted) || r.cell.socLimit != null} style:left={pct(r.a)} style:width={pct(r.b - r.a)}></span>
          {/each}
        </span>
      </button>
    </li>
  {/each}
</ol>
<div class="axis num muted" aria-hidden="true"><span>00</span><span>06</span><span>12</span><span>18</span><span>24</span></div>

<style>
  .week { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; }
  .day {
    width: 100%; display: grid; grid-template-columns: 44px minmax(0, 1fr); align-items: center; gap: 12px;
    min-height: 32px; padding: 4px 8px; background: none; border: 1px solid transparent; border-radius: var(--radius-sm); cursor: pointer;
    transition: background-color var(--dur-micro), border-color var(--dur-micro);
  }
  .day:hover { background: var(--surface-2); }
  .day.target { border-color: var(--border-strong); border-style: dashed; }
  .day.open { background: var(--surface-2); border-color: var(--text); }
  .dn { text-align: left; color: var(--muted); font-weight: 600; }
  .open .dn { color: var(--text); }
  .mini {
    position: relative; height: 18px; border-radius: 3px; background-color: var(--bg); overflow: hidden;
    background-image: linear-gradient(to right, var(--border) 1px, transparent 1px);
    background-size: calc(100% / 4) 100%;
  }
  .b { position: absolute; top: 0; bottom: 0; background: var(--k); border-right: 1px solid var(--bg); }
  .b.pc { background-image: repeating-linear-gradient(135deg, var(--bg) 0 2px, transparent 2px 6px); }
  .axis { display: flex; justify-content: space-between; margin: 4px 8px 0 60px; font-size: var(--text-xs); }
</style>
