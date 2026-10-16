// The open library: opening another folder, renaming it, its index on this
// computer, messages (with Undo) and background jobs.
import { ui, addJob } from "./state.js";
import { ctx } from "./context.js";

export const isDesktop = () => ctx.platform?.kind === "desktop";
export const api = (cmd, args) => ctx.platform.api(cmd, args);

/** How far above the window's foot a message goes: clear of the status bar. */
function clearance() {
  let top = innerHeight;
  for (const bar of document.querySelectorAll(".statusbar-inner")) {
    const r = bar.getBoundingClientRect();
    if (r.height && r.top > innerHeight * 0.6 && r.top < top) top = r.top;
  }
  return Math.max(20, Math.round(innerHeight - top + 10));
}

/** Show a short message at the bottom of the window. `opts`: how long (ms), or
 *  { ms, action: { label, run } } for a button in the message (Undo). It stays
 *  while the pointer is on it. */
export function toast(text, opts = {}) {
  const { ms = 3500, action = null, actions = [] } = typeof opts === "number" ? { ms: opts } : opts;
  const el = document.getElementById("toast");
  if (!el) return;
  const span = document.createElement("span");
  span.textContent = text;
  el.replaceChildren(span);
  for (const item of [...(action ? [action] : []), ...actions]) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "toast-action";
    b.textContent = item.label;
    b.onclick = () => { el.hidden = true; item.run(); };
    el.append(b);
  }
  el.hidden = false;
  // again once the page has drawn what the change brought
  const place = () => { el.style.bottom = `${clearance()}px`; };
  place(); setTimeout(place, 300);
  const hide = (after) => { clearTimeout(toast.timer); toast.timer = setTimeout(() => { el.hidden = true; }, after); };
  el.onmouseenter = () => clearTimeout(toast.timer);
  el.onmouseleave = () => hide(2500);
  hide(ms);
}

/** Changes made in this window that can be undone, newest last: { text, undo }. */
const undos = [];

/** Say a change was made, with Undo in the message; Ctrl+Z undoes it too.
 *  `undo()` puts it back, and may return the message to show then. Returns a
 *  function that undoes it (for an Undo button on the page). */
export function undoable(text, undo, ms = 8000, actions = []) {
  const entry = { text, undo };
  undos.push(entry);
  if (undos.length > 20) undos.shift();
  toast(text, { ms, action: { label: "Undo", run: () => runUndo(entry) }, actions });
  return () => runUndo(entry);
}

/** "Renamed the library Retro. More…" → "renamed the library Retro": the first clause, for "Undone: …". */
const undoneText = (text) => {
  const first = text.split(/[.:](?:\s|$)/)[0];
  return /^[A-Z][a-z]/.test(first) ? first[0].toLowerCase() + first.slice(1) : first;
};

async function runUndo(entry) {
  const i = undos.indexOf(entry);
  if (i < 0) return toast("That's been undone already.");
  undos.splice(i, 1);
  try {
    const said = await entry.undo();
    toast(typeof said === "string" ? said : `Undone: ${undoneText(entry.text)}.`, 5000);
  } catch (e) {
    toast(`Couldn't undo it: ${e.message || e}`, 8000);
  }
}

/** Ctrl+Z: undo the last change made in this window. */
export function undoLast() {
  const entry = undos[undos.length - 1];
  if (!entry) return toast("Nothing to undo.");
  return runUndo(entry);
}

/** Read the app's info again and update what the interface shows. */
export async function refreshLibrary() {
  const info = await ctx.platform.refreshInfo();
  ui.set({ library: info.library || null, libraryError: info.library_error || null, recent: info.recent_libraries || [], firstRun: !!info.first_run });
  return info;
}

/** The open library's index on this computer: where it is, its size and tables
 *  (opening it makes it, or brings its tables up to date). */
export async function loadIndex() {
  if (!ui.get().library) { ui.set({ index: null }); return null; }
  try {
    const index = await api("index_info");
    ui.set({ index });
    if (index.set_aside) toast(`The index on this computer was made again: ${index.set_aside}.`, 8000);
    return index;
  } catch (e) {
    ui.set({ index: { error: e.message || String(e) } });
    return null;
  }
}

/** SQLite's own check of the index file, as a job in the status bar. */
export async function checkIndex() {
  try {
    const { job } = await api("index_check");
    const done = await followJob(job, "Checking the index");
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    toast(r.ok ? "The index is sound." : `The index has problems: ${(r.problems || []).slice(0, 3).join("; ")}`, r.ok ? 4000 : 10000);
    await loadIndex();
    return r;
  } catch (e) {
    toast(`Couldn't check the index: ${e.message || e}`, 6000);
    return null;
  }
}

/** Open (or create) a library folder: `path`, or one picked in a dialog. */
export async function openLibrary(path) {
  const p = ctx.platform;
  const target = path || (await p.library.pickFolder("Open or create a library folder"));
  if (!target) return null;
  const done = addJob("Opening library");
  try {
    const info = await api("library_open", { path: target });
    await refreshLibrary();
    undos.length = 0;
    await loadIndex();
    toast(`Opened ${info.name}.`);
    return info;
  } catch (e) {
    toast(`Couldn't open that folder: ${e.message || e}`, 6000);
    return null;
  } finally {
    done();
  }
}

export async function renameLibrary(name) {
  await api("library_rename", { name });
  await refreshLibrary();
}

export function showLibraryFolder() {
  const path = ui.get().library?.path;
  if (path) ctx.platform.library.openPath(path);
}

/** A file in the library as the page can load it (pictures, documents). */
export const libraryUrl = (rel) => ctx.platform.library?.url(rel);

/** Wait for a background job (scanning, checking), showing it in the status bar.
 *  `onProgress(job)` sees each poll. Resolves with the finished job. */
export async function followJob(id, label, onProgress) {
  const done = addJob(label, () => api("job_cancel", { id }));
  try {
    for (;;) {
      const job = await api("job", { id });
      onProgress?.(job);
      const p = job.progress || {};
      if (p.items) done.update(`${label}: ${Math.min(p.item + 1, p.items)} of ${p.items}`);
      else if (p.text) done.update(`${label}: ${p.text}`);
      if (job.done) return job;
      await new Promise((r) => setTimeout(r, 300));
    }
  } finally {
    done();
  }
}

/** A size in bytes as people read it: "12 KB", "1.4 GB". */
export function size(bytes) {
  if (bytes == null) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes, i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n < 10 && i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

export const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
