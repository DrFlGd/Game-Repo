// The library's games (Phase 1): the folders with games, scanning them, the DAT
// files and what Home shows. The Audit page is in audit.js; the commands behind
// these are in desktop/core/src/api.rs.
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, loadIndex, baseName } from "./library.js";

/** The platforms the app knows ([{ id, name, dat, exts }]), read once. */
export async function platforms() {
  if (!ui.get().platforms?.length) ui.set({ platforms: await api("platforms") });
  return ui.get().platforms;
}

/** Read again what Home and Settings show: the folders with games, the DATs and the counts. */
export async function refreshGames() {
  if (!ui.get().library) { ui.set({ home: null, roots: [], dats: [] }); return; }
  try {
    // the systems too: a library can have systems of its own (Settings › Systems)
    const [home, dats, list] = await Promise.all([api("home"), api("dats"), api("platforms")]);
    ui.set({ home, roots: home.roots, dats, platforms: list });
    loadIndex();
  } catch (e) {
    ui.set({ home: { error: e.message || String(e) } });
  }
}

/** Show a file (or the ZIP it's in) in the file manager. */
export function reveal(path) {
  if (path && isDesktop()) ctx.platform.library.reveal(path);
}

// ------------------------------------------------------------ folders with games

export async function addRoot(path, platform) {
  const target = path || (await ctx.platform.library.pickFolder("Choose a folder with games"));
  if (!target) return null;
  try {
    const root = await api("roots_add", { path: target, platform: platform || null });
    await refreshGames();
    toast(`Added ${baseName(root.dir)}. Scan to read what's in it.`, { ms: 8000, action: { label: "Scan now", run: () => scan() } });
    return root;
  } catch (e) {
    toast(`Couldn't add that folder: ${e.message || e}`, 8000);
    return null;
  }
}

export async function updateRoot(id, patch) {
  try {
    await api("roots_update", { id, ...patch });
    await refreshGames();
  } catch (e) {
    toast(`Couldn't change that folder: ${e.message || e}`, 8000);
  }
}

/** Point a folder that isn't on this computer somewhere else (it keeps its id). */
export async function repointRoot(root) {
  const target = await ctx.platform.library.pickFolder(`Where is ${root.path} on this computer?`);
  if (target) await updateRoot(root.id, { path: target });
}

export async function removeRoot(root) {
  await api("roots_remove", { id: root.id });
  await refreshGames();
  toast(`${baseName(root.dir)} won't be scanned any more; nothing in it was changed.`, 6000);
}

// ------------------------------------------------------------ scanning

/** Scan the folders with games, following the job in the status bar. */
export async function scan(full) {
  if (!ui.get().roots?.length) await refreshGames();
  if (!ui.get().roots?.length) {
    toast("Add a folder with games first (Import › Folders with games).", 6000);
    return null;
  }
  try {
    const { job } = await api("scan", full == null ? {} : { full });
    ui.set({ scanning: { text: "Listing files" } });
    const done = await followJob(job, "Scanning", (j) => ui.set({ scanning: j.progress || {} }));
    ui.set({ scanning: null });
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    const parts = [];
    if (r.added) parts.push(`${r.added} new`);
    if (r.changed) parts.push(`${r.changed} changed`);
    if (r.moved) parts.push(`${r.moved} moved`);
    if (r.removed) parts.push(`${r.removed} gone`);
    const what = parts.length ? parts.join(", ") : "nothing changed";
    toast(`${r.stopped ? "Scan stopped" : "Scanned"} ${plural(r.files || 0, "file", "files")}: ${what}${r.hashed ? `; read ${plural(r.hashed, "file", "files")} (${size(r.bytes)})` : ""}.${r.failed ? ` ${plural(r.failed, "file", "files")} couldn't be read.` : ""}`, 8000);
    await refreshGames();
    return r;
  } catch (e) {
    ui.set({ scanning: null });
    toast(`Couldn't scan: ${e.message || e}`, 8000);
    return null;
  }
}

/** Scans the app starts itself, when the folder watcher saw files change: shown in
 *  the status bar like the page's own, and what the page shows is read again after. */
export function followAutoScans() {
  const seen = new Set();
  let first = true;
  const told = async (done) => {
    const r = done.result || {};
    const parts = [r.added && `${r.added} new`, r.changed && `${r.changed} changed`, r.moved && `${r.moved} moved`, r.removed && `${r.removed} gone`].filter(Boolean);
    if (!parts.length) return;
    await refreshGames();
    toast(`Scanned what changed in your folders: ${parts.join(", ")}.`, 5000);
  };
  const poll = async () => {
    if (ui.get().library && !document.hidden) {
      try {
        for (const j of await api("jobs")) {
          if (!j.auto || seen.has(j.id)) continue;
          seen.add(j.id);
          if (first) continue;
          if (j.done) told(j);
          else followJob(j.id, "Scanning what changed").then(told);
        }
        first = false;
      } catch { /* the next poll tries again */ }
    }
    setTimeout(poll, 2000);
  };
  poll();
}

export async function scanSettings() {
  return api("scan_settings");
}
export async function setScanSettings(patch) {
  return api("scan_settings_set", patch);
}

// ------------------------------------------------------------ DATs

/** Import DATs (`paths`, or files chosen in a dialog). A DAT whose platform
 *  isn't recognised is asked about, and imported again with the answer. */
export async function importDats(paths, answers = {}) {
  const chosen = paths || (await ctx.platform.library.pickFiles("Import DAT files (.dat, .xml or .zip)"));
  if (!chosen?.length) return null;
  try {
    const { job } = await api("dat_import", { paths: chosen, answers });
    const done = await followJob(job, "Importing DATs");
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    const got = r.imported || [];
    // one message for the lot, each file by its name: a message each replaced the one
    // before, so only the last failure showed (review M39)
    const said = [];
    if (got.length) {
      const fresh = got.filter((d) => !d.already);
      said.push(fresh.length
        ? `Imported ${fresh.map((d) => `${d.name} for ${d.platform_name}`).join(", ")}: ${plural(fresh.reduce((n, d) => n + d.games, 0), "game", "games")}, ${fresh.reduce((n, d) => n + d.matched, 0)} matched.`
        : `${got.map((d) => d.name).join(", ")} ${got.length === 1 ? "was" : "were"} imported already.`);
    }
    said.push(...failedLines(r.failed, (f) => baseName(f.path), "Couldn't import", "files"));
    if (said.length) toast(said.join(" "), r.failed?.length ? 15000 : 8000);
    await refreshGames();
    if (got.some((d) => !d.already)) offerFolderRenames();
    if (r.needs_platform?.length) {
      await platforms();
      ui.set({ dialog: { type: "dat-platform", asks: r.needs_platform, answers } });
    }
    return r;
  } catch (e) {
    toast(`Couldn't import the DATs: ${e.message || e}`, 8000);
    return null;
  }
}

/** After a DAT's new version: the game folders whose games it renamed are offered as one
 *  change, renaming them to the new names (D45). Until then they're found by their ROMs. */
async function offerFolderRenames() {
  try {
    const plan = await api("organise_rename_folders");
    if (!plan.ops) return;
    toast(`The new DAT renamed ${plural(plan.ops, "game", "games")} the library has a folder for.`, { ms: 15000, actions: [{ label: "Rename game folders to the DAT's names…", run: () => ui.set({ dialog: { type: "organise", plan, platformName: "" } }) }] });
  } catch {
    // a read-only library, or no games' folders: nothing to offer
  }
}

/** Download the No-Intro cartridge DATs, Redump's disc DATs and libretro's list of BIOS
 *  files from libretro's public copy (D17, D40), for the platforms that have no DAT yet
 *  (or only `platforms`), after asking. */
export function starterDats(platforms = null) {
  const bios = platforms?.length === 1 && platforms[0] === "bios";
  ui.set({ dialog: { type: "ask", title: bios ? "Get libretro's list of BIOS files?" : "Get starter DATs?",
    text: bios ? ["It lists the BIOS files libretro's emulators look for, with their hashes, so the library's can be audited against it.",
      "libretro's System.dat is downloaded from its public copy on GitHub and imported. It lists the files, not the files themselves: those are yours to find. A BIOS DAT you import yourself is used beside it."]
      : ["No-Intro's DATs for the cartridge systems and Redump's for the disc systems are downloaded and imported, for each system that has no DAT yet.",
      "They come from libretro's public copies on GitHub, with libretro's list of BIOS files. They can be a little behind the newest, and libretro's copies of Redump's DATs list only one track of each disc. A DAT you import yourself becomes the version in use."],
    button: bios ? "Download it" : "Download them", run: () => runStarter(platforms) } });
}

/** What failed, as one or two sentences: the first three by name, and how many more. */
function failedLines(failed, name, verb, what) {
  if (!failed?.length) return [];
  const why = (f) => String(f.error || "").replace(/\.$/, "");
  if (failed.length === 1) return [`${verb} ${name(failed[0])}: ${why(failed[0])}.`];
  const more = failed.length > 3 ? `; and ${failed.length - 3} more` : "";
  return [`${verb} ${failed.length} ${what}: ${failed.slice(0, 3).map((f) => `${name(f)} (${why(f)})`).join("; ")}${more}.`];
}

async function runStarter(platforms) {
  try {
    const { job } = await api("starter_dats", platforms ? { platforms } : {});
    const done = await followJob(job, "Getting starter DATs");
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    const n = (r.imported || []).length;
    const extra = [r.skipped?.length ? `${r.skipped.length} already had one` : null, r.missing?.length ? `${r.missing.length} not available` : null, r.failed?.length ? `${r.failed.length} failed` : null].filter(Boolean).join(", ");
    const said = [`${n ? `Imported starter DATs for ${plural(n, "system", "systems")}` : "No starter DATs were needed"}${extra ? ` (${extra})` : ""}.`,
      ...failedLines(r.failed, (f) => `the ${(ui.get().platforms || []).find((p) => p.id === f.platform)?.name || f.platform} DAT`, "Couldn't get", "DATs"), ...(r.note ? [r.note] : [])];
    toast(said.join(" "), r.failed?.length || r.note ? 15000 : 8000);
    await refreshGames();
    return r;
  } catch (e) {
    toast(`Couldn't get the starter DATs: ${e.message || e}`, 8000);
    return null;
  }
}

export async function useDat(d) {
  await api("dat_use", { id: d.id });
  await refreshGames();
  toast(`${d.name} (${d.version}) is the version in use for ${d.platform_name}.`);
}

export async function removeDat(d) {
  await api("dat_remove", { id: d.id });
  await refreshGames();
  toast(`Removed ${d.name} (${d.version}).`);
}

/** Make a DAT of a scanned set: `{ root, folder, name, platform }`. */
export async function makeDat(opts) {
  const { job } = await api("make_dat", opts);
  const done = await followJob(job, "Making a DAT");
  if (done.error) throw new Error(done.error);
  const r = done.result;
  await refreshGames();
  toast(`Wrote ${r.name}: ${plural(r.games, "game", "games")}, ${plural(r.roms, "file", "files")}${r.imported?.id ? `, imported for ${r.imported.platform_name}` : ""}.${r.skipped ? ` ${plural(r.skipped, "file wasn't", "files weren't")} scanned yet and ${r.skipped === 1 ? "was" : "were"} left out.` : ""}`,
    { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.path) }] : [] });
  return r;
}
