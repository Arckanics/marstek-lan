import uPlot from "uplot";
import { locale, t as tx } from "./i18n.svelte";

type Col = readonly (number | null | undefined)[];
const has = (v: number | null | undefined): v is number => v != null;

/** Tangente monotone (Fritsch-Butland) au point k, d'après ses seuls voisins du même tronçon : la courbe ne dépasse jamais les valeurs réelles voisines. */
function tangent(xs: Col, ys: Col, k: number, gap: number): number {
  const y = ys[k]!;
  const l = k > 0 && has(ys[k - 1]) && xs[k]! - xs[k - 1]! <= gap ? k - 1 : -1;
  const r = k < xs.length - 1 && has(ys[k + 1]) && xs[k + 1]! - xs[k]! <= gap ? k + 1 : -1;
  const d0 = l < 0 ? 0 : (y - ys[l]!) / (xs[k]! - xs[l]!);
  const d1 = r < 0 ? 0 : (ys[r]! - y) / (xs[r]! - xs[k]!);
  if (l < 0) return d1;
  if (r < 0) return d0;
  if (d0 * d1 <= 0) return 0;
  const h0 = xs[k]! - xs[l]!;
  const h1 = xs[r]! - xs[k]!;
  return (3 * (h0 + h1)) / ((2 * h1 + h0) / d0 + (h1 + 2 * h0) / d1);
}

/** Courbe de Hermite monotone entre les points réels ; un écart de plus de `gap()` (unité de x) coupe le tracé, qui s'arrête à `until()` si fourni. */
export const monotone =
  (gap: () => number, until?: () => number): uPlot.Series.PathBuilder =>
  (u, si, i0, i1) => {
    const xs = u.data[0] as Col;
    const ys = u.data[si] as Col;
    const g = gap();
    const sx = (v: number) => u.valToPos(v, "x", true);
    const sy = (v: number) => u.valToPos(v, u.series[si].scale!, true);
    const path = new Path2D();
    let pen = false;
    for (let k = Math.max(i0 - 1, 0); k < Math.min(i1 + 1, xs.length - 1); k++) {
      const x0 = xs[k]!;
      const x1 = xs[k + 1]!;
      if (!has(ys[k]) || !has(ys[k + 1]) || x1 - x0 > g || x1 <= x0) {
        pen = false;
        continue;
      }
      const h = x1 - x0;
      const m0 = tangent(xs, ys, k, g);
      const m1 = tangent(xs, ys, k + 1, g);
      if (!pen) path.moveTo(sx(x0), sy(ys[k]!));
      path.bezierCurveTo(sx(x0 + h / 3), sy(ys[k]! + (m0 * h) / 3), sx(x1 - h / 3), sy(ys[k + 1]! - (m1 * h) / 3), sx(x1), sy(ys[k + 1]!));
      pen = true;
    }
    if (!until) return { stroke: path } as uPlot.Series.Paths;
    const clip = new Path2D();
    clip.rect(u.bbox.left, u.bbox.top, Math.max(0, sx(until()) - u.bbox.left), u.bbox.height);
    return { stroke: path, clip } as uPlot.Series.Paths;
  };

/** Points réels visibles seulement quand ils sont assez espacés (zoom fin). */
export const showPoints = (u: uPlot, _si: number, i0: number, i1: number) => (i1 - i0) * 4 <= u.bbox.width / uPlot.pxRatio;

export type Reading = { v: number; kind: "measured" | "interpolated" | "held" };

/** Valeur lue à l'instant t : mesure à `tol` près, sinon interpolation monotone entre deux mesures voisines, sinon valeur tenue. */
export function valueAt(xs: Col, ys: Col, t: number, o: { gap: number; tol: number; hold?: boolean; pointsOnly?: boolean }): Reading | undefined {
  let lo = 0;
  let hi = xs.length;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (xs[m]! <= t) lo = m + 1;
    else hi = m;
  }
  const j = lo - 1;
  const near = [j, j + 1].filter((k) => k >= 0 && k < xs.length && has(ys[k]) && Math.abs(xs[k]! - t) <= o.tol).sort((a, b) => Math.abs(xs[a]! - t) - Math.abs(xs[b]! - t))[0];
  if (near != null) return { v: ys[near]!, kind: "measured" };
  if (o.pointsOnly || j < 0) return undefined;
  if (j + 1 < xs.length) {
    if (!has(ys[j]) || !has(ys[j + 1]) || xs[j + 1]! - xs[j]! > o.gap) return undefined;
    const h = xs[j + 1]! - xs[j]!;
    const s = (t - xs[j]!) / h;
    const m0 = tangent(xs, ys, j, o.gap) * h;
    const m1 = tangent(xs, ys, j + 1, o.gap) * h;
    const s2 = s * s;
    const s3 = s2 * s;
    return { v: (2 * s3 - 3 * s2 + 1) * ys[j]! + (s3 - 2 * s2 + s) * m0 + (3 * s2 - 2 * s3) * ys[j + 1]! + (s3 - s2) * m1, kind: "interpolated" };
  }
  return o.hold && has(ys[j]) && t - xs[j]! <= o.gap ? { v: ys[j]!, kind: "held" } : undefined;
}

const kindLabel = (k: Reading["kind"]) => tx(`curve.${k}`);

/** Étendue pointillée d'une série : de (x0, y0) à (x1, y1), en unités de données. */
export type Ext = { si: number; x0: number; y0: number; x1: number; y1: number };

const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

/** Dessine les prolongements pointillés des séries visibles et la ligne du zéro de l'échelle y. */
export const overlay = (ext: (u: uPlot) => Ext[], zero = true): uPlot.Plugin => ({
  hooks: {
    draw: (u) => {
      const { ctx, bbox } = u;
      const dpr = uPlot.pxRatio;
      ctx.save();
      ctx.beginPath();
      ctx.rect(bbox.left, bbox.top, bbox.width, bbox.height);
      ctx.clip();
      if (zero) {
        const y = u.valToPos(0, "y", true);
        ctx.strokeStyle = css("--muted");
        ctx.globalAlpha = 0.6;
        ctx.lineWidth = dpr;
        ctx.beginPath();
        ctx.moveTo(bbox.left, y);
        ctx.lineTo(bbox.left + bbox.width, y);
        ctx.stroke();
        ctx.globalAlpha = 1;
      }
      ctx.setLineDash([6 * dpr, 5 * dpr]);
      ctx.lineWidth = 2 * dpr;
      for (const e of ext(u)) {
        const s = u.series[e.si];
        if (!s.show) continue;
        ctx.strokeStyle = s.stroke as string;
        ctx.beginPath();
        ctx.moveTo(u.valToPos(e.x0, "x", true), u.valToPos(e.y0, s.scale!, true));
        ctx.lineTo(u.valToPos(e.x1, "x", true), u.valToPos(e.y1, s.scale!, true));
        ctx.stroke();
      }
      ctx.restore();
    },
  },
});

/** Échelle Y qui inclut toujours 0 (puissances). */
export const rangeWithZero = (_u: uPlot, min: number | null, max: number | null): uPlot.Range.MinMax => {
  const [a, b] = uPlot.rangeNum(Math.min(min ?? 0, 0), Math.max(max ?? 0, 0), 0.1, true);
  return [Math.min(a ?? 0, 0), Math.max(b ?? 0, 0)];
};

/** Axe temps : hh:mm:ss sous 10 min, hh:mm au-delà. */
export const timeValues = (u: uPlot, splits: number[]) => {
  const span = (u.scales.x.max ?? 0) - (u.scales.x.min ?? 0);
  const o: Intl.DateTimeFormatOptions = span < 600 ? { hour: "2-digit", minute: "2-digit", second: "2-digit" } : { hour: "2-digit", minute: "2-digit" };
  return splits.map((s) => new Date(s * 1000).toLocaleTimeString(locale(), o));
};

export type TipOptions = {
  gap: () => number;
  /** Valeur lisible d'une série (unité comprise). */
  fmt: (si: number, v: number) => string;
  hold?: boolean;
  /** Séries de points seuls : jamais interpolées. */
  pointsOnly?: (si: number) => boolean;
  /** Séries sans infobulle. */
  skip?: (si: number) => boolean;
  /** Titre de l'infobulle ; par défaut la date et l'heure (x en secondes). */
  head?: (t: number) => string;
  /** Libellés de nature de la valeur, quand « mesuré » ne convient pas. */
  kinds?: Partial<Record<Reading["kind"], string>>;
};

/** Infobulle : heure, valeur exacte de chaque série visible et sa nature (mesuré, interpolé, estimé). */
export function tooltip(o: TipOptions): uPlot.Plugin {
  const tip = document.createElement("div");
  tip.className = "u-tip";
  tip.hidden = true;
  return {
    hooks: {
      init: (u) => u.over.appendChild(tip),
      destroy: () => tip.remove(),
      setCursor: (u) => {
        const left = u.cursor.left ?? -1;
        if (left < 0) return void (tip.hidden = true);
        const t = u.posToVal(left, "x");
        const tol = u.posToVal(left + 6, "x") - t;
        const head = document.createElement("strong");
        head.textContent = o.head ? o.head(t) : new Date(t * 1000).toLocaleString(locale(), { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit" });
        const rows: HTMLElement[] = [head];
        for (let si = 1; si < u.series.length; si++) {
          const s = u.series[si];
          if (!s.show || o.skip?.(si)) continue;
          const r = valueAt(u.data[0] as Col, u.data[si] as Col, t, { gap: o.gap(), tol, hold: o.hold, pointsOnly: o.pointsOnly?.(si) });
          if (!r) continue;
          const row = document.createElement("div");
          const dot = document.createElement("span");
          dot.className = "u-tip-dot";
          dot.style.background = s.stroke as string;
          row.append(dot, tx("curve.row", { label: String(s.label), value: o.fmt(si, r.v), kind: o.kinds?.[r.kind] ?? kindLabel(r.kind) }));
          rows.push(row);
        }
        tip.hidden = rows.length < 2;
        tip.replaceChildren(...rows);
        tip.style.left = `${left + 12 + tip.offsetWidth > u.over.clientWidth ? left - 12 - tip.offsetWidth : left + 12}px`;
        tip.style.top = "4px";
      },
    },
  };
}
