import { load, type Store } from "@tauri-apps/plugin-store";
import { configChanged, type Notify, type Plan, type Rules, type SlotTest, type Tariff } from "./api";

export type Presets = { charge: { power: number; minutes: number }; discharge: { power: number; minutes: number } };
export type Settings = {
  lastIp?: string;
  scanRange: string;
  tariff: Tariff;
  presets: Presets;
  plans: Plan[];
  sentPlan: Plan | null;
  rules: Rules;
  notify: Notify;
  outsideSlots: { verdict: NonNullable<SlotTest["verdict"]>; ts: number } | null;
};

export const defaults = (): Settings => ({
  scanRange: "",
  tariff: { offpeak: [{ start: "22:00", end: "06:00" }], chargePower: 800, dischargePower: 300, days: 127 },
  presets: { charge: { power: 800, minutes: 60 }, discharge: { power: 300, minutes: 60 } },
  plans: [],
  sentPlan: null,
  rules: { low: { enabled: false, soc: 15, mode: "auto" }, high: { enabled: false, soc: 95, mode: "auto" } },
  notify: { lowSoc: { enabled: true, soc: 15 }, offline: true, modeChanged: false, writeFailed: true },
  outsideSlots: null,
});

const backendKeys: (keyof Settings)[] = ["plans", "rules", "notify", "outsideSlots"];

let store: Promise<Store> | undefined;
const open = () => (store ??= load("settings.json", { defaults: {}, autoSave: 300 }));

export async function loadSettings(): Promise<Settings> {
  const base = defaults();
  const s = await open();
  const saved = await Promise.all(Object.keys(base).map(async (k) => [k, await s.get(k)] as const));
  return { ...base, ...Object.fromEntries(saved.filter(([, v]) => v != null)) };
}

export async function saveSetting<K extends keyof Settings>(key: K, value: Settings[K]) {
  const s = await open();
  await s.set(key, value);
  if (!backendKeys.includes(key)) return;
  await s.save();
  await configChanged();
}

export const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));
