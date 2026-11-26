<script lang="ts">
  import Icon from "./Icon.svelte";
  import { t } from "../lib/i18n.svelte";
  import { STEPS, describeCell, isManual, kindName, runs, slotTime, type Band, type Cell } from "../lib/plan";

  type Range = { a: number; b: number };
  /** Une journée : `cells` (96 cases de 15 min), `paint` ce que pose le glisser (null = vider), `selected` les blocs choisis. */
  let {
    cells,
    paint,
    piloted,
    day,
    now,
    selected,
    bands = [],
    tiers = 0,
    onpaint,
    onselect,
    onmove,
    onresize,
    ondelete,
  }: {
    cells: Cell[];
    paint: Cell;
    piloted: boolean;
    day: string;
    now: number | null;
    selected: Range[];
    /** Bandes de prix du tarif en fond (heures creuses/pleines) ; vide : pas de fond. */
    bands?: Band[];
    tiers?: number;
    onpaint: (a: number, b: number, value: Cell) => void;
    onselect: (r: Range | null, additive: boolean) => void;
    onmove: (d: number) => void;
    onresize: (d: number) => void;
    ondelete: () => void;
  } = $props();


  let track: HTMLDivElement;
  let cursor = $state(32);
  let sel = $state<Range>();
  let press: { at: number; shift: boolean; dragged: boolean } | undefined;

  const blocks = $derived(runs(cells));
  const pct = (i: number) => `${(i / STEPS) * 100}%`;
  const lo = $derived(sel ? Math.min(sel.a, sel.b) : cursor);
  const hi = $derived(sel ? Math.max(sel.a, sel.b) : cursor);
  const isSel = (r: Range) => selected.some((x) => x.a === r.a && x.b === r.b);
  const runAt = (i: number) => blocks.find((r) => r.a <= i && i < r.b);
  const needsPc = (c: NonNullable<Cell>) => (!isManual(c.kind) && piloted) || c.socLimit != null;

  const slotAt = (e: PointerEvent) => {
    const r = track.getBoundingClientRect();
    return Math.max(0, Math.min(STEPS - 1, Math.floor(((e.clientX - r.left) / r.width) * STEPS)));
  };

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    track.setPointerCapture(e.pointerId);
    const i = slotAt(e);
    press = { at: i, shift: e.shiftKey, dragged: false };
    cursor = i;
  }
  function move(e: PointerEvent) {
    if (!press) return;
    const i = slotAt(e);
    if (i !== press.at) press.dragged = true;
    if (press.dragged) sel = { a: press.at, b: (cursor = i) };
  }
  function up() {
    if (!press) return;
    if (press.dragged) onpaint(lo, hi, paint);
    else {
      const r = runAt(press.at);
      onselect(r ? { a: r.a, b: r.b } : null, press.shift);
    }
    press = undefined;
    sel = undefined;
  }

  const STEP: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowDown: -1, ArrowUp: 1, PageDown: -4, PageUp: 4 };

  function key(e: KeyboardEvent) {
    const d = STEP[e.key];
    if (selected.length) {
      if (d) {
        e.preventDefault();
        (e.shiftKey ? onresize : onmove)(d);
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        ondelete();
      } else if (e.key === "Escape") onselect(null, false);
      return;
    }
    if (d || e.key === "Home" || e.key === "End") {
      e.preventDefault();
      const next = e.key === "Home" ? 0 : e.key === "End" ? STEPS - 1 : Math.max(0, Math.min(STEPS - 1, cursor + d));
      sel = e.shiftKey ? { a: sel?.a ?? cursor, b: next } : undefined;
      cursor = next;
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const r = runAt(cursor);
      if (!sel && r) onselect({ a: r.a, b: r.b }, e.shiftKey);
      else onpaint(lo, hi, paint);
      sel = undefined;
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      onpaint(lo, hi, null);
      sel = undefined;
    } else if (e.key === "Escape") sel = undefined;
  }

  const span = (a: number, b: number) => t("confirm.range", { start: slotTime(a), end: slotTime(b) });
  const status = $derived(
    selected.length
      ? `${selected.length > 1 ? t("tl.selMany", { n: selected.length }) : t("tl.selOne", { range: span(selected[0].a, selected[0].b) })}. ${t("tl.selHelp")}`
      : sel
        ? t("tl.range", { range: span(lo, hi + 1) })
        : `${t("tl.cursor", { time: slotTime(cursor), desc: describeCell(cells[cursor]) })}${cells[cursor] && needsPc(cells[cursor]!) ? t("tl.needsPc") : ""}`,
  );
</script>

<div class="wrap">
  {#if tiers > 1}
    <div class="tariff-bar" aria-hidden="true">
      {#each bands as b (b.a)}
        <div class="tb" style:left={pct(b.a)} style:width={pct(b.b - b.a)} style:--r={(b.tier / (tiers - 1)).toFixed(3)}></div>
      {/each}
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="track"
    bind:this={track}
    role="application"
    tabindex="0"
    aria-roledescription={t("tl.roledesc")}
    aria-label={t("tl.label", { day })}
    aria-describedby="tl-help"
    onkeydown={key}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={() => ((press = undefined), (sel = undefined))}>
    {#if tiers > 1}
      {#each bands as b (b.a)}
        <div class="band" style:left={pct(b.a)} style:width={pct(b.b - b.a)} style:--r={(b.tier / (tiers - 1)).toFixed(3)} aria-hidden="true"></div>
      {/each}
    {/if}
    {#each blocks as r (r.a)}
      <div
        class="block k-{r.cell.kind}"
        class:pc={needsPc(r.cell)}
        class:picked={isSel(r)}
        class:live={now != null && r.a <= now && now < r.b}
        style:left={pct(r.a)}
        style:width={pct(r.b - r.a)}
        title="{t("tl.title", { range: span(r.a, r.b), desc: describeCell(r.cell) })}{needsPc(r.cell) ? t("tl.needsPcParen") : ''}">
        <span class="name"><Icon name={r.cell.kind} size={14} />{kindName(r.cell.kind)}</span>
        {#if isManual(r.cell.kind)}<span class="num">{r.cell.power} W</span>{/if}
        {#if r.cell.socLimit != null}<span class="num lim">{t(r.cell.kind === "charge" ? "tl.upTo" : "tl.notBelow", { soc: r.cell.socLimit })}</span>{/if}
        <span class="num when">{span(r.a, r.b)}</span>
      </div>
    {/each}
    {#if sel}
      <div class="sel {paint ? `k-${paint.kind}` : 'erase'}" style:left={pct(lo)} style:width={pct(hi - lo + 1)} aria-hidden="true"></div>
    {/if}
    <div class="cursor" style:left={pct(cursor)} style:width={pct(1)} aria-hidden="true"></div>
    {#if now != null}<div class="now" style:left={pct(now)} aria-hidden="true"><span>{t("tl.now")}</span></div>{/if}
  </div>
  <div class="hours num" aria-hidden="true">
    {#each Array(25) as _, h}<span style:left={pct(h * 4)} class:major={h % 6 === 0}>{String(h).padStart(2, "0")}</span>{/each}
  </div>
</div>
<p class="visually-hidden" aria-live="polite">{status}</p>
<p id="tl-help" class="help muted">
  {t("tl.help")}
</p>

<style>
  .wrap { padding-bottom: 24px; position: relative; }
  .track {
    position: relative; height: 132px; border-radius: var(--radius-sm); cursor: crosshair;
    user-select: none; touch-action: none; overflow: hidden;
    background-color: var(--bg);
    background-image:
      linear-gradient(to right, var(--border-strong) 1px, transparent 1px),
      linear-gradient(to right, var(--border) 1px, transparent 1px),
      linear-gradient(to right, color-mix(in oklab, var(--border) 45%, transparent) 1px, transparent 1px);
    background-size: calc(100% / 24) 100%, calc(100% / 96) 100%, calc(100% / 288) 34%;
    background-repeat: repeat-x;
    background-position: 0 0, 0 0, 0 100%;
    box-shadow: inset 0 0 0 1px var(--border);
  }
  .tariff-bar { position: relative; height: 10px; margin-bottom: 6px; border-radius: 3px; overflow: hidden; box-shadow: inset 0 0 0 1px var(--border); }
  .tb { position: absolute; top: 0; bottom: 0; background: color-mix(in oklab, var(--discharge) calc(var(--r) * 100%), var(--charge)); }
  .band {
    position: absolute; top: 0; bottom: 0; z-index: 0; pointer-events: none;
    background: color-mix(in oklab, color-mix(in oklab, var(--discharge) calc(var(--r) * 100%), var(--charge)) 16%, transparent);
  }
  .block {
    position: absolute; top: 10px; bottom: 10px; padding: 8px 8px 8px;
    display: flex; flex-direction: column; gap: 1px; overflow: hidden; white-space: nowrap;
    background: color-mix(in oklab, var(--k) var(--tint), var(--surface));
    border-top: 3px solid var(--k); border-right: 1px solid var(--bg);
    font-size: var(--text-sm); line-height: 1.25; pointer-events: none;
    --sheen: linear-gradient(180deg, color-mix(in oklab, var(--k) 20%, transparent), transparent 65%);
    background-image: var(--sheen);
  }
  .block.pc {
    background-image: repeating-linear-gradient(135deg, color-mix(in oklab, var(--k) 30%, transparent) 0 2px, transparent 2px 9px), var(--sheen);
  }
  .block.picked { box-shadow: inset 0 0 0 2px var(--text), 0 0 14px color-mix(in oklab, var(--k) var(--glow), transparent); z-index: 1; }
  .block.live { box-shadow: inset 0 -2px 0 var(--k), 0 0 12px color-mix(in oklab, var(--k) var(--glow), transparent); animation: live 2.4s ease-in-out infinite; }
  .block.live.picked { box-shadow: inset 0 0 0 2px var(--text), 0 0 14px color-mix(in oklab, var(--k) var(--glow), transparent); }
  .block.picked::before, .block.picked::after { content: ""; position: absolute; top: 50%; width: 4px; height: 22px; margin-top: -12px; border-radius: 2px; background: var(--text); }
  .block.picked::before { left: 2px; }
  .block.picked::after { right: 2px; }
  .name { display: flex; align-items: center; gap: 4px; font-weight: 700; color: color-mix(in oklab, var(--k) 70%, var(--text)); }
  .when { margin-top: auto; color: var(--muted); font-size: var(--text-xs); }
  .lim { color: var(--muted); }
  .sel {
    position: absolute; top: 4px; bottom: 4px; pointer-events: none; border-radius: 3px;
    background: color-mix(in oklab, var(--k) 45%, transparent); box-shadow: inset 0 0 0 2px var(--k);
  }
  .sel.erase { --k: var(--muted); background: color-mix(in oklab, var(--bg) 70%, transparent); }
  .cursor { position: absolute; top: 0; bottom: 0; pointer-events: none; box-shadow: inset 0 0 0 2px var(--focus); opacity: 0; }
  .track:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
  .track:focus-visible .cursor { opacity: 1; }
  .now {
    position: absolute; top: 0; bottom: 0; width: 2px; margin-left: -1px; background: var(--text); pointer-events: none; z-index: 2;
    box-shadow: 0 0 8px 1px color-mix(in oklab, var(--focus) var(--glow), transparent); animation: beam 2.4s ease-in-out infinite;
  }
  .now span {
    position: absolute; top: 0; left: 4px; padding: 0 4px; border-radius: 3px;
    font-size: var(--text-xs); background: var(--text); color: var(--bg);
  }
  @keyframes live { 50% { filter: brightness(1.18); } }
  @keyframes beam { 50% { box-shadow: 0 0 3px 0 color-mix(in oklab, var(--focus) var(--glow), transparent); } }
  .hours { position: absolute; left: 0; right: 0; bottom: 0; height: 18px; font-size: var(--text-xs); color: var(--muted); }
  .hours span { position: absolute; transform: translateX(-50%); }
  .hours span:first-child { transform: none; }
  .hours span:last-child { transform: translateX(-100%); }
  .hours .major { color: var(--text); font-weight: 600; }
  .help { margin-top: 8px; font-size: var(--text-sm); }
</style>
