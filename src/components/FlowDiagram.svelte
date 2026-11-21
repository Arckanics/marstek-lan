<script lang="ts">
  import { fmtW, type Flows } from "../lib/energy";
  import { t } from "../lib/i18n.svelte";

  let { f }: { f: Flows } = $props();

  const MIN_W = 10;
  const hub = { x: 320, y: 150 };
  const width = (p: number) => 3 + 9 * Math.min(1, Math.sqrt(Math.abs(p) / 2500));
  const on = (p: number | null) => p != null && Math.abs(p) >= MIN_W;

  type Seg = { d: string; p: number; color: string; name: string };
  const segs = $derived<Seg[]>([
    { name: t("flowName.pv"), p: f.pv ?? 0, color: "var(--solar)", d: `M320 74 V${hub.y - 16}` },
    {
      name: t("flowName.grid"),
      p: f.grid ?? 0,
      color: "var(--grid)",
      d: (f.grid ?? 0) >= 0 ? `M152 150 H${hub.x - 16}` : `M${hub.x - 16} 150 H152`,
    },
    {
      name: t("flowName.bat"),
      p: f.bat ?? 0,
      color: (f.bat ?? 0) >= 0 ? "var(--charge)" : "var(--discharge)",
      d: (f.bat ?? 0) >= 0 ? `M320 ${hub.y + 16} V226` : `M320 226 V${hub.y + 16}`,
    },
    { name: t("flowName.house"), p: f.house ?? 0, color: "var(--house)", d: `M${hub.x + 16} 150 H488` },
  ]);

  const summary = $derived(
    segs.filter((s) => on(s.p)).map((s) => `${s.name} ${fmtW(Math.abs(s.p))}`).join(", ") || t("flow.none"),
  );
  const batState = $derived(!on(f.bat) ? t("flow.standby") : f.bat! > 0 ? t("dir.charge") : t("dir.discharge"));
  const gridState = $derived(f.grid == null ? t("flow.noMeter") : !on(f.grid) ? t("flow.balanced") : f.grid > 0 ? t("flow.import") : t("flow.export"));
</script>

<svg viewBox="0 0 640 300" role="img" aria-label={t("flow.aria", { summary })}>
  <defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="10" refY="5" markerWidth="2.2" markerHeight="2.2" orient="auto">
      <path d="M0 0 10 5 0 10z" fill="context-stroke" />
    </marker>
  </defs>

  {#each segs as s}
    {#if on(s.p)}
      <path d={s.d} class="flow" stroke={s.color} stroke-width={width(s.p)} style:color={s.color} marker-end="url(#arrow)" />
    {:else}
      <path d={s.d} class="idle" />
    {/if}
  {/each}

  <circle cx={hub.x} cy={hub.y} r="16" class="hub" />
  <path d="M322 141 314 152h6l-2 9 8-11h-6z" class="bolt" />

  <g transform="translate(250 14)" class="node k-solar" class:live={on(f.pv)}>
    <rect width="140" height="60" rx="10" />
    <g class="ico" style="stroke: var(--solar)" transform="translate(10 18)">
      <circle cx="12" cy="12" r="4" /><path d="M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8" />
    </g>
    <text x="44" y="22" class="lbl">{t("flow.pv")}</text>
    <text x="44" y="41" class="val">{fmtW(f.pv)}</text>
    <text x="44" y="54" class="sub">{on(f.pv) ? t("flow.production") : t("flow.idle")}</text>
  </g>

  <g transform="translate(10 120)" class="node k-grid" class:live={on(f.grid)}>
    <rect width="140" height="60" rx="10" />
    <g class="ico" style="stroke: var(--grid)" transform="translate(10 18)">
      <path d="M12 2 7 22M12 2l5 20M8.5 9h7M7.3 15h9.4M5 6h14" />
    </g>
    <text x="44" y="22" class="lbl">{t("flow.grid")}</text>
    <text x="44" y="41" class="val">{fmtW(f.grid == null ? null : Math.abs(f.grid))}</text>
    <text x="44" y="54" class="sub">{gridState}</text>
  </g>

  <g transform="translate(490 120)" class="node k-house" class:live={on(f.house)}>
    <rect width="140" height="60" rx="10" />
    <g class="ico" style="stroke: var(--house)" transform="translate(10 18)">
      <path d="M3 11.5 12 4l9 7.5M5 10v10h14V10" />
    </g>
    <text x="44" y="22" class="lbl">{t("flow.house")}</text>
    <text x="44" y="41" class="val">{fmtW(f.house)}</text>
    <text x="44" y="54" class="sub">{f.house == null ? t("flow.undetermined") : f.houseDeduced ? t("flow.deduced") : t("flow.measured")}</text>
  </g>

  <g transform="translate(250 226)" class="node {(f.bat ?? 0) < 0 ? 'k-discharge' : 'k-charge'}" class:live={on(f.bat)}>
    <rect width="140" height="60" rx="10" />
    <g class="ico" style="stroke: {(f.bat ?? 0) < 0 ? 'var(--discharge)' : 'var(--charge)'}" transform="translate(10 18)">
      <rect x="2" y="7" width="17" height="10" rx="2" /><path d="M22 10.5v3M6 10.5v3M10 10.5v3" />
    </g>
    <text x="44" y="22" class="lbl">{t("flow.bat")}</text>
    <text x="44" y="41" class="val">{fmtW(f.bat == null ? null : Math.abs(f.bat))}</text>
    <text x="44" y="54" class="sub">{batState}</text>
  </g>
</svg>

<style>
  svg { width: 100%; height: auto; display: block; }
  .flow { fill: none; stroke-dasharray: 10 12; animation: flow 1s linear infinite; filter: drop-shadow(0 0 3px color-mix(in oklab, currentColor var(--glow), transparent)); }
  .idle { fill: none; stroke: var(--border); stroke-width: 2; }
  .hub { fill: var(--surface); stroke: var(--border); stroke-width: 1.5; }
  .bolt { fill: var(--muted); }
  .node rect { fill: var(--surface-2); stroke: var(--border); }
  .node.live rect { fill: color-mix(in oklab, var(--k) 12%, var(--surface-2)); stroke: color-mix(in oklab, var(--k) 60%, var(--border)); }
  .ico { fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .lbl { font-size: 11px; fill: var(--muted); }
  .val { font-size: 16px; font-weight: 600; fill: var(--text); font-variant-numeric: tabular-nums; }
  .sub { font-size: 10.5px; fill: var(--muted); }
  @keyframes flow { to { stroke-dashoffset: -22; } }
</style>
