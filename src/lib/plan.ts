import type { Block, BlockKind, Plan } from "./api";

export const STEPS = 96;
export const dayNames = ["Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche"];
export type Cell = { kind: BlockKind; power: number; socLimit?: number } | null;

export const isManual = (k: BlockKind) => k === "charge" || k === "discharge";
export const kindLabel: Record<BlockKind, string> = { charge: "charge", discharge: "décharge", auto: "Auto", ai: "IA", ups: "UPS" };

export function describeCell(c: Cell): string {
  if (!c) return "vide";
  if (!isManual(c.kind)) return kindLabel[c.kind];
  const limit = c.socLimit == null ? "" : c.kind === "charge" ? `, cible ${c.socLimit} % de SOC` : `, plancher ${c.socLimit} % de SOC`;
  return `${kindLabel[c.kind]} ${c.power} W${limit}`;
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

export function fromCells(cells: Cell[][]): Block[] {
  const blocks: Block[] = [];
  cells.forEach((col, day) => {
    let start = 0;
    for (let i = 1; i <= STEPS; i++) {
      const a = col[start];
      const b = i < STEPS ? col[i] : null;
      if (same(a, b)) continue;
      if (a) blocks.push({ day, start: slotTime(start), end: slotTime(i), ...a });
      start = i;
    }
  });
  return blocks;
}
