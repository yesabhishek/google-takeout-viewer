// Columnar timeline data and the justified layout. Everything here is plain
// typed arrays so 200k+ items lay out in a few milliseconds.

export const FLAG_VIDEO = 1;
export const FLAG_LIVE = 2;
export const FLAG_FAV = 4;

export interface Timeline {
  n: number;
  ts: Float64Array;
  ids: Uint32Array;
  day: Int32Array;
  w: Uint16Array;
  h: Uint16Array;
  dur: Uint16Array;
  flags: Uint8Array;
  index: Map<number, number>; // id -> position
}

export function emptyTimeline(): Timeline {
  return decode(new Uint32Array([0x544c3031, 0]).buffer);
}

export function decode(buf: ArrayBuffer): Timeline {
  const head = new DataView(buf, 0, 8);
  if (head.getUint32(0, true) !== 0x544c3031) throw new Error("bad timeline payload");
  const n = head.getUint32(4, true);
  let o = 8;
  const ts = new Float64Array(buf, o, n);
  o += 8 * n;
  const ids = new Uint32Array(buf, o, n);
  o += 4 * n;
  const day = new Int32Array(buf, o, n);
  o += 4 * n;
  const w = new Uint16Array(buf, o, n);
  o += 2 * n;
  const h = new Uint16Array(buf, o, n);
  o += 2 * n;
  const dur = new Uint16Array(buf, o, n);
  o += 2 * n;
  const flags = new Uint8Array(buf, o, n);
  const index = new Map<number, number>();
  for (let i = 0; i < n; i++) index.set(ids[i], i);
  return { n, ts, ids, day, w, h, dur, flags, index };
}

export interface Header {
  x: number;
  y: number;
  width: number;
  day: number;
  first: number;
  last: number; // exclusive
}

export interface Layout {
  width: number;
  height: number;
  x: Float32Array;
  y: Float32Array;
  tw: Float32Array;
  th: Float32Array;
  headers: Header[];
  years: { year: number; y: number }[];
}

export const GAP = 4;
export const HEADER_H = 52;
const SEGMENT_GAP = 24;
const BAND_GAP = 12;

function aspect(t: Timeline, i: number): number {
  const a = t.w[i] / Math.max(1, t.h[i]);
  return Math.min(4, Math.max(0.33, a || 1));
}

/**
 * Google Photos-style layout: each day gets a header; days with only a few
 * items are packed side by side on one row band; larger days are laid out in
 * justified rows whose height stays close to `rowH`.
 */
export function layout(t: Timeline, width: number, rowH: number, grouped = true): Layout {
  const n = t.n;
  const x = new Float32Array(n);
  const y = new Float32Array(n);
  const tw = new Float32Array(n);
  const th = new Float32Array(n);
  const headers: Header[] = [];
  const years: { year: number; y: number }[] = [];
  width = Math.max(200, width);

  let cursorY = 0;
  // open band of packed small days
  let bandOpen = false;
  let bandX = 0;
  let bandY = 0;
  let bandH = 0;
  const closeBand = () => {
    if (bandOpen) {
      cursorY = bandY + bandH + BAND_GAP;
      bandOpen = false;
    }
  };
  const headH = grouped ? HEADER_H : 0;

  let lastYear = NaN;
  let s = 0;
  while (s < n) {
    let e = s + 1;
    if (grouped) {
      while (e < n && t.day[e] === t.day[s]) e++;
    } else {
      e = n;
    }

    // natural width of the segment at rowH
    let natural = -GAP;
    for (let i = s; i < e; i++) natural += aspect(t, i) * rowH + GAP;

    let segTop: number;
    if (grouped && natural <= width) {
      // fits on one line: pack next to the previous small day if possible
      let sx = 0;
      if (bandOpen && bandX + SEGMENT_GAP + natural <= width) {
        sx = bandX + SEGMENT_GAP;
      } else {
        closeBand();
        bandOpen = true;
        bandY = cursorY;
        bandH = headH + rowH;
      }
      segTop = bandY;
      headers.push({ x: sx, y: bandY, width: natural, day: t.day[s], first: s, last: e });
      let cx = sx;
      for (let i = s; i < e; i++) {
        const w = aspect(t, i) * rowH;
        x[i] = cx;
        y[i] = bandY + headH;
        tw[i] = w;
        th[i] = rowH;
        cx += w + GAP;
      }
      bandX = sx + natural;
    } else {
      closeBand();
      segTop = cursorY;
      if (grouped) headers.push({ x: 0, y: cursorY, width, day: t.day[s], first: s, last: e });
      cursorY += headH;
      // justified rows
      let rs = s;
      while (rs < e) {
        let sumA = 0;
        let re = rs;
        let h = rowH;
        let full = false;
        while (re < e) {
          const a = aspect(t, re);
          const k = re - rs + 1; // items in the row if `re` is included
          if ((sumA + a) * rowH + GAP * (k - 1) < width) {
            sumA += a;
            re++;
            continue;
          }
          full = true;
          if (k === 1) {
            sumA = a;
            re++;
            h = width / a;
            break;
          }
          // include the overflowing item only if that keeps the height closer to rowH
          const hWith = (width - GAP * (k - 1)) / (sumA + a);
          const hWithout = (width - GAP * (k - 2)) / sumA;
          if (Math.abs(hWith - rowH) <= Math.abs(hWithout - rowH)) {
            sumA += a;
            re++;
            h = hWith;
          } else {
            h = hWithout;
          }
          break;
        }
        if (!full) h = rowH; // last row of the day stays left-aligned
        const capped = h > rowH * 1.6;
        if (capped) h = rowH * 1.6;
        let cx = 0;
        for (let i = rs; i < re; i++) {
          const w = aspect(t, i) * h;
          x[i] = cx;
          y[i] = cursorY;
          tw[i] = w;
          th[i] = h;
          cx += w + GAP;
        }
        // absorb float rounding so justified rows end exactly at the edge
        if (full && !capped) tw[re - 1] += width - (cx - GAP);
        cursorY += h + GAP;
        rs = re;
      }
      cursorY += BAND_GAP - GAP;
    }

    const yr = new Date(t.day[s] * 86400000).getUTCFullYear();
    if (yr !== lastYear) {
      years.push({ year: yr, y: segTop });
      lastYear = yr;
    }
    s = e;
  }
  closeBand();
  return { width, height: Math.max(0, cursorY), x, y, tw, th, headers, years };
}

/** First index whose tile bottom is below `top`. */
export function firstVisible(l: Layout, top: number): number {
  let lo = 0;
  let hi = l.y.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (l.y[mid] + l.th[mid] < top) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** First index whose tile top is below `bottom`. */
export function lastVisible(l: Layout, bottom: number): number {
  let lo = 0;
  let hi = l.y.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (l.y[mid] <= bottom) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
