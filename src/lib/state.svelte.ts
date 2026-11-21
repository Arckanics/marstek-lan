import { connect, discover, onBusy, onConnected, onControl, onLog, onSlotTest, onStatus, type Control, type Device, type Snapshot, type SlotTest } from "./api";
import { loadSettings, defaults, saveSetting, type Settings } from "./store";
import { flows } from "./energy";

export const HISTORY_POINTS = 360;

export const app = $state({
  tab: "devices" as "devices" | "status" | "modes" | "planning" | "history" | "device" | "settings",
  snapshot: undefined as Snapshot | undefined,
  busy: false,
  /** Dernier message du backend pendant une écriture (nouvelle tentative, etc.). */
  progress: "",
  control: undefined as Control | undefined,
  slotTest: undefined as SlotTest | undefined,
  connectedIp: null as string | null,
  devices: [] as Device[],
  scanning: false,
  logs: [] as string[],
  settings: defaults() as Settings,
  historyRev: 0,
  ready: false,
});

/** Horloge de l'interface : l'âge des données avance même sans nouveau sondage. */
export const clock = $state({ now: Date.now() });

export const history: (number | null)[][] = [[], [], [], []];

export function log(line: string) {
  app.logs.push(`${new Date().toLocaleTimeString("fr-FR")}  ${line}`);
  if (app.logs.length > 300) app.logs.splice(0, app.logs.length - 300);
}

export const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

function pushHistory(s: Snapshot) {
  const f = flows(s);
  const t = Math.floor(s.ts / 1000);
  const row = [t, f.pv, f.bat, f.grid];
  history.forEach((col, i) => col.push(row[i]));
  while (history[0].length > HISTORY_POINTS) history.forEach((col) => col.shift());
  app.historyRev++;
}

export function resetHistory() {
  history.forEach((col) => (col.length = 0));
  app.historyRev++;
}

export function setting<K extends keyof Settings>(key: K, value: Settings[K]) {
  app.settings[key] = value;
  saveSetting(key, value).catch((e) => log(`Sauvegarde impossible : ${errText(e)}`));
}

export async function init() {
  await onStatus((s) => {
    app.snapshot = s;
    // Un sondage sans lecture fraîche ne doit pas allonger la courbe avec des valeurs périmées.
    if ((["es", "em", "pv"] as const).some((k) => s.updated[k] === s.ts)) pushHistory(s);
  });
  await onLog((l) => {
    log(l);
    if (app.busy) app.progress = l;
  });
  await onBusy((b) => {
    app.busy = b;
    app.progress = "";
  });
  // Le backend a retrouvé la batterie à une autre IP (il a déjà écrit lastIp dans le store).
  await onConnected((ip) => {
    app.connectedIp = ip;
    app.settings.lastIp = ip;
  });
  setInterval(() => (clock.now = Date.now()), 1000);
  await onControl((c) => (app.control = c));
  await onSlotTest((t) => (app.slotTest = t));
  try {
    app.settings = await loadSettings();
  } catch (e) {
    log(`Réglages non chargés : ${errText(e)}`);
  }
}

export async function scan(range?: string) {
  app.scanning = true;
  try {
    app.devices = await discover(range || undefined);
    log(`Balayage terminé : ${app.devices.length} appareil(s).`);
  } catch (e) {
    log(`Balayage impossible : ${errText(e)}`);
  } finally {
    app.scanning = false;
  }
}

export async function connectTo(ip: string) {
  log(`Connexion à ${ip}…`);
  try {
    await connect(ip);
    app.connectedIp = ip;
    app.snapshot = undefined;
    resetHistory();
    setting("lastIp", ip);
    app.tab = "status";
  } catch (e) {
    log(`Connexion impossible : ${errText(e)}`);
  }
}

export async function start() {
  await init();
  app.ready = true;
  await scan(app.settings.scanRange);
  const last = app.settings.lastIp;
  if (last) await connectTo(last);
}

/** Exécute une écriture et journalise l'issue ; renvoie le résultat ou undefined en cas d'erreur. */
export async function write<T>(label: string, call: () => Promise<T>, ok: (r: T) => boolean = (r) => r === true) {
  try {
    const r = await call();
    log(`${label} : ${ok(r) ? "appliqué" : "refusé par l'appareil"}.`);
    return r;
  } catch (e) {
    log(`${label} : ${errText(e)}`);
  }
}
