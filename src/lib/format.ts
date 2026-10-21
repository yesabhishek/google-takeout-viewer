const nf = new Intl.NumberFormat();
export const fmtCount = (n: number) => nf.format(n);

export function plural(n: number, one: string, many = one + "s") {
  return `${fmtCount(n)} ${n === 1 ? one : many}`;
}

const today = () => Math.floor((Date.now() - new Date().getTimezoneOffset() * 60000) / 86400000);

const dayFmtThisYear = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric", timeZone: "UTC" });
const dayFmtOther = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });

/** Header label for a local calendar day number (days since epoch). */
export function dayLabel(day: number): string {
  const t = today();
  if (day === t) return "Today";
  if (day === t - 1) return "Yesterday";
  const d = new Date(day * 86400000);
  return d.getUTCFullYear() === new Date().getFullYear() ? dayFmtThisYear.format(d) : dayFmtOther.format(d);
}

const monthYearFmt = new Intl.DateTimeFormat(undefined, { month: "short", year: "numeric" });
export const monthYear = (ts: number) => monthYearFmt.format(new Date(ts * 1000));

const longDate = new Intl.DateTimeFormat(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
export const fmtLongDate = (ts: number) => longDate.format(new Date(ts * 1000));
export const fmtTime = (ts: number) => timeFmt.format(new Date(ts * 1000));

const rangeFmt = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
export function fmtRange(a: number | null, b: number | null): string {
  if (!a || !b) return "";
  const da = new Date(a * 1000);
  const db = new Date(b * 1000);
  if (da.toDateString() === db.toDateString()) return rangeFmt.format(da);
  try {
    return rangeFmt.formatRange(da, db);
  } catch {
    return `${rangeFmt.format(da)} – ${rangeFmt.format(db)}`;
  }
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const u = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${u[i]}`;
}

export function fmtDuration(sec: number): string {
  const s = Math.max(0, Math.round(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? h + ":" : ""}${mm}:${String(r).padStart(2, "0")}`;
}

export function fmtAgo(ts: number | null): string {
  if (!ts) return "never";
  const d = Date.now() / 1000 - ts;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  if (d < 2 * 86400) return "yesterday";
  if (d < 7 * 86400) return `${Math.floor(d / 86400)} days ago`;
  return `on ${fmtLongDate(ts)}`;
}

/**
 * A library folder as people think of it: its name, and where it lives
 * ("Western Digital Passport › Google Photos Backup").
 */
export function describeFolder(path: string): { name: string; where: string; external: boolean } {
  const p = path.replace(/^\\\\\?\\/, ""); // Windows long-path prefix
  const parts = p.split(/[\\/]/).filter(Boolean);
  const name = parts.pop() ?? p;
  let crumbs = parts;
  let external = false;
  const home = p.match(/^\/Users\/[^/]+/)?.[0];
  if (p.startsWith("/Volumes/")) {
    external = true;
    crumbs = parts.slice(1); // drop "Volumes"
  } else if (/^[A-Za-z]:/.test(p)) {
    external = !/^C:/i.test(p);
  } else if (home) {
    crumbs = ["~", ...parts.slice(2)];
  }
  if (crumbs.length > 4) crumbs = [crumbs[0], "…", ...crumbs.slice(-2)];
  return { name, where: crumbs.join(" › ") || "/", external };
}

/** Shorten a long path with a middle ellipsis, keeping the start and the folder name. */
export function shortPath(p: string, max = 72): string {
  const home = p.match(/^\/Users\/[^/]+/)?.[0];
  if (home) p = "~" + p.slice(home.length);
  if (p.length <= max) return p;
  const tail = Math.floor(max * 0.6);
  return `${p.slice(0, max - tail - 1)}…${p.slice(-tail)}`;
}
