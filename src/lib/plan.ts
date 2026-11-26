import type { Block, BlockKind, Plan, Price } from "./api";
import { t, type Key } from "./i18n.svelte";

export const STEPS = 96;

const hhmm = (s: string) => { const [h, m] = s.split(":").map(Number); return (h || 0) * 60 + (m || 0); };
const priceAt = (p: Price, minute: number) => {
  for (const s of p.slots) {
    const a = hhmm(s.start), b = hhmm(s.end);
    if (a <= b ? minute >= a && minute < b : minute >= a || minute < b) return s.price;
  }
  return p.default;
};

/** Bande de prix sur la frise : pas `[a, b)` au même prix, `tier` = rang du prix (0 = le moins cher). */
export type Band = { a: number; b: number; tier: number; price: number };
/** Découpe la journée (96 pas de 15 min) en bandes contiguës de même prix, classées du moins cher au plus cher. */
export function tariffBands(p: Price): { bands: Band[]; tiers: number } {
  const prices = Array.from({ length: STEPS }, (_, i) => priceAt(p, i * 15));
  const distinct = [...new Set(prices)].sort((a, b) => a - b);
  const bands: Band[] = [];
  for (let i = 0; i < STEPS; i++) {
    const price = prices[i];
    const last = bands.at(-1);
    if (last && last.price === price) last.b = i + 1;
    else bands.push({ a: i, b: i + 1, tier: distinct.indexOf(price), price });
  }
  return { bands, tiers: distinct.length };
}
export type Cell = { kind: BlockKind; power: number; socLimit?: number } | null;

export const isManual = (k: BlockKind) => k === "charge" || k === "discharge";
/** Nom d'un type de bloc, en tête de phrase. */
export const kindName = (k: BlockKind) => (k === "auto" ? "Auto" : k === "ups" ? "UPS" : t(k === "ai" ? "mode.ai" : k === "charge" ? "dir.charge" : "dir.discharge"));

const kindLabel = (k: BlockKind) => (k === "auto" ? "Auto" : k === "ups" ? "UPS" : t(k === "ai" ? "mode.ai" : (`kind.${k}` as Key)));

export function describeCell(c: Cell): string {
  if (!c) return t("plan.empty");
  if (!isManual(c.kind)) return kindLabel(c.kind);
  const limit = c.socLimit == null ? "" : c.kind === "charge" ? t("plan.target", { soc: c.socLimit }) : t("plan.floor", { soc: c.socLimit });
  return `${kindLabel(c.kind)} ${c.power} W${limit}`;
}

const same = (a: Cell, b: Cell) => a?.kind === b?.kind && a?.power === b?.power && a?.socLimit === b?.socLimit;

/** Anciens blocs `{ direction, power }` : lus comme charge ou décharge. */
export const migratePlan = (p: Plan): Plan => ({
  ...p,
  blocks: p.blocks.map((b) => {
    const { direction, ...rest } = b as Block & { direction?: BlockKind };
    return { ...rest, kind: rest.kind ?? direction };
  }),
});

export const slotTime = (i: number) => (i >= STEPS ? "24:00" : `${String(Math.floor(i / 4)).padStart(2, "0")}:${String((i % 4) * 15).padStart(2, "0")}`);
const slotIndex = (t: string) => {
  const [h, m] = t.split(":").map(Number);
  return Math.min(STEPS, Math.round((h * 60 + m) / 15));
};

export function toCells(blocks: Block[]): Cell[][] {
  const cells: Cell[][] = Array.from({ length: 7 }, () => Array<Cell>(STEPS).fill(null));
  for (const b of blocks)
    for (let i = slotIndex(b.start); i < slotIndex(b.end); i++) cells[b.day][i] = { kind: b.kind, power: b.power, ...(b.socLimit != null && { socLimit: b.socLimit }) };
  return cells;
}

/** Suites de cases identiques d'une journée : `b` exclu. */
export function runs(col: Cell[]) {
  const out: { a: number; b: number; cell: NonNullable<Cell> }[] = [];
  let start = 0;
  for (let i = 1; i <= STEPS; i++) {
    const a = col[start];
    if (i < STEPS && same(a, col[i])) continue;
    if (a) out.push({ a: start, b: i, cell: a });
    start = i;
  }
  return out;
}

export const fromCells = (cells: Cell[][]): Block[] =>
  cells.flatMap((col, day) => runs(col).map(({ a, b, cell }) => ({ day, start: slotTime(a), end: slotTime(b), ...cell })));

/** Nombre de plages de 15 min contiguës qui diffèrent entre deux plans. */
export function diffRuns(a: Block[], b: Block[]) {
  const x = toCells(a);
  const y = toCells(b);
  let n = 0;
  for (let d = 0; d < 7; d++)
    for (let i = 0; i < STEPS; i++) if (!same(x[d][i], y[d][i]) && (i === 0 || same(x[d][i - 1], y[d][i - 1]))) n++;
  return n;
}

export const WEEKDAYS = 0b0011111;
export const WEEKEND = 0b1100000;
export const ALL_DAYS = 0b1111111;

/** Un bloc d'une journée et tous les jours où le même bloc existe à l'identique. */
export type Group = { a: number; b: number; days: number; cell: NonNullable<Cell> };

export function groupAt(cells: Cell[][], day: number, a: number, b: number): Group | undefined {
  const cell = cells[day][a];
  if (!cell || !runs(cells[day]).some((r) => r.a === a && r.b === b)) return;
  let days = 0;
  cells.forEach((col, d) => runs(col).some((r) => r.a === a && r.b === b && same(r.cell, cell)) && (days |= 1 << d));
  return { a, b, days, cell };
}

/** Efface les groupes `from` puis pose les groupes `to` : un bloc déplacé n'efface pas son voisin déplacé avec lui. */
export function rewrite(cells: Cell[][], from: Group[], to: Group[]): Cell[][] {
  const next = cells.map((c) => [...c]);
  for (const [list, put] of [[from, false], [to, true]] as const)
    for (const g of list)
      for (let d = 0; d < 7; d++) if (g.days & (1 << d)) for (let i = g.a; i < g.b; i++) next[d][i] = put ? g.cell : null;
  return next;
}

/** "hh:mm" (jusqu'à 24:00) arrondi au quart d'heure ; undefined si illisible. */
export function parseSlot(t: string): number | undefined {
  const m = /^(\d{1,2}):(\d{2})$/.exec(t.trim());
  if (!m) return;
  const min = Number(m[1]) * 60 + Number(m[2]);
  return Number(m[2]) < 60 && min <= 1440 ? Math.round(min / 15) : undefined;
}
