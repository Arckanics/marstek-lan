import type { Block, Direction } from "./api";

export const STEPS = 96;
export const dayNames = ["Lundi", "Mardi", "Mercredi", "Jeudi", "Vendredi", "Samedi", "Dimanche"];
export type Cell = { direction: Direction; power: number } | null;

export const slotTime = (i: number) => (i >= STEPS ? "24:00" : `${String(Math.floor(i / 4)).padStart(2, "0")}:${String((i % 4) * 15).padStart(2, "0")}`);
const slotIndex = (t: string) => {
  const [h, m] = t.split(":").map(Number);
  return Math.min(STEPS, Math.round((h * 60 + m) / 15));
};

export function toCells(blocks: Block[]): Cell[][] {
  const cells: Cell[][] = Array.from({ length: 7 }, () => Array<Cell>(STEPS).fill(null));
  for (const b of blocks)
    for (let i = slotIndex(b.start); i < slotIndex(b.end); i++) cells[b.day][i] = { direction: b.direction, power: b.power };
  return cells;
}

export function fromCells(cells: Cell[][]): Block[] {
  const blocks: Block[] = [];
  cells.forEach((col, day) => {
    let start = 0;
    for (let i = 1; i <= STEPS; i++) {
      const a = col[start];
      const b = i < STEPS ? col[i] : null;
      if (a?.direction === b?.direction && a?.power === b?.power) continue;
      if (a) blocks.push({ day, start: slotTime(start), end: slotTime(i), direction: a.direction, power: a.power });
      start = i;
    }
  });
  return blocks;
}
