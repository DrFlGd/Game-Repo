// Sizes, counts and times as Game Repo Go's pages say them.

export function bytes(n) {
  n = Number(n) || 0;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n < 10 && i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

export const plural = (n, one, many = `${one}s`) => `${Number(n).toLocaleString("en-GB")} ${n === 1 ? one : many}`;

export const count = (n) => Number(n || 0).toLocaleString("en-GB");

/** How long ago, as people say it ("3 days ago"); `secs` since 1970. */
export function ago(secs) {
  if (!secs) return "";
  const s = Date.now() / 1000 - secs;
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} minutes ago`;
  if (s < 86400) return plural(Math.round(s / 3600), "hour") + " ago";
  if (s < 86400 * 45) return plural(Math.round(s / 86400), "day") + " ago";
  return new Date(secs * 1000).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

/** ES-DE's `lastplayed` (20241014T153000, the device's own time) as seconds since 1970;
 *  0 when never. */
export function played(t) {
  const m = /^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/.exec(t || "");
  return m ? new Date(+m[1], m[2] - 1, +m[3], +m[4], +m[5], +m[6]).getTime() / 1000 : 0;
}

export const message = (e) => e?.message || String(e);

/** The media kinds (gamerepo_wire::MEDIA and MANUAL), with their names. */
export const KINDS = [
  ["cover", "Box front"], ["back", "Box back"], ["box3d", "3D box"], ["screenshot", "Screenshot"],
  ["titlescreen", "Title screen"], ["marquee", "Marquee (logo)"], ["fanart", "Fan art"],
  ["physical", "Cartridge or disc"], ["video", "Video"], ["manual", "Manual"],
];
export const kindName = (k) => (KINDS.find(([v]) => v === k) || [k, k])[1];
