import type { Down, LinkRequest, Outcome, Section, Snapshot } from "./api";
import { t } from "./i18n.svelte";

/** Au-delà, une carte affiche « données d'il y a N s » (avec un sondage de 10 s ; voir `staleAfter`). */
export const STALE_S = 25;
/** Sans aucune lecture réussie depuis ce délai, la batterie est jugée hors ligne. */
const OFFLINE_S = 30;

/** Sondage de base du snapshot ; 10 s tant qu'il est inconnu. */
const pollOf = (s: Snapshot) => s.pollSecs || 10;
/** Entre deux sondages, une lecture a l'âge de l'intervalle : les seuils suivent le sondage choisi. */
export const staleAfter = (s: Snapshot) => Math.max(STALE_S, 2.5 * pollOf(s));

export type Quality = "good" | "fair" | "poor" | "offline";
export const qualityLabel = (q: Quality) => t(`quality.${q}`);

/** Âge en secondes de la plus ancienne des sections ; undefined si aucune n'a encore été lue. */
export function ageOf(s: Snapshot, keys: Section[], now: number) {
  const ts = keys.map((k) => s.updated[k]).filter((t): t is number => t != null);
  return ts.length ? Math.max(0, (now - Math.min(...ts)) / 1000) : undefined;
}

export const fmtAge = (s: number) => (s < 90 ? `${Math.round(s)} s` : `${Math.round(s / 60)} min`);

/** Âge en secondes de la mesure du cloud, prolongé depuis le snapshot ; undefined hors secours. */
export const cloudAge = (s: Snapshot, now: number) => (s.cloud ? s.cloud.ageSecs + Math.max(0, (now - s.ts) / 1000) : undefined);

export function quality(s: Snapshot, now: number): Quality {
  const last = Math.max(0, ...Object.values(s.updated));
  if (!last || (now - last) / 1000 > Math.max(OFFLINE_S, 3 * pollOf(s))) return "offline";
  const { ok, total, rttMs } = s.link;
  const rate = total ? ok / total : 0;
  if (rate >= 0.9 && (rttMs ?? 0) < 400) return "good";
  return rate >= 0.6 ? "fair" : "poor";
}

export const outcomeLabel = (o: Outcome) => t(`outcome.${o}`);

export const downText = (d: Down) => (d.kind === "portClosed" ? t("link.downPort", { port: d.port ?? "", age: fmtAge(d.sinceSecs) }) : t("link.downSilent", { age: fmtAge(d.sinceSecs) }));

/** Les requêtes de chaque sondage à plat, chacune datée (ms) : début du sondage plus le délai des précédentes, +1 ms par rang pour garder des dates strictement croissantes. */
export function spread<T extends { ts: number; requests: LinkRequest[] }>(polls: T[]) {
  return polls.flatMap((poll) => {
    let elapsed = 0;
    return poll.requests.map((r, k) => {
      const x = poll.ts + elapsed + k;
      elapsed += r.rttMs ?? 0;
      return { ...r, x, poll };
    });
  });
}
