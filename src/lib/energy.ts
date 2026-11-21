import type { PowerSeries, Snapshot } from "./api";

// shortcut: signes non documentés (charge = négatif, réseau > 0 = soutirage), à valider sur l'appareil.
export const CHARGE_SIGN = -1;

export type Flows = {
  pv: number | null;
  grid: number | null;
  bat: number | null;
  house: number | null;
  houseDeduced: boolean;
};

const pvTotal = (s: Snapshot) => {
  const p = s.pv;
  if (!p) return null;
  const v = [p.pv1Power, p.pv2Power, p.pv3Power, p.pv4Power].filter((x) => x != null);
  return v.length ? v.reduce((a, b) => a + b, 0) : null;
};

const deduceHouse = (pv: number | null, grid: number | null, bat: number | null) => (grid != null && bat != null ? Math.max(0, (pv ?? 0) + grid - bat) : null);

/** Série d'historique dont la maison absente ou nulle est déduite comme dans l'onglet État. */
export const withHouse = (p: PowerSeries): PowerSeries => ({
  ...p,
  bat: p.bat.map((b) => (b == null ? null : b * CHARGE_SIGN)),
  load: p.load.map((l, i) => l || deduceHouse(p.pv[i], p.grid[i], p.bat[i] == null ? null : p.bat[i]! * CHARGE_SIGN)),
});

/** Puissances en W : batterie > 0 = charge, réseau > 0 = soutirage. */
export function flows(s: Snapshot | undefined): Flows {
  if (!s) return { pv: null, grid: null, bat: null, house: null, houseDeduced: false };
  const pv = s.es?.pvPower ?? pvTotal(s);
  const bat = s.es?.batPower != null ? s.es.batPower * CHARGE_SIGN : null;
  const grid = s.em?.ctState === 1 ? (s.em.totalPower ?? null) : null;
  const measured = s.es?.offgridPower;
  if (measured) return { pv, grid, bat, house: measured, houseDeduced: false };
  const house = deduceHouse(pv, grid, bat);
  return { pv, grid, bat, house, houseDeduced: house != null };
}

export const fmtW = (w: number | null | undefined) => (w == null ? "—" : `${Math.round(w).toLocaleString("fr-FR")} W`);
export const fmtKwh = (v: number | undefined, divisor: number) => (v == null ? "—" : `${(v / divisor).toLocaleString("fr-FR", { maximumFractionDigits: 2 })} kWh`);
