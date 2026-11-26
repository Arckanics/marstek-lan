import type { Snapshot } from "./api";

export const DOD_RANGE = [30, 88] as const;
/** Puissance par défaut quand le modèle est inconnu : l'utilisateur l'ajuste. */
export const UNKNOWN_MAX = 800;

type Model = { name: string; max: number | null; slots: number };
const MODELS: [RegExp, Model][] = [
  [/mini/i, { name: "Venus E mini", max: 800, slots: 6 }],
  [/venus\s*d/i, { name: "Venus D", max: 2200, slots: 10 }],
  [/venus\s*e/i, { name: "Venus E", max: 2500, slots: 10 }],
  [/venus\s*a/i, { name: "Venus A", max: 1200, slots: 10 }],
  [/venus\s*c/i, { name: "Venus C", max: null, slots: 10 }],
];

export type Limits = {
  /** Identifiant brut lu (`device.device`). */
  device?: string;
  model?: Model;
  firmware?: string | number;
  rated?: number;
  remaining?: number;
  /** Fonctions SYS (firmware ≥ 150) ; undefined si le firmware n'est pas lu. */
  sys?: boolean;
};

export function limits(s: Snapshot | undefined): Limits {
  const device = s?.device?.device;
  const firmware = s?.device?.ver;
  const v = Number(firmware);
  const model = device ? MODELS.find(([re]) => re.test(device))?.[1] : undefined;
  return {
    device,
    model,
    firmware,
    rated: s?.bat?.ratedCapacity,
    remaining: s?.bat?.batCapacity ?? s?.es?.batCap,
    // Doc Open API §4 : DOD, LED et Bluetooth sur firmware ≥ 150, sauf Venus E mini qui les a toujours.
    sys: model?.slots === 6 ? true : firmware == null || Number.isNaN(v) ? undefined : v >= 150,
  };
}

/** Puissance max utilisable : le réglage de l'utilisateur, jamais au-dessus du max du modèle. */
export const powerCap = (saved: number | undefined, l: Limits) => {
  const modelMax = l.model?.max ?? undefined;
  return Math.min(saved ?? modelMax ?? UNKNOWN_MAX, modelMax ?? Infinity);
};

export const slotCount = (l: Limits) => l.model?.slots ?? 10;
