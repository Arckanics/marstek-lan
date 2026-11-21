import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Device = { ip: string; port: number; name: string | null; version: string | null; wifiMac: string | null };
export type Direction = "charge" | "discharge";
export type ModeReq =
  | { kind: "auto" }
  | { kind: "ai" }
  | { kind: "ups" }
  | { kind: "passive"; direction: Direction; power: number; seconds: number };
export type Slot = { start: string; end: string; days: number; direction: Direction; power: number; enabled: boolean };
export type Tariff = {
  offpeak: { start: string; end: string }[]; chargePower: number; dischargePower: number; days: number;
  /** SOC visé en heures creuses ; 100 = pas de limite. */
  chargeSoc: number;
  /** Heures pleines : bloc auto, ou décharge à `dischargePower`. */
  peak: "auto" | "discharge";
};

export type BlockKind = "charge" | "discharge" | "auto" | "ai" | "ups";
/** charge/discharge : créneau Manuel de `power` W ; auto/ai/ups : `power` vaut 0. `socLimit` : cible de SOC (charge) ou plancher (décharge). */
export type Block = { day: number; start: string; end: string; kind: BlockKind; power: number; socLimit?: number };
export type Plan = { name: string; blocks: Block[] };
export type ModeKind = "auto" | "ai" | "ups";
export type Rule = { enabled: boolean; soc: number; mode: ModeKind };
export type Rules = { low: Rule; high: Rule };
export type Notify = { lowSoc: { enabled: boolean; soc: number }; offline: boolean; modeChanged: boolean; writeFailed: boolean };
export type PowerSeries = { ts: number[]; soc: (number | null)[]; pv: (number | null)[]; bat: (number | null)[]; grid: (number | null)[]; load: (number | null)[]; cloud: boolean[] };
/** SOC agrégé : par jour local (`ts`) ou par heure du jour (`hour`, jours ouvrés ou week-end). */
export type SocStat = { ts: number; hour: number; weekend: boolean; min: number; max: number; avg: number; n: number };
/** Journée moyenne : valeur par quart d'heure de l'heure locale (null sans mesure) et jours distincts couverts. */
export type AverageDay = { days: number; avg: (number | null)[] };
export type AverageColumn = "soc" | "pv" | "bat" | "grid" | "load";
export type EnergyRow = { ts: number; pv: number | null; gridIn: number | null; gridOut: number | null; load: number | null };
export type SlotTest = { samples: { ts: number; batPower: number | null }[]; verdict: "idle" | "follows" | null };
export type Control = { source: "plan" | "rule" | "manual" | "hybrid"; detail: string };

export type DeviceInfo = { device?: string; ver?: string | number; bleMac?: string; wifiMac?: string; wifiName?: string; ip?: string };
export type Wifi = { ssid?: string; rssi?: number; staIp?: string; wifiMac?: string };
export type Ble = { state?: string; bleMac?: string };
export type Bat = { soc?: number; batTemp?: number; batCapacity?: number; ratedCapacity?: number; chargFlag?: boolean; dischrgFlag?: boolean };
export type Es = {
  batSoc?: number;
  batCap?: number;
  pvPower?: number;
  ongridPower?: number;
  offgridPower?: number;
  batPower?: number;
  totalPvEnergy?: number;
  totalGridOutputEnergy?: number;
  totalGridInputEnergy?: number;
  totalLoadEnergy?: number;
};
export type Mode = { mode?: string; ongridPower?: number; offgridPower?: number; batSoc?: number };
export type Em = { ctState?: number; aPower?: number; bPower?: number; cPower?: number; totalPower?: number; inputEnergy?: number; outputEnergy?: number };
export type Pv = {
  pv1Power?: number; pv1Voltage?: number; pv1Current?: number;
  pv2Power?: number; pv2Voltage?: number; pv2Current?: number;
  pv3Power?: number; pv3Voltage?: number; pv3Current?: number;
  pv4Power?: number; pv4Voltage?: number; pv4Current?: number;
};
export type Section = "device" | "wifi" | "ble" | "bat" | "es" | "mode" | "em" | "pv";
/** ok réponses sur total pour les 20 dernières requêtes ; rttMs = médiane ; rssi = Wi-Fi vu par la batterie. */
export type Link = { ok: number; total: number; rttMs?: number; rssi?: number };
export type SyncState = "ok" | "warn" | "bad" | "unknown";
/** `expectedW` : puissance batterie que le plan fixe ; `measuredW` : puissance lue. Signées comme `es.batPower` (voir `CHARGE_SIGN`) ; null quand elle ne se juge pas ou manque. */
export type Sync = { state: SyncState; expected: string; measured: string; expectedW: number | null; measuredW: number | null; reason: string | null };
/** Résultat d'une requête : refusée = la batterie a répondu par une erreur. */
export type Outcome = "ok" | "refused" | "timeout" | "portClosed";
export type LinkRequest = { method: string; rttMs?: number; outcome: Outcome };
/** Batterie injoignable : `portClosed` (l'IP répond, rien n'écoute sur `port`) ou `silent`, depuis `sinceSecs`. */
export type Down = { kind: "portClosed" | "silent"; port?: number; sinceSecs: number };
export type Region = "eu" | "us";
/** Appareil d'un compte Marstek cloud (lecture seule). */
export type CloudDevice = {
  devid: string; name: string | null; type: string | null; sn: string | null;
  soc: number | null; charge: number | null; discharge: number | null; load: number | null; pv: number | null; reportTime: number | null;
};
/** Valeurs du cloud pendant une panne de l'API locale ; `ageSecs` à la date du snapshot. */
export type CloudReading = { soc: number | null; chargeW: number | null; dischargeW: number | null; loadW: number | null; pvW: number | null; reportTime: number; ageSecs: number };
export type Snapshot = {
  ts: number;
  /** Date (ms) de la dernière lecture réussie de chaque section : une lecture ratée garde l'ancienne valeur. */
  updated: Partial<Record<Section, number>>;
  link: Link;
  /** Requêtes du dernier sondage, dans l'ordre d'envoi. */
  requests: LinkRequest[];
  /** Présent tant que la batterie ne répond pas. */
  down?: Down;
  /** Durée réelle d'un cycle de sondage, en secondes. */
  pollSecs: number;
  sync: Sync;
  /** Présent seulement pendant une panne de l'API locale, avec un compte cloud et un appareil choisis. */
  cloud?: CloudReading;
  device?: DeviceInfo; wifi?: Wifi; ble?: Ble; bat?: Bat; es?: Es; mode?: Mode; em?: Em; pv?: Pv;
};

export type Scan = { devices: Device[]; addresses: number; sources: string[]; ports: string; savedPorts: string | null };
export type ScanProgress = { stage: string; sent: number; total: number; found: number };
export const discover = (range: string | undefined, ports: string, auto: boolean) => invoke<Scan>("discover", { range, ports, auto });
export const cancelScan = () => invoke<void>("cancel_scan");
export const connect = (ip: string, port: number) => invoke<void>("connect", { ip, port });
export const refresh = () => invoke<void>("refresh");
export const setMode = (mode: ModeReq) => invoke<boolean>("set_mode", { mode });
export const compilePlan = (plan: Plan) => invoke<Slot[]>("compile_plan", { plan });
export const applyPlan = (plan: Plan) => invoke<boolean[]>("apply_plan", { plan });
export const tariffPlan = (tariff: Tariff) => invoke<Block[]>("tariff_plan", { tariff });
export const configChanged = () => invoke<void>("config_changed");
export type PriceSlot = { start: string; end: string; price: number };
/** Tarif par créneaux pour les économies ; `default` hors créneau, `feedIn` = prix de revente (null : export non valorisé). */
export type Price = { slots: PriceSlot[]; default: number; feedIn: number | null };
/** Économies sur une période, dans la monnaie de l'utilisateur ; énergies en kWh. */
export type Savings = { baseline: number; cost: number; saved: number; imported: number; exported: number; selfUsed: number };
export const historyPower = (from: number, to: number) => invoke<PowerSeries>("history_power", { from, to });
export const historySavings = (from: number, to: number, price: Price) => invoke<Savings>("history_savings", { from, to, price });
export const historyEnergy = (from: number, to: number, bucket: "hour" | "day" | "month") => invoke<EnergyRow[]>("history_energy", { from, to, bucket });
export const historySoc = (from: number, to: number, profile: boolean) => invoke<SocStat[]>("history_soc", { from, to, profile });
/** `days` jours avant aujourd'hui (aujourd'hui exclu) ; 0 : aujourd'hui jusqu'à maintenant. */
export const historyAverage = (column: AverageColumn, days: number, weekend: boolean | null) => invoke<AverageDay>("history_average", { column, days, weekend });
export const exportCsv = (from: number, to: number) => invoke<string>("export_csv", { from, to });
export const runSlotTest = () => invoke<void>("run_slot_test");
export const cloudLogin = (email: string, password: string, region: Region) => invoke<void>("cloud_login", { email, password, region });
export const cloudDevices = () => invoke<CloudDevice[]>("cloud_devices");
export const cloudLogout = () => invoke<void>("cloud_logout");
export const cloudStatus = () => invoke<{ email: string; region: Region; expired: boolean } | null>("cloud_status");
/** Rouvre l'Open API locale de l'appareil via le cloud (seule voie quand l'API locale est coupée). */
export const reselectCt = (devid: string, meter: number, mac?: string) => invoke<string>("cloud_reselect_ct", { devid, meter, mac });
export const cloudEnableLocalApi = (devid: string) => invoke<void>("cloud_enable_local_api", { devid });
export const setDod = (value: number) => invoke<boolean>("set_dod", { value });
export const setLed = (on: boolean) => invoke<boolean>("set_led", { on });
export const setBle = (on: boolean) => invoke<boolean>("set_ble", { on });

export const onStatus = (fn: (s: Snapshot) => void): Promise<UnlistenFn> => listen<Snapshot>("status", (e) => fn(e.payload));
export const onScan = (fn: (p: ScanProgress) => void): Promise<UnlistenFn> => listen<ScanProgress>("scan", (e) => fn(e.payload));
export const onLog = (fn: (line: string) => void): Promise<UnlistenFn> => listen<string>("log", (e) => fn(e.payload));
export const onBusy = (fn: (busy: boolean) => void): Promise<UnlistenFn> => listen<boolean>("busy", (e) => fn(e.payload));
export const onSlotTest = (fn: (t: SlotTest) => void): Promise<UnlistenFn> => listen<SlotTest>("slot_test", (e) => fn(e.payload));
export const onConnected = (fn: (ip: string, port: number) => void): Promise<UnlistenFn> =>
  listen<{ ip: string; port: number }>("connected", (e) => fn(e.payload.ip, e.payload.port));
export const onControl = (fn: (c: Control) => void): Promise<UnlistenFn> => listen<Control>("control", (e) => fn(e.payload));
