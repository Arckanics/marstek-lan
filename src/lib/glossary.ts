import type { BlockKind, SlotTest } from "./api";
import { t, type Key } from "./i18n.svelte";

export type ModeId = "auto" | "ai" | "ups" | "passive" | "manual" | "charge" | "discharge" | "hybrid";
export type Fiche = { name: string; tone: string; does: string; when: string; pcOff: string; limits: string };

export const FICHE_ROWS: [keyof Omit<Fiche, "name" | "tone">, Key][] = [
  ["does", "fiche.does"],
  ["when", "fiche.when"],
  ["pcOff", "fiche.pcOff"],
  ["limits", "fiche.limits"],
];

const TONES: Record<ModeId, string> = {
  auto: "k-auto",
  ai: "k-ai",
  ups: "k-ups",
  passive: "k-none",
  manual: "k-none",
  charge: "k-charge",
  discharge: "k-discharge",
  hybrid: "k-auto",
};

/** Fiche d'un mode dans la langue courante (textes dans `locales`, clés `fiche.<mode>.<champ>`). */
export const fiche = (id: ModeId): Fiche => {
  const f = (k: string) => t(`fiche.${id}.${k}` as Key);
  return { name: f("name"), tone: TONES[id], does: f("does"), when: f("when"), pcOff: f("pcOff"), limits: f("limits") };
};

export type HelpId =
  | "dod" | "socLimit" | "rules" | "power" | "maxPower" | "slotTest" | "presets" | "tariff" | "sign"
  | "applyTo" | "slots" | "scan" | "notify" | "autostart" | "sync" | "led" | "ble" | "poll" | "autoControl" | "reapplyPlan";

/** Conséquence d'un bloc du planning selon qui l'exécute. */
export function consequence(kind: BlockKind, socLimit: number | undefined, verdict: SlotTest["verdict"] | undefined): string {
  if (kind === "charge" || kind === "discharge")
    return socLimit == null
      ? t("consequence.written")
      : t(kind === "charge" ? "consequence.cap" : "consequence.floor", { soc: socLimit });
  return t(verdict === "follows" ? "consequence.hole" : "consequence.app");
}
