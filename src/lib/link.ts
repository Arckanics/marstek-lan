import type { Section, Snapshot } from "./api";

/** Au-delà, une carte affiche « données d'il y a N s ». */
export const STALE_S = 25;
/** Sans aucune lecture réussie depuis ce délai, la batterie est jugée hors ligne. */
const OFFLINE_S = 30;

export type Quality = "good" | "fair" | "poor" | "offline";
export const QUALITY_LABEL: Record<Quality, string> = { good: "bonne", fair: "moyenne", poor: "faible", offline: "hors ligne" };

/** Âge en secondes de la plus ancienne des sections ; undefined si aucune n'a encore été lue. */
export function ageOf(s: Snapshot, keys: Section[], now: number) {
  const ts = keys.map((k) => s.updated[k]).filter((t): t is number => t != null);
  return ts.length ? Math.max(0, (now - Math.min(...ts)) / 1000) : undefined;
}

export const fmtAge = (s: number) => (s < 90 ? `${Math.round(s)} s` : `${Math.round(s / 60)} min`);

export function quality(s: Snapshot, now: number): Quality {
  const last = Math.max(0, ...Object.values(s.updated));
  if (!last || (now - last) / 1000 > OFFLINE_S) return "offline";
  const { ok, total, rttMs } = s.link;
  const rate = total ? ok / total : 0;
  if (rate >= 0.9 && (rttMs ?? 0) < 400) return "good";
  return rate >= 0.6 ? "fair" : "poor";
}

export function linkTip({ link }: Snapshot) {
  const parts = [`Réponses : ${link.ok} sur ${link.total}`];
  if (link.rttMs != null) parts.push(`délai médian ${link.rttMs} ms`);
  if (link.rssi != null) parts.push(`Wi-Fi de la batterie ${link.rssi} dBm`);
  return parts.join(" · ");
}
