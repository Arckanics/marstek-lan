import { cancelScan, connect, discover, onBusy, onConnected, onControl, onLog, onScan, onSlotTest, onStatus, type Control, type Device, type ScanProgress, type LinkRequest, type Snapshot, type SlotTest, type SyncState } from "./api";
import { loadSettings, defaults, saveSetting, type Settings } from "./store";
import { CHARGE_SIGN, flows, type Flows } from "./energy";
import { valueAt } from "./curve";
import { limits, powerCap } from "./limits";
import { locale, setLang, t } from "./i18n.svelte";

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
  scanProgress: undefined as ScanProgress | undefined,
  scanError: "",
  logs: [] as string[],
  settings: defaults() as Settings,
  historyRev: 0,
  linkRev: 0,
  ready: false,
});

/** Horloge de l'interface : l'âge des données avance même sans nouveau sondage. */
export const clock = $state({ now: Date.now() });

export const history: (number | null)[][] = [[], [], [], []];

/** Mesures réelles récentes pour l'affichage 1 Hz : [t en s, solaire, batterie, réseau, maison, SOC]. */
const samples: (number | null)[][] = [[], [], [], [], [], []];

export type View = { flows: Flows; last: Flows; soc: number | undefined; lastSoc: number | undefined; lag: number; age: number; t: number; state: "none" | "smooth" | "held" | "waiting" };
/**
 * Valeurs affichées, décalées d'un intervalle de sondage et interpolées chaque seconde entre les deux mesures
 * qui encadrent `t` (jamais d'extrapolation : sans mesure suivante, la dernière valeur est tenue).
 */
export const view: View = $state({ flows: flows(undefined), last: flows(undefined), soc: undefined, lastSoc: undefined, lag: 0, age: 0, t: 0, state: "none" });

function refresh() {
  const xs = samples[0] as number[];
  const s = app.snapshot;
  if (!s || !xs.length) return void (view.state = "none");
  const poll = s.pollSecs || 10;
  const now = Date.now() / 1000;
  const t = now - poll;
  let j = xs.length - 1;
  while (j > 0 && xs[j] > t) j--;
  const at = (c: number) => valueAt(xs, samples[c], t, { gap: 2.5 * poll, tol: 0, hold: true })?.v ?? samples[c][j];
  const tail = (c: number) => samples[c][xs.length - 1];
  const houseDeduced = flows(s).houseDeduced;
  const age = now - xs[xs.length - 1];
  Object.assign(view, {
    flows: { pv: at(1), bat: at(2), grid: at(3), house: at(4), houseDeduced },
    last: { pv: tail(1), bat: tail(2), grid: tail(3), house: tail(4), houseDeduced },
    soc: at(5) ?? undefined,
    lastSoc: tail(5) ?? undefined,
    lag: poll,
    age,
    t,
    state: age > 2.5 * poll ? "waiting" : t > xs[xs.length - 1] ? "held" : "smooth",
  });
}

function pushSample(s: Snapshot) {
  const f = flows(s);
  [s.ts / 1000, f.pv, f.bat, f.grid, f.house, s.bat?.soc ?? s.es?.batSoc ?? null].forEach((v, i) => samples[i].push(v));
  if (samples[0].length > 12) samples.forEach((c) => c.shift());
  refresh();
}

export const LINK_WINDOW_MS = 60 * 60_000;
/**
 * Un sondage vu par chaque Snapshot, sur la dernière heure, avec ses requêtes. `expected` : puissance que le plan fixe,
 * `measured` : puissance batterie réelle, toutes deux en W, charge +.
 */
export type LinkPoint = { ts: number; requests: LinkRequest[]; sync: SyncState; expected: number | null; measured: number | null };
export const linkLog: LinkPoint[] = [];

export function log(line: string) {
  app.logs.push(`${new Date().toLocaleTimeString(locale())}  ${line}`);
  if (app.logs.length > 300) app.logs.splice(0, app.logs.length - 300);
}

/** Puissance max de tous les champs de puissance, réactive. */
export const powerMax = () => powerCap(app.settings.maxPower, limits(app.snapshot));

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
  saveSetting(key, value).catch((e) => log(t("log.saveFailed", { error: errText(e) })));
}

export async function init() {
  await onStatus((s) => {
    // Relecture du mode après une écriture : même sondage, rien à ajouter aux courbes.
    const again = app.snapshot?.ts === s.ts;
    app.snapshot = s;
    if (again) return;
    const fresh = (["es", "em", "pv"] as const).some((k) => s.updated[k] === s.ts);
    linkLog.push({ ts: s.ts, requests: s.requests, sync: s.sync.state, expected: s.sync.expectedW == null ? null : s.sync.expectedW * CHARGE_SIGN, measured: flows(s).bat });
    while (linkLog[0].ts < s.ts - LINK_WINDOW_MS) linkLog.shift();
    app.linkRev++;
    // Un sondage sans lecture fraîche ne doit pas allonger la courbe avec des valeurs périmées.
    if (fresh) {
      pushHistory(s);
      pushSample(s);
    }
  });
  await onLog((l) => {
    log(l);
    if (app.busy) app.progress = l;
  });
  await onScan((p) => (app.scanProgress = p));
  await onBusy((b) => {
    app.busy = b;
    app.progress = "";
  });
  // Le backend a retrouvé la batterie à une autre IP (il a déjà écrit lastIp dans le store).
  await onConnected((ip, port) => {
    app.connectedIp = ip;
    app.settings.lastIp = ip;
    app.settings.lastPort = port;
  });
  setInterval(() => {
    clock.now = Date.now();
    refresh();
  }, 1000);
  await onControl((c) => (app.control = c));
  await onSlotTest((t) => (app.slotTest = t));
  try {
    app.settings = await loadSettings();
    setLang(app.settings.lang);
  } catch (e) {
    log(t("log.settingsNotLoaded", { error: errText(e) }));
  }
}

/** Ports unitaires du réglage (plages ignorées) : les seuls sondés sans clic de l'utilisateur ; vide : 30000. */
const unitPorts = (ports: string) => ports.split(",").map((p) => p.trim()).filter((p) => /^\d+$/.test(p)).join(", ");

/** Nombre de ports d'une saisie comme « 30000, 49152-65535 » (doublons non retirés). */
export const portCount = (ports: string) =>
  ports.split(",").reduce((n, p) => {
    const [a, b] = p.split("-").map((x) => parseInt(x, 10));
    return n + (Number.isNaN(a) ? 0 : Number.isNaN(b) ? 1 : Math.max(0, b - a + 1));
  }, 0);

/**
 * `auto` : recherche en étapes ; sans plage ni ports saisis, c'est aussi le comportement par défaut.
 * `manual` : lancé par l'utilisateur ; sinon (démarrage) seuls les ports unitaires sont sondés et les réglages ne changent pas.
 */
export async function scan(range = "", auto = false, manual = true) {
  app.scanning = true;
  app.scanProgress = undefined;
  app.scanError = "";
  try {
    const ports = manual ? app.settings.ports : unitPorts(app.settings.ports);
    const s = await discover(range || undefined, ports, manual && (auto || (!range && !ports.trim())));
    app.devices = s.devices;
    log(t("log.scanDone", { n: s.devices.length, addresses: s.addresses, sources: s.sources.join(", "), ports: s.ports }));
    if (manual && s.savedPorts) {
      app.settings.ports = s.savedPorts;
      log(t("log.portsAdded", { ports: s.savedPorts }));
    }
  } catch (e) {
    app.scanError = errText(e);
    log(t("log.scanFailed", { error: app.scanError }));
  } finally {
    app.scanning = false;
  }
}

export const stopScan = () => cancelScan().catch((e) => log(t("log.cancelFailed", { error: errText(e) })));

export async function connectTo(ip: string, port = app.settings.lastPort ?? 30000) {
  log(t("log.connecting", { ip, port }));
  try {
    await connect(ip, port);
    app.connectedIp = ip;
    app.snapshot = undefined;
    resetHistory();
    samples.forEach((c) => (c.length = 0));
    linkLog.length = 0;
    setting("lastIp", ip);
    setting("lastPort", port);
    app.tab = "status";
  } catch (e) {
    log(t("log.connectFailed", { error: errText(e) }));
  }
}

export async function start() {
  await init();
  app.ready = true;
  await scan(app.settings.scanRange, false, false);
  const last = app.settings.lastIp;
  if (last) await connectTo(last, app.settings.lastPort);
}

/** Exécute une écriture et journalise l'issue ; renvoie le résultat ou undefined en cas d'erreur. */
export async function write<T>(label: string, call: () => Promise<T>, ok: (r: T) => boolean = (r) => r === true) {
  try {
    const r = await call();
    log(t(ok(r) ? "log.applied" : "log.refused", { label }));
    return r;
  } catch (e) {
    log(t("log.writeError", { label, error: errText(e) }));
  }
}
