import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import type { AverageDay, Block, SocStat, BlockKind, Device, EnergyRow, LinkRequest, ModeReq, Plan, PowerSeries, Section, Slot, Snapshot, Sync, Tariff } from "./api";
import { STEPS, fromCells, type Cell } from "./plan";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const rand = (a: number, b: number) => a + Math.random() * (b - a);
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));

const devices: Device[] = [
  { ip: "192.168.1.42", port: 30000, name: "Venus D", version: "155", wifiMac: "a4:cf:12:8b:3e:01" },
  { ip: "192.168.1.57", port: 30000, name: "Venus E", version: "147", wifiMac: "a4:cf:12:8b:3e:7c" },
];

const on = (mask: number, start: string, end: string, kind: BlockKind, power = 0, socLimit?: number): Block[] =>
  [0, 1, 2, 3, 4, 5, 6].filter((d) => mask & (1 << d)).map((day) => ({ day, start, end, kind, power, ...(socLimit != null && { socLimit }) }));

const week: Plan = {
  name: "Semaine type",
  blocks: [
    ...on(31, "00:00", "06:00", "charge", 800, 90),
    ...on(31, "07:00", "09:00", "discharge", 400, 30),
    ...on(31, "09:00", "17:00", "auto"),
    ...on(31, "17:00", "22:00", "discharge", 600, 20),
    ...on(31, "22:00", "24:00", "charge", 800, 90),
    ...on(96, "00:00", "07:00", "charge", 600),
    ...on(96, "10:00", "18:00", "ai"),
    ...on(96, "18:00", "23:00", "discharge", 500),
  ],
};
const winter: Plan = {
  name: "Hiver, secours",
  blocks: [...on(127, "00:00", "06:00", "charge", 1200), ...on(127, "06:00", "24:00", "ups")],
};

const store = new Map<string, unknown>([
  ["lastIp", "192.168.1.42"],
  ["plans", [week, winter]],
  ["sentPlan", clone(week)],
]);

function compile(plan: Plan): Slot[] {
  const manual = plan.blocks
    .filter((b) => b.kind === "charge" || b.kind === "discharge")
    .sort((a, b) => a.day - b.day || a.start.localeCompare(b.start));
  const merged: Block[] = [];
  for (const b of manual) {
    const last = merged.at(-1);
    if (last && last.day === b.day && last.end === b.start && last.kind === b.kind && last.power === b.power) last.end = b.end;
    else merged.push({ ...b });
  }
  const slots = new Map<string, Slot>();
  for (const b of merged) {
    const key = `${b.start}|${b.end}|${b.kind}|${b.power}`;
    const s = slots.get(key);
    if (s) s.days |= 1 << b.day;
    else slots.set(key, { start: b.start, end: b.end === "24:00" ? "23:59" : b.end, days: 1 << b.day, direction: b.kind as "charge" | "discharge", power: b.power, enabled: true });
  }
  if (slots.size > 10) throw `Le plan demande ${slots.size} créneaux Manuel, la batterie n'en accepte que 10.`;
  return [...slots.values()];
}

function tariffBlocks(t: Tariff): Block[] {
  const idx = (hm: string) => {
    const [h, m] = hm.split(":").map(Number);
    return Math.round((h * 60 + m) / 15);
  };
  const peak: Cell = t.peak === "auto" ? { kind: "auto", power: 0 } : { kind: "discharge", power: t.dischargePower };
  const charge: Cell = { kind: "charge", power: t.chargePower, ...(t.chargeSoc < 100 && { socLimit: t.chargeSoc }) };
  const cells = Array.from({ length: 7 }, (_, d) => Array<Cell>(STEPS).fill(t.days & (1 << d) ? peak : null));
  for (const r of t.offpeak) {
    const a = idx(r.start);
    const b = idx(r.end);
    for (let d = 0; d < 7; d++) {
      if (!(t.days & (1 << d))) continue;
      for (let i = 0; i < STEPS; i++) if (b > a ? i >= a && i < b : i >= a || i < b) cells[d][i] = charge;
    }
  }
  return fromCells(cells);
}

const sim = {
  ip: null as string | null,
  timer: 0,
  tick: 0,
  soc: 64,
  override: undefined as ModeReq | undefined,
  answers: [] as boolean[],
  updated: {} as Partial<Record<Section, number>>,
  autostart: false,
};

function currentBlock(now: Date): Block | undefined {
  const plan = store.get("sentPlan") as Plan | null;
  const day = (now.getDay() + 6) % 7;
  const hm = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  return plan?.blocks.find((b) => b.day === day && b.start <= hm && hm < b.end);
}

const blockText = (b: Block | undefined) =>
  !b ? "hors bloc" : b.kind === "charge" || b.kind === "discharge" ? `${b.kind === "charge" ? "charge" : "décharge"} ${b.power} W` : `mode ${b.kind === "ai" ? "IA" : b.kind === "ups" ? "UPS" : "Auto"}`;

function snapshot(): Snapshot {
  const now = new Date();
  const ts = now.getTime();
  const h = now.getHours() + now.getMinutes() / 60;
  sim.tick++;
  const weak = sim.tick % 90 > 55 && sim.tick % 90 < 72;
  // Panne d'environ 80 s toutes les 5 min.
  const down = sim.tick % 150 >= 100 && sim.tick % 150 < 140;
  // Une panne sur deux est un port fermé (Open API coupée), l'autre un silence.
  const closed = down && Math.floor(sim.tick / 150) % 2 === 1;
  const ok = !down && Math.random() > (weak ? 0.45 : 0.04);
  sim.answers = [...sim.answers, ok].slice(-20);

  const pv = Math.round(Math.max(0, Math.sin(((h - 7) / 13) * Math.PI)) * 1350 * rand(0.8, 1));
  const load = Math.round(rand(220, 380) + (h >= 18 && h < 22 ? 650 : 0) + (Math.random() < 0.1 ? 1800 : 0));
  const block = currentBlock(now);
  const o = sim.override;
  let bat = 0;
  if (o?.kind === "passive") bat = o.direction === "charge" ? o.power : -o.power;
  else if (o?.kind === "auto" || o?.kind === "ai" || block?.kind === "auto" || block?.kind === "ai") bat = Math.max(-2500, Math.min(2500, pv - load));
  else if (block?.kind === "charge") bat = block.socLimit != null && sim.soc >= block.socLimit ? 0 : block.power;
  else if (block?.kind === "discharge") bat = block.socLimit != null && sim.soc <= block.socLimit ? 0 : -block.power;
  bat = Math.round(bat * rand(0.94, 1.02));
  sim.soc = Math.max(12, Math.min(100, sim.soc + bat / 4000));
  const grid = load - pv + bat;

  const phase = sim.tick % 45;
  const sent = store.get("sentPlan") as Plan | null;
  const modeRead = phase >= 35 && !o ? "Auto" : o?.kind === "passive" ? "Passive" : o?.kind === "ai" ? "AI" : o?.kind === "ups" ? "UPS" : o?.kind === "auto" ? "Auto" : "Manual";
  const driven = block && (block.kind === "charge" || block.kind === "discharge");
  const planned = driven ? (block.kind === "charge" ? -block.power : block.power) : null;
  const sync: Sync = { expectedW: null, measuredW: down ? null : -bat, ...(down
    ? { state: "unknown", expected: "", measured: "", reason: `batterie sans réponse depuis ${Math.round((ts - (sim.updated.es ?? ts)) / 1000)} s` }
    : !sent
    ? { state: "unknown", expected: "aucun plan envoyé", measured: "", reason: null }
    : o
      ? { state: "ok", expected: "Action manuelle", measured: `mode ${modeRead}`, reason: null }
      : phase >= 35
        ? { state: "bad", expected: `mode Manual, ${blockText(block)}`, measured: "mode Auto", reason: "mode attendu Manual, lu Auto" }
        : phase >= 25 && block && (block.kind === "charge" || block.kind === "discharge")
          ? { state: "warn", expected: blockText(block), measured: "0 W", expectedW: planned, reason: "puissance nulle alors que le bloc est actif depuis plus de 60 s" }
          : { state: "ok", expected: `mode Manual, ${blockText(block)}`, measured: `mode ${modeRead}, ${Math.abs(bat)} W`, expectedW: planned, reason: null }) };

  // Une requête par lecture du sondage ; en panne, seule la première part.
  const methods = ["Bat.GetStatus", "ES.GetStatus", sim.tick % 5 === 0 ? "EM.GetStatus" : "ES.GetMode"];
  const requests: LinkRequest[] = [];
  for (const method of methods) {
    if (down && requests.length) break;
    const lost = down || (weak && Math.random() < 0.45) || Math.random() < 0.03;
    const refused = !lost && Math.random() < 0.02;
    requests.push(
      lost
        ? { method, outcome: closed ? "portClosed" : "timeout" }
        : { method, rttMs: Math.round(weak ? rand(280, 900) : rand(35, 90) + (Math.random() < 0.08 ? 220 : 0)), outcome: refused ? "refused" : "ok" },
    );
  }

  if (ok) for (const k of ["device", "wifi", "ble", "bat", "es", "mode", "em", "pv"] as Section[]) sim.updated[k] = ts;
  const rssi = Math.round(weak ? rand(-84, -76) : rand(-63, -55));
  return {
    ts,
    updated: { ...sim.updated },
    link: {
      ok: sim.answers.filter(Boolean).length,
      total: sim.answers.length,
      rttMs: ok ? Math.round(weak ? rand(280, 900) : rand(35, 90) + (Math.random() < 0.08 ? 220 : 0)) : undefined,
      rssi,
    },
    requests,
    ...(down ? { down: { kind: closed ? "portClosed" : "silent", port: closed ? 30000 : undefined, sinceSecs: Math.round((ts - (sim.updated.es ?? ts)) / 1000) } } : {}),
    pollSecs: 10,
    sync,
    device: { device: "VenusD", ver: 155, bleMac: "a4:cf:12:8b:3e:02", wifiMac: "a4:cf:12:8b:3e:01", wifiName: "Maison", ip: sim.ip ?? undefined },
    wifi: { ssid: "Maison", rssi, staIp: sim.ip ?? undefined, wifiMac: "a4:cf:12:8b:3e:01" },
    ble: { state: "disconnect", bleMac: "a4:cf:12:8b:3e:02" },
    bat: { soc: Math.round(sim.soc), batTemp: 24, batCapacity: Math.round(5120 * (sim.soc / 100)), ratedCapacity: 5120, chargFlag: true, dischrgFlag: true },
    es: {
      batSoc: Math.round(sim.soc), batCap: 5120, pvPower: pv, ongridPower: -grid, offgridPower: 0, batPower: -bat,
      totalPvEnergy: 184230, totalGridOutputEnergy: 412800, totalGridInputEnergy: 1093400, totalLoadEnergy: 52100,
    },
    mode: { mode: modeRead, batSoc: Math.round(sim.soc) },
    em: { ctState: 1, aPower: grid, bPower: 0, cPower: 0, totalPower: grid, inputEnergy: 10934000, outputEnergy: 4128000 },
    pv: { pv1Power: Math.round(pv * 0.55), pv1Voltage: 38.2, pv1Current: 9.1, pv2Power: pv - Math.round(pv * 0.55), pv2Voltage: 37.6, pv2Current: 8.3 },
  };
}

async function busy<T>(work: () => Promise<T>) {
  await emit("busy", true);
  try {
    return await work();
  } finally {
    await emit("busy", false);
  }
}

function powerSeries(from: number, to: number): PowerSeries {
  const s: PowerSeries = { ts: [], soc: [], pv: [], bat: [], grid: [], load: [], cloud: [] };
  let soc = 40;
  for (let t = from; t < Math.min(to, Date.now()); t += 60_000) {
    const h = new Date(t).getHours() + new Date(t).getMinutes() / 60;
    const pv = Math.max(0, Math.sin(((h - 7) / 13) * Math.PI)) * 1300;
    const load = 300 + (h >= 18 && h < 22 ? 600 : 0) + Math.sin(t / 900_000) * 80;
    const bat = h < 6 ? 800 : h >= 17 && h < 22 ? -600 : Math.max(-800, Math.min(800, pv - load));
    soc = Math.max(15, Math.min(95, soc + bat / 6000));
    s.ts.push(t);
    s.soc.push(Math.round(soc));
    s.pv.push(Math.round(pv));
    s.bat.push(Math.round(-bat));
    s.grid.push(Math.round(load - pv + bat));
    s.load.push(Math.round(load));
    // Simule une courte fenêtre de points cloud (coupure locale) pour voir le marquage pointillé.
    s.cloud.push(h >= 3 && h < 3.5);
  }
  return s;
}

function energyRows(from: number, to: number): EnergyRow[] {
  const rows: EnergyRow[] = [];
  for (let t = from; t < Math.min(to, Date.now()); t += 86_400_000)
    rows.push({ ts: t, pv: Math.round(rand(2500, 7800)), gridIn: Math.round(rand(1500, 5200)), gridOut: Math.round(rand(0, 900)), load: Math.round(rand(6000, 10500)) });
  return rows;
}

function socStats(from: number, to: number, profile: boolean): SocStat[] {
  if (!profile) {
    const rows: SocStat[] = [];
    for (let t = from; t < Math.min(to, Date.now()); t += 86_400_000) {
      const avg = 55 + Math.sin(t / 86_400_000 / 3) * 15;
      rows.push({ ts: t, hour: 0, weekend: false, min: Math.round(avg - 20), max: Math.round(avg + 25), avg: Math.round(avg), n: 1440 });
    }
    return rows;
  }
  return [false, true].flatMap((weekend) =>
    Array.from({ length: 24 }, (_, hour) => {
      const avg = 55 + 30 * Math.sin(((hour - 10) / 24) * 2 * Math.PI) + (weekend ? 5 : 0);
      return { ts: 0, hour, weekend, min: Math.round(avg - 18), max: Math.round(avg + 15), avg: Math.round(avg), n: 100 };
    }),
  );
}

function averageDay(column: string, days: number): AverageDay {
  const avg = Array.from({ length: 96 }, (_, i) => {
    const h = i / 4;
    if (days === 0 && h > new Date().getHours() + new Date().getMinutes() / 60) return null;
    const pv = Math.max(0, Math.sin(((h - 7) / 13) * Math.PI)) * 1300;
    const load = 300 + (h >= 18 && h < 22 ? 600 : 0);
    const k = days === 0 ? 1.1 : 1 - Math.log10(days + 1) * 0.08 + Math.sin(h * 2 + days) * 0.03;
    const bat = h < 6 ? 800 : h >= 17 && h < 22 ? -600 : Math.max(-800, Math.min(800, pv - load));
    const v = { pv, load, bat: -bat, grid: load - pv + bat, soc: 50 + 30 * Math.sin(((h - 10) / 24) * 2 * Math.PI) }[column] ?? 0;
    return Math.round(column === "soc" ? Math.max(5, Math.min(100, v * k)) : v * k);
  });
  return { days: days === 0 ? 1 : days > 90 ? 40 : days - 3, avg };
}

async function slotTest() {
  const samples: { ts: number; batPower: number | null }[] = [];
  for (let i = 0; i < 24; i++) {
    await wait(500);
    samples.push({ ts: Date.now(), batPower: i < 10 ? -200 + rand(-8, 8) : rand(-6, 6) });
    await emit("slot_test", { samples: [...samples], verdict: null });
  }
  store.set("outsideSlots", { verdict: "idle", ts: Date.now() });
  await emit("slot_test", { samples, verdict: "idle" });
}

async function handle(cmd: string, args: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    case "discover":
      await wait(900);
      return { devices, addresses: 254, sources: ["Wi-Fi (192.168.1.20/24)"], ports: args.ports || "30000", savedPorts: null };
    case "connect":
      clearInterval(sim.timer);
      sim.ip = args.ip as string;
      sim.timer = window.setInterval(() => emit("status", snapshot()), 2000);
      setTimeout(() => emit("status", snapshot()), 300);
      await emit("control", { source: "plan", detail: `plan « ${(store.get("sentPlan") as Plan | null)?.name ?? "aucun"} »` });
      return null;
    case "refresh":
      return null;
    case "set_mode": {
      const mode = args.mode as ModeReq;
      return busy(async () => {
        await wait(400);
        sim.override = mode;
        await emit("control", { source: "manual", detail: mode.kind === "passive" ? `passif ${mode.power} W` : `mode ${mode.kind}` });
        return true;
      });
    }
    case "compile_plan":
      return compile(args.plan as Plan);
    case "apply_plan": {
      const plan = args.plan as Plan;
      const slots = compile(plan);
      return busy(async () => {
        for (let i = 0; i < 10; i++) {
          await wait(150);
          await emit("log", `Créneau ${i} écrit${slots[i] ? "" : " (désactivé)"}.`);
        }
        store.set("sentPlan", clone(plan));
        sim.override = undefined;
        await emit("control", { source: "plan", detail: `plan « ${plan.name} »` });
        return Array(10).fill(true);
      });
    }
    case "tariff_plan":
      return tariffBlocks(args.tariff as Tariff);
    case "config_changed":
      return null;
    case "history_power":
      return powerSeries(args.from as number, args.to as number);
    case "history_energy":
      return energyRows(args.from as number, args.to as number);
    case "history_soc":
      return socStats(args.from as number, args.to as number, args.profile as boolean);
    case "history_average":
      return averageDay(args.column as string, args.days as number);
    case "history_savings": {
      const ps = powerSeries(args.from as number, args.to as number);
      const price = args.price as { slots: { start: string; end: string; price: number }[]; default: number; feedIn: number | null };
      const hhmm = (s: string) => { const [h, m] = s.split(":").map(Number); return h * 60 + m; };
      const at = (ts: number) => {
        const min = new Date(ts).getHours() * 60 + new Date(ts).getMinutes();
        for (const s of price.slots) { const a = hhmm(s.start), b = hhmm(s.end); if (a <= b ? min >= a && min < b : min >= a || min < b) return s.price; }
        return price.default;
      };
      let baseline = 0, cost = 0, imported = 0, exported = 0, selfUsed = 0;
      for (let i = 1; i < ps.ts.length; i++) {
        const h = Math.min(ps.ts[i] - ps.ts[i - 1], 300_000) / 3_600_000, p = at(ps.ts[i - 1]);
        const load = (Math.max(0, ps.load[i - 1] ?? 0) / 1000) * h, g = ps.grid[i - 1] ?? 0;
        const imp = (Math.max(0, g) / 1000) * h, exp = (Math.max(0, -g) / 1000) * h;
        baseline += load * p; cost += imp * p - (price.feedIn ?? 0) * exp;
        imported += imp; exported += exp; selfUsed += Math.max(0, load - imp);
      }
      return { baseline, cost, saved: baseline - cost, imported, exported, selfUsed };
    }
    case "export_csv":
      await wait(300);
      return "C:\\Users\\demo\\Downloads\\marstek-historique.csv";
    case "run_slot_test":
      slotTest();
      return null;
    case "set_dod":
    case "set_led":
    case "set_ble":
      return busy(async () => (await wait(300), true));
    case "plugin:store|load":
      return 1;
    case "plugin:store|get":
      return [store.get(args.key as string) ?? null, store.has(args.key as string)];
    case "plugin:store|set":
      store.set(args.key as string, args.value);
      return null;
    case "plugin:autostart|is_enabled":
      return sim.autostart;
    case "plugin:autostart|enable":
    case "plugin:autostart|disable":
      sim.autostart = cmd === "plugin:autostart|enable";
      return null;
    default:
      return null;
  }
}

/** Batterie simulée pour `npm run dev` hors de Tauri. */
export function installMock() {
  mockIPC((cmd, args) => handle(cmd, (args ?? {}) as Record<string, unknown>), { shouldMockEvents: true });
}
