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
export type Tariff = { offpeak: { start: string; end: string }[]; chargePower: number; dischargePower: number; days: number };

export type Block = { day: number; start: string; end: string; direction: Direction; power: number };
export type Plan = { name: string; blocks: Block[] };
export type ModeKind = "auto" | "ai" | "ups";
export type Rule = { enabled: boolean; soc: number; mode: ModeKind };
export type Rules = { low: Rule; high: Rule };
export type Notify = { lowSoc: { enabled: boolean; soc: number }; offline: boolean; modeChanged: boolean; writeFailed: boolean };
export type PowerSeries = { ts: number[]; soc: (number | null)[]; pv: (number | null)[]; bat: (number | null)[]; grid: (number | null)[]; load: (number | null)[] };
export type EnergyRow = { ts: number; pv: number | null; gridIn: number | null; gridOut: number | null; load: number | null };
export type Region = "eu" | "us";
export type CloudDevice = {
  devid: string; name: string | null; type: string | null; version: string | null; sn: string | null;
  soc: number | null; charge: number | null; discharge: number | null; load: number | null; profit: number | null;
  capacityKwh: number | null; status: string | null;
};
export type SlotTest = { samples: { ts: number; batPower: number | null }[]; verdict: "idle" | "follows" | null };
export type Control = { source: "plan" | "rule" | "manual"; detail: string };

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
export type Snapshot = { ts: number; device?: DeviceInfo; wifi?: Wifi; ble?: Ble; bat?: Bat; es?: Es; mode?: Mode; em?: Em; pv?: Pv };

export const discover = (range?: string) => invoke<Device[]>("discover", { range });
export const connect = (ip: string) => invoke<void>("connect", { ip });
export const refresh = () => invoke<void>("refresh");
export const setMode = (mode: ModeReq) => invoke<boolean>("set_mode", { mode });
export const compilePlan = (plan: Plan) => invoke<Slot[]>("compile_plan", { plan });
export const applyPlan = (plan: Plan) => invoke<boolean[]>("apply_plan", { plan });
export const tariffPlan = (tariff: Tariff) => invoke<Block[]>("tariff_plan", { tariff });
export const configChanged = () => invoke<void>("config_changed");
export const historyPower = (from: number, to: number) => invoke<PowerSeries>("history_power", { from, to });
export const historyEnergy = (from: number, to: number, bucket: "hour" | "day" | "month") => invoke<EnergyRow[]>("history_energy", { from, to, bucket });
export const exportCsv = (from: number, to: number) => invoke<string>("export_csv", { from, to });
export const runSlotTest = () => invoke<void>("run_slot_test");
export const cloudLogin = (email: string, password: string, region: Region) => invoke<void>("cloud_login", { email, password, region });
export const cloudDevices = () => invoke<CloudDevice[]>("cloud_devices");
export const cloudLogout = () => invoke<void>("cloud_logout");
export const cloudStatus = () => invoke<{ email: string; region: string } | null>("cloud_status");
export const setDod = (value: number) => invoke<boolean>("set_dod", { value });
export const setLed = (on: boolean) => invoke<boolean>("set_led", { on });
export const setBle = (on: boolean) => invoke<boolean>("set_ble", { on });

export const onStatus = (fn: (s: Snapshot) => void): Promise<UnlistenFn> => listen<Snapshot>("status", (e) => fn(e.payload));
export const onLog = (fn: (line: string) => void): Promise<UnlistenFn> => listen<string>("log", (e) => fn(e.payload));
export const onBusy = (fn: (busy: boolean) => void): Promise<UnlistenFn> => listen<boolean>("busy", (e) => fn(e.payload));
export const onSlotTest = (fn: (t: SlotTest) => void): Promise<UnlistenFn> => listen<SlotTest>("slot_test", (e) => fn(e.payload));
export const onControl = (fn: (c: Control) => void): Promise<UnlistenFn> => listen<Control>("control", (e) => fn(e.payload));
