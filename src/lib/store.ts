import { load, type Store } from "@tauri-apps/plugin-store";
import { configChanged, type Notify, type Plan, type Price, type Rules, type SlotTest, type Tariff } from "./api";
import type { Lang } from "./i18n.svelte";
import { migratePlan } from "./plan";

export type Presets = { charge: { power: number; minutes: number }; discharge: { power: number; minutes: number } };
export type Settings = {
  lastIp?: string;
  lastPort?: number;
  scanRange: string;
  /** « 30000, 49152-65535 » ; vide : recherche automatique. */
  ports: string;
  tariff: Tariff;
  presets: Presets;
  plans: Plan[];
  sentPlan: Plan | null;
  rules: Rules;
  notify: Notify;
  outsideSlots: { verdict: NonNullable<SlotTest["verdict"]>; ts: number } | null;
  /** W ; absent = maximum du modèle. */
  maxPower?: number;
  /** Pilotage automatique (règles SOC, hybride, reprise du plan) ; faux ou absent = lecture seule. */
  autoControl: boolean;
  /** Réécrit le plan quand la batterie a quitté le mode Manual ; sans effet si `autoControl` est faux. */
  reapplyPlan: boolean;
  /** Secondes entre deux sondages (30 à 600) ; absent = 240 s. */
  pollSeconds?: number;
  /** Cherche seule la cadence la plus rapide que la batterie supporte (calibration). */
  autoTune: boolean;
  /** Cadence apprise par la calibration, en secondes (écrite par le backend) ; absent = pas encore calibré. */
  tunedPoll?: number;
  /** Planning : dernier jour ouvert (0 = lundi) et jours qui reçoivent le dessin (masque de 7 bits). */
  planDay?: number;
  planTargets?: number;
  /** Appareil du compte cloud suivi quand l'API locale est coupée ; null : pas de secours. */
  cloudDevice: { id: string; name: string } | null;
  /** Rouvrir seul l'API locale via le cloud quand elle est détectée coupée ; vrai par défaut. */
  autoReenable: boolean;
  /** Modèle de compteur (code 0-12) à re-sélectionner via le cloud si le CT est perdu après une coupure ; absent = désactivé. */
  ctReselectMeter?: number;
  /** MAC (12 hexa) du compteur, pour les modèles qui en ont besoin. */
  ctReselectMac?: string;
  /** Tarif par créneaux pour le calcul d'économies. */
  price: Price;
  /** Symbole de monnaie des économies (indépendant de la langue). */
  currency: string;
  /** Langue de l'interface, du backend (notifications, icône de la zone de notification) et du journal affiché. */
  lang: Lang;
};

export const defaults = (): Settings => ({
  // Clés optionnelles listées pour que loadSettings les relise.
  lastIp: undefined,
  lastPort: undefined,
  maxPower: undefined,
  pollSeconds: undefined,
  planDay: undefined,
  planTargets: undefined,
  scanRange: "",
  ports: "",
  tariff: { offpeak: [{ start: "22:00", end: "06:00" }], chargePower: 800, dischargePower: 300, days: 127, chargeSoc: 90, peak: "auto" },
  presets: { charge: { power: 800, minutes: 60 }, discharge: { power: 300, minutes: 60 } },
  plans: [],
  sentPlan: null,
  rules: { low: { enabled: false, soc: 15, mode: "auto" }, high: { enabled: false, soc: 95, mode: "auto" } },
  notify: { lowSoc: { enabled: true, soc: 15 }, offline: true, modeChanged: false, writeFailed: true },
  outsideSlots: null,
  autoControl: false,
  reapplyPlan: false,
  cloudDevice: null,
  autoReenable: true,
  autoTune: false,
  tunedPoll: undefined,
  ctReselectMeter: undefined,
  ctReselectMac: undefined,
  price: { slots: [{ start: "22:00", end: "06:00", price: 0.15 }], default: 0.25, feedIn: null },
  currency: "€",
  lang: "en",
});

const backendKeys: (keyof Settings)[] = ["plans", "rules", "notify", "outsideSlots", "maxPower", "autoControl", "reapplyPlan", "pollSeconds", "cloudDevice", "autoReenable", "autoTune", "tunedPoll", "ctReselectMeter", "ctReselectMac", "lang"];

let store: Promise<Store> | undefined;
const open = () => (store ??= load("settings.json", { defaults: {}, autoSave: 300 }));

export async function loadSettings(): Promise<Settings> {
  const base = defaults();
  const s = await open();
  const saved = await Promise.all(Object.keys(base).map(async (k) => [k, await s.get(k)] as const));
  const merged: Settings = { ...base, ...Object.fromEntries(saved.filter(([, v]) => v != null)) };
  // Réglages écrits avant le mode hybride : champs du tarif absents, blocs en `direction`.
  return {
    ...merged,
    // Avant les plages, les ports étaient un tableau de nombres.
    ports: Array.isArray(merged.ports) ? merged.ports.join(", ") : merged.ports,
    tariff: { ...base.tariff, ...merged.tariff },
    plans: merged.plans.map(migratePlan),
    sentPlan: merged.sentPlan && migratePlan(merged.sentPlan),
  };
}

export async function saveSetting<K extends keyof Settings>(key: K, value: Settings[K]) {
  const s = await open();
  // Le plugin store refuse `set` sans valeur : une valeur absente (réglage optionnel remis à zéro) supprime la clé.
  if (value === undefined) await s.delete(key);
  else await s.set(key, value);
  if (!backendKeys.includes(key)) return;
  await s.save();
  await configChanged();
}

export const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));
