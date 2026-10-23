// Organising a set by its DAT (docs/PLAN.md, "Phase 2 design", 2a): the Organise…
// dialog (how, where and in what format), its preview from the index alone, then
// the change as a job whose message has Undo. "Rename these" (Misnamed) and "Set
// aside duplicates" (Duplicates) open the same preview. The history of changes is
// in Settings (settings.js), from the commands here. Convert… (2b) plans a change
// the same way: a game, a selection or a platform to its preferred format, or back.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { api, toast, followJob, undoable, isDesktop, plural, size, shortPath } from "./library.js";
import { refreshGames, reveal } from "./games.js";
import { Dialog, More, close, confirmDialog } from "./dialogs.js";
import { Icon } from "./icons.js";

const PAGE = 100;

export const MODES = [
  ["copy", "Copy to a folder", "Your files stay as they are; a sorted copy is written to a folder you choose (hardlinked when it's on the same drive, so it takes no space)."],
  ["rename", "Rename where they are", "Files are renamed in their folders to their DAT names. Extra copies and replaced archives are set aside, not deleted."],
  ["move", "Move into the library", "Files are moved into a folder for the system in the library, named by the DAT. What they replace is set aside, not deleted."],
];

export const FORMATS = [
  ["keep", "As they are"],
  ["zip", "A ZIP for each game (TorrentZip)"],
  ["loose", "Loose files"],
];

const OPS = { text: "Write", image: "Write smaller picture", rename: "Rename", move: "Move", copy: "Copy", link: "Hardlink", zip: "Write ZIP", extract: "Unpack", aside: "Set aside", convert: "Convert", sheet: "Write cue sheet", cue: "Rewrite cue sheet" };

/** The formats a game can be converted to, and what each is for. */
export const CONVERT_FORMATS = {
  chd: ["CHD", "Compressed, with every track of the disc in one file. Read by RetroArch, DuckStation, PCSX2, Flycast and most other emulators."],
  rvz: ["RVZ", "Dolphin's compressed format, for GameCube and Wii. Needs dolphin-tool (Settings › Tools)."],
  cso: ["CSO", "A compressed ISO, read by PPSSPP and by PSPs with custom firmware."],
  zso: ["ZSO", "Like CSO but quicker to read (LZ4), for PPSSPP and newer PSP firmware."],
  zip: ["ZIP", "A ZIP for each game, written the TorrentZip way."],
};

const verb = (kind) => ({ folders: "Renaming games' library folders", aside: "Setting aside duplicates", convert: "Converting", extract: "Unpacking", export: "Exporting", import: "Importing PC games", add: "Importing a game", bring: "Importing games", docs: "Importing documents", magazines: "Importing magazines", videos: "Importing videos", bios: "Importing BIOS files", metadata: "Deleting metadata", pc_remove: "Removing a PC game", video_remove: "Removing videos", magazine_move: "Moving magazine issues" })[kind] || "Organising";

/** The changes that bring files into the library (the Import page's Recent imports, D78). */
export const IMPORT_KINDS = ["bring", "add", "import", "docs", "magazines", "videos", "bios"];

/** How an import brings its files in (D79), the same three choices everywhere: "copy"
 *  (hardlinked on the same drive), "always" (a full copy) or "move"; and for games from
 *  folders (`leave`), "leave": their folders become folders with games, nothing copied. */
export function HowControl({ id, value, onChange, what = "them", leave = false }) {
  const row = (v, text, note) => html`<label class="check-row" key=${v}><input type="radio" name=${id} value=${v} checked=${value === v} onChange=${() => onChange(v)} /> ${text} <span class="muted">${note}</span></label>`;
  return html`<div class="field-block how-control" role="radiogroup" aria-label="How" id=${id}>
    <span class="field-label">How</span>
    ${row("copy", `Copy ${what}, hardlinked on the same drive`, "(no extra space; the originals stay)")}
    ${row("always", `Copy ${what}, always`, "(a full copy, even on the same drive)")}
    ${row("move", `Move ${what} into the library`, "(Undo puts them back)")}
    ${leave ? row("leave", `Leave ${what} where they are`, "(their folders become folders with games; Move into the library… brings them in later)") : null}
  </div>`;
}

/** The plan arguments for a HowControl's value. */
export const howArgs = (how) => ({ mode: how === "move" ? "move" : "copy", always_copy: how === "always" });

/** What a plan's preview counts, in its own words (review M40): [one, many, what's done]. */
const COUNTS = {
  export: ["game", "games", "export"], bring: ["game", "games", "import"], add: ["game", "games", "import"], import: ["PC game", "PC games", "import"],
  docs: ["document", "documents", "import"], magazines: ["issue", "issues", "import"], bios: ["system's BIOS files", "systems' BIOS files", "import"],
  convert: ["game", "games", "convert"], extract: ["game", "games", "unpack"], folders: ["game's folder", "games' folders", "rename"],
  metadata: ["game's metadata", "games' metadata", "delete"], aside: ["game", "games", "set extra copies aside for"],
  pc_remove: ["PC game", "PC games", "remove from the library"], videos: ["video", "videos", "import"], video_remove: ["video", "videos", "remove"],
  magazine_move: ["issue", "issues", "move"],
};

/** Open the Organise… dialog for a DAT: `{ dat, platform, platformName }`. */
export const organiseDialog = (opts) => ui.set({ dialog: { type: "organise", ...opts } });

/** Move into the library…: every game left in a folder with games outside the library
 *  folder (an import that left them where they were), moved in as one change, each
 *  system's into its own folder there; the preview first. */
export async function gatherDialog(platform = "") {
  try {
    const plan = await api("organise_gather", { platform });
    ui.set({ dialog: { type: "organise", plan, platformName: "" } });
  } catch (e) {
    toast(`Couldn't plan the move: ${e.message || e}`, 8000);
  }
}

/** Open the Convert… dialog: `{ dat, platform, platformName, games? }` (games by name; all when left out). */
export const convertDialog = (opts) => ui.set({ dialog: { type: "convert", ...opts } });

/** Plan renaming only the misnamed files where they are, and show the preview. */
export async function renameMisnamed(dat, platformName) {
  try {
    const plan = await api("organise_plan", { dat, mode: "rename", only_misnamed: true });
    ui.set({ dialog: { type: "organise", dat, platformName, plan } });
  } catch (e) {
    toast(`Couldn't plan the renames: ${e.message || e}`, 8000);
  }
}

/** Plan setting aside the extra copies of each matched file, and show the preview. */
export async function setAsideDuplicates(dat, platformName) {
  try {
    const plan = await api("organise_duplicates", { dat });
    ui.set({ dialog: { type: "organise", dat, platformName, plan } });
  } catch (e) {
    toast(`Couldn't plan setting the duplicates aside: ${e.message || e}`, 8000);
  }
}

/** Put a change back, as a job. Resolves with what the page says then. */
export async function undoChange(id) {
  const { job } = await api("change_undo", { id });
  const done = await followJob(job, "Undoing");
  await refreshGames();
  ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
  if (done.error) throw new Error(done.error);
  const r = done.result || {};
  const problems = r.problems || [];
  const kept = problems.filter((p) => p.kept);
  if (r.finished === false) {
    const first = problems.find((p) => !p.kept);
    return `Put back ${plural(r.undone, "step", "steps")}; ${problems.length - kept.length} couldn't be: ${first?.error}. Nothing else was removed; undo it again from History once that's put right.`;
  }
  return kept.length ? `Undone: ${r.label}. ${plural(kept.length, "file was", "files were")} changed after it ran, so ${kept.length === 1 ? "it stays" : "they stay"}: ${kept[0].to}` : `Undone: ${r.label}.`;
}

/** Finish a change that was interrupted. */
export async function finishChange(id) {
  const { job } = await api("change_finish", { id });
  const done = await followJob(job, "Finishing a change");
  await refreshGames();
  ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
  if (done.error) throw new Error(done.error);
  return done.result;
}

/** At start-up: a change left half-done (the app closed or the computer stopped while
 *  it ran) is offered to be finished or put back. */
export async function offerInterrupted() {
  if (!ui.get().library) return;
  try {
    const c = (await api("change_history")).find((x) => x.state === "interrupted");
    if (!c) return;
    const run = (fn) => fn(c.id).then((said) => toast(typeof said === "string" ? said : "Finished it.", 6000)).catch((e) => toast(e.message || String(e), 10000));
    toast(`A change didn't finish: ${c.label} (${c.done_ops} of ${plural(c.ops, "step", "steps")} done).`, {
      ms: 30000, actions: [{ label: "Finish it", run: () => run(finishChange) }, ...(c.can_undo ? [{ label: "Undo", run: () => run(undoChange) }] : [])],
    });
  } catch { /* History offers it too */ }
}

/** Run a planned change, then say what it did, with Undo. Resolves with the job's result
 *  (null when it failed). */
export async function runPlan(plan, more = null) {
  try {
    const { job } = await api("organise_run", { id: plan.id });
    const done = await followJob(job, verb(plan.kind));
    await refreshGames();
    ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    const failed = r.failed?.length ? ` ${plural(r.failed.length, "step", "steps")} failed: ${r.failed[0].error}` : "";
    const warned = r.warnings?.length ? ` ${plural(r.warnings.length, "file", "files")} will show once the folder is scanned again.` : "";
    const art = r.art?.length ? ` Artwork for ${plural(r.art.length, "game", "games")}.` : "";
    // `more`: what's left to do afterwards, with its button (files no DAT knows to review)
    const text = `${plan.label}: ${r.done} of ${plural(r.ops, "step", "steps")} done${r.stopped ? " (stopped)" : ""}.${failed}${warned}${art}${more ? ` ${more.text}` : ""}`;
    undoable(text, () => undoChange(r.id), more ? 15000 : 8000, [...(more?.actions || []), { label: "History", run: () => { location.hash = "#/history"; } }]);
    return r;
  } catch (e) {
    toast(`Couldn't finish it: ${e.message || e}`, 10000);
    return null;
  }
}

/** The totals of a plan: what each kind of step does, and what's left alone. */
function Totals({ plan }) {
  return html`<div class="organise-totals" id="organise-totals">
    <p><b>${plural(plan.games, ...(COUNTS[plan.kind] || ["game", "games"]).slice(0, 2))}</b> to ${(COUNTS[plan.kind] || [])[2] || "organise"}${plan.in_place ? html`, <span class="muted">${plan.in_place} already in place</span>` : null}.</p>
    ${plan.by_op.length ? html`<ul class="ls-list">${plan.by_op.map((o) => html`<li key=${o.op} data-op=${o.op}><span>${OPS[o.op] || o.op}: ${plan.kind === "folders" ? plural(o.count, "folder", "folders") : plural(o.count, "file", "files")}</span><span class="muted">${size(o.bytes)}</span></li>`)}</ul>` : null}
    ${plan.set_aside ? html`<p class="muted">${plural(plan.set_aside, "file is", "files are")} set aside in <code>.gamerepo-set-aside</code> at the top of ${plan.set_aside === 1 ? "its" : "their"} folder, until you empty it (History).</p>` : null}
    ${plan.out ? html`<p class="muted">Into <span class="op-to" title=${plan.out}>${shortPath(plan.out, 3)}</span>${plan.new_root ? ", which becomes a folder with games" : ""}.</p>` : null}
    ${plan.space.filter((s) => s.short).map((s) => html`<p class="warn-note" role="alert" key=${s.drive}>Not enough space on ${s.drive}: ${size(s.needed)} needed, ${size(s.free)} free.</p>`)}
    ${plan.space.filter((s) => s.copied > 0).map((s) => html`<p class="muted" key=${"copied-" + s.drive}>${s.drive} can't hold hardlinks, so ${size(s.copied)} is copied instead.</p>`)}
  </div>`;
}

/** The preview: the totals, then each list a page at a time. */
export function Preview({ plan }) {
  const tabs = [["all", "Steps", plan.ops], ["present", "There already", plan.present], ["conflicts", "In the way", plan.conflicts], ["duplicates", "Duplicates", plan.duplicates], ["left", "Left alone", plan.left]].filter(([k, , n]) => k === "all" || n);
  const [what, setWhat] = useState("all");
  const [list, setList] = useState({ total: 0, items: [] });
  useEffect(() => {
    let gone = false;
    api("organise_preview", { id: plan.id, what, offset: 0, limit: PAGE }).then((r) => { if (!gone) setList(r); }).catch((e) => { if (!gone) setList({ total: 0, items: [], error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [plan.id, what]);
  const more = async () => {
    const r = await api("organise_preview", { id: plan.id, what, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  const row = (it, i) => {
    if (what === "all") {
      return html`<li key=${i} data-op=${it.op}><span class="op-kind">${OPS[it.op] || it.op}</span>
        <span class="op-paths"><span class="op-to" title=${it.to_full}>${it.to}</span>
          ${it.op !== "aside" ? it.from.map((f, k) => html`<span class="muted op-from" title=${it.from_full[k]}>from ${f}</span>`) : null}
          ${it.note ? html`<span class="muted">${it.note}</span>` : null}</span></li>`;
    }
    if (what === "conflicts") return html`<li key=${i}><span class="op-paths"><span>${it.game}</span><span class="muted">${it.to} is another file: it's left as it is, and this game isn't changed.</span></span></li>`;
    if (what === "duplicates") return html`<li key=${i}><span class="op-paths"><span title=${it.path}>${shortPath(it.path, 3)}</span><span class="muted">another copy of ${it.rom}</span></span></li>`;
    if (what === "present") return html`<li key=${i}><span class="op-paths"><span>${it.game}</span><span class="muted op-to">${it.to}</span></span></li>`;
    return html`<li key=${i}><span class="op-paths"><span title=${it.path}>${shortPath(it.path, 3)}</span><span class="muted">${it.why}</span></span></li>`;
  };
  return html`<${Totals} plan=${plan} />
    ${tabs.length > 1 ? html`<div class="seg audit-tabs" role="tablist" id="organise-tabs">${tabs.map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${what === k ? "true" : "false"} onClick=${() => setWhat(k)}>${label} <span class="tab-count">${n}</span></button>`)}</div>` : null}
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    <ul class="ls-list organise-list" id="organise-list" data-total=${list.total}>${list.items.map(row)}</ul>
    ${list.items.length < list.total ? html`<div><button type="button" class="ghost" id="organise-more" onClick=${more}>Show more (${list.total - list.items.length} left)</button></div>` : null}`;
}

/** What a change did, step by step, a page at a time: opened under its row in History
 *  and Recent imports (D78). Rows are divs, so a history's own list items stay its changes. */
export function ChangeSteps({ id }) {
  const [list, setList] = useState(null);
  useEffect(() => {
    let gone = false;
    api("change_items", { id, offset: 0, limit: PAGE }).then((r) => { if (!gone) setList(r); }).catch((e) => { if (!gone) setList({ total: 0, items: [], error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [id]);
  if (!list) return html`<p class="muted change-steps">Reading…</p>`;
  const more = async () => {
    const r = await api("change_items", { id, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  return html`<div class="change-steps" data-total=${list.total}>
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${list.items.map((it, i) => html`<div class="change-step" key=${i} data-op=${it.op}><span class="op-kind">${OPS[it.op] || it.op}</span>
      <span class="op-paths"><span class="op-to" title=${it.to_full}>${it.to}</span>
        ${it.op !== "aside" && it.from[0] ? html`<span class="muted op-from" title=${it.from_full.join("\n")}>from ${it.from[0]}${it.from.length > 1 ? ` and ${it.from.length - 1} more` : ""}</span>` : null}</span></div>`)}
    ${list.items.length < list.total ? html`<button type="button" class="ghost compact" data-action="more-steps" onClick=${more}>Show more (${list.total - list.items.length} left)</button>` : null}
  </div>`;
}

const ASIDE_PAGE = 200;
const fileName = (p) => String(p || "").split(/[\\/]/).filter(Boolean).pop() || String(p || "");

/** What a change set aside and still holds (D168): each file with its game, where it was
 *  and its size, picked to be deleted for good. `files` when they're read already. */
export function AsideFiles({ id, label, files: given = null, off }) {
  const changes = useStore(ui, (st) => st.changes);
  const [list, setList] = useState(given);
  const [picked, setPicked] = useState(() => new Set());
  const [shown, setShown] = useState(ASIDE_PAGE);
  useEffect(() => {
    if (given) { setList(given); return; }
    let gone = false;
    api("change_aside", { id }).then((r) => { if (!gone) setList(r); }).catch((e) => { if (!gone) setList({ error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [id, given, changes]);
  useEffect(() => setPicked(new Set()), [list]);
  if (!list) return html`<p class="muted change-steps">Reading…</p>`;
  if (list.error) return html`<p class="warn-note change-steps" role="alert">${list.error}</p>`;
  if (!list.length) return html`<p class="muted change-steps aside-none">Nothing of it is set aside any more.</p>`;
  const toggle = (held) => setPicked((s) => { const n = new Set(s); n.has(held) ? n.delete(held) : n.add(held); return n; });
  const all = picked.size === list.length;
  const chosen = list.filter((f) => picked.has(f.held));
  const bytes = chosen.reduce((t, f) => t + (f.size || 0), 0);
  const remove = () => confirmDialog({
    title: chosen.length === 1 ? `Delete ${fileName(chosen[0].held)}?` : `Delete ${plural(chosen.length, "file", "files")}?`,
    text: `${chosen.length === 1 ? "It's" : "They're"} deleted for good (${size(bytes)}), which can't be undone. Undo of "${label}" leaves the ${chosen.length === 1 ? "step it came from" : "steps they came from"} as ${chosen.length === 1 ? "it is" : "they are"}.`,
    button: "Delete",
    run: async () => {
      const r = await api("change_aside_delete", { id, held: chosen.map((f) => f.held) });
      toast(`Deleted ${plural(r.deleted, "file", "files")}, freeing ${size(r.freed)}.`, 6000);
      ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
    },
  });
  return html`<div class="change-steps aside-files" data-change=${id} data-total=${list.length}>
    <div class="aside-bar">
      <label class="check-row"><input type="checkbox" data-action="aside-all" checked=${all} disabled=${off} onChange=${() => setPicked(all ? new Set() : new Set(list.map((f) => f.held)))} /> All</label>
      <span class="muted">${plural(list.length, "file", "files")} · ${size(list.reduce((t, f) => t + (f.size || 0), 0))}</span>
      <button type="button" class="ghost compact" data-action="aside-delete" disabled=${off || !chosen.length} onClick=${remove}>${Icon.trash(13)} Delete${chosen.length ? ` ${chosen.length}` : ""}…</button>
    </div>
    ${list.slice(0, shown).map((f) => html`<div class="change-step aside-file" key=${f.held} data-held=${f.held}>
      <input type="checkbox" aria-label=${`Pick ${fileName(f.held)}`} checked=${picked.has(f.held)} disabled=${off} onChange=${() => toggle(f.held)} />
      <span class="op-paths"><span class="op-to" title=${f.held}>${fileName(f.held)}${f.folder ? " (a folder)" : ""}</span>
        <span class="muted op-from" title=${f.from}>${f.game ? `${f.game} · ` : ""}from ${shortPath(f.from, 3)}</span></span>
      <span class="muted aside-size">${size(f.size)}</span>
      ${isDesktop() ? html`<button type="button" class="ghost compact" data-action="aside-show" title="Show it in its folder" aria-label=${`Show ${fileName(f.held)} in its folder`} onClick=${() => reveal(f.held)}>${Icon.folder(13)}</button>` : null}
    </div>`)}
    ${list.length > shown ? html`<button type="button" class="ghost compact" data-action="more-aside" onClick=${() => setShown(shown + ASIDE_PAGE)}>Show more (${list.length - shown} left)</button>` : null}
  </div>`;
}

/** Organise…: the choices, then the preview; Run starts the change. Opened with a
 *  `plan` it goes straight to the preview (Rename these, Set aside duplicates, Remove from
 *  the library…), and `onRun` is called as it starts (a removed game's page or panel closes
 *  before the pages are refreshed). */
export function OrganiseDialog({ dat, platform, platformName, plan: given, onRun = null }) {
  const disc = !!ui.get().platforms?.find((p) => p.id === platform)?.disc;
  const saved = ctx.platform?.store?.prefs?.get("gr-organise", {}) || {};
  const [mode, setMode] = useState(saved.mode || "copy");
  const [out, setOut] = useState(saved.out || "");
  const [format, setFormat] = useState(saved.format || "keep");
  const [partial, setPartial] = useState(false);
  const [alwaysCopy, setAlwaysCopy] = useState(false);
  const [plan, setPlan] = useState(given || null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder(mode === "copy" ? "The folder the sorted copy goes to" : "The folder in the library the games go to");
    if (p) setOut(p);
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      ctx.platform?.store?.prefs?.set("gr-organise", { mode, format, out: mode === "copy" ? out : "" });
      setPlan(await api("organise_plan", { dat, mode, out: out.trim() || null, format: disc ? "keep" : format, partial, always_copy: alwaysCopy }));
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.title || plan.label} id="organise-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) { onRun?.(); runPlan(plan); } }} busy=${plan.short} submitLabel=${nothing ? "OK" : plan.kind === "aside" ? "Set them aside" : plan.kind === "pc_remove" || plan.kind === "video_remove" ? "Remove it" : "Run"}
      cancelLabel=${given ? "Cancel" : "Back"} onCancel=${given ? close : () => setPlan(null)}>
      <${Preview} plan=${plan} />
      ${nothing ? html`<p id="organise-nothing">Nothing to do: ${plan.kind === "folders" ? "no game's library folder can be renamed (the preview says why)." : plan.kind === "metadata" ? "there's none of that to delete." : plan.in_place ? "everything is in place already." : "no files match this DAT yet."}</p>`
        : plan.kind === "metadata" ? html`<p class="muted">The files are set aside, not deleted, and the games' details made again from what's left; it can be undone.</p>
          <${More} id="organise-more"><p>Your own documents and what you've written yourself stay. Undo is in the message afterwards and in History, which also empties what was set aside.</p><//>`
        : plan.kind === "folders" ? html`<p class="muted">Each game's library folder is renamed where it is, with its details, pictures and documents; it can be undone.</p>`
        : plan.kind === "pc_remove" ? html`<p class="muted" id="pc-remove-note">Its files, details, pictures and documents are set aside, not deleted, and Undo brings it all back.</p>
          ${plan.steam ? html`<p class="muted" id="pc-remove-steam">Add to Steam… made it a Steam shortcut, which stays in Steam until you run Add to Steam… again.</p>` : null}
          <${More} id="organise-more"><p>It goes from PC games and the Library. Undo is in the message afterwards and in History, which also empties what was set aside.</p><//>`
        : html`<p class="muted">Every file written is checked before anything is set aside, and the change can be undone.</p>`}
    <//>`;
  }
  const modeHelp = MODES.find(([m]) => m === mode)?.[2];
  return html`<${Dialog} title=${`Organise ${platformName || "these games"}`} id="organise-dialog" onSubmit=${preview} busy=${busy || (mode === "copy" && !out.trim())} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <div class="field-block" role="radiogroup" aria-label="How">
      ${MODES.map(([m, label]) => html`<label class="check-row" key=${m}><input type="radio" name="organise-mode" value=${m} checked=${mode === m} onChange=${() => setMode(m)} /> ${label}</label>`)}
      <small class="muted" id="organise-mode-help">${modeHelp}</small>
    </div>
    ${mode !== "rename" ? html`<div class="field-block"><span class="field-label">${mode === "copy" ? "Copy to" : "Move to (optional)"}</span>
      <div class="pick-row">
        <input type="text" id="organise-out" placeholder=${mode === "copy" ? "A folder for the sorted copy" : `The library's ${platformName || "system's"} folder`} value=${out} onInput=${(e) => setOut(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="organise-pick" onClick=${pick}>Choose…</button>` : null}
      </div></div>` : null}
    ${disc ? null : html`<label class="field-block"><span class="field-label">Format</span>
      <select id="organise-format" class="compact" value=${format} onChange=${(e) => setFormat(e.target.value)}>
        ${FORMATS.map(([v, label]) => html`<option key=${v} value=${v}>${label}</option>`)}
      </select></label>`}
    <label class="check-row"><input type="checkbox" id="organise-partial" checked=${partial} onChange=${(e) => setPartial(e.target.checked)} /> Include games partly there</label>
    ${mode === "copy" ? html`<label class="check-row"><input type="checkbox" id="organise-copy" checked=${alwaysCopy} onChange=${(e) => setAlwaysCopy(e.target.checked)} /> <span title="Hardlinks are a second name for the same file on the same drive: they take no space">Always copy (no hardlinks)</span></label>` : null}
    <p class="muted">Nothing changes until you've seen the preview and pressed Run.</p>
    <${More} id="organise-more">
      ${disc ? html`<p id="organise-disc-note">Disc images are kept as they are (CHDs, ISOs and cue/bin sets, with their cue sheets rewritten to the new names). Convert… changes their format.</p>`
        : html`<p>ZIPs are written the TorrentZip way, so the same game gives the same bytes as other collectors' tools.</p>`}
      <p>Names follow Settings › Naming.</p>
    <//>
  <//>`;
}

/** Convert…: the format (the platform's preferred one first) or Extract, into a folder
 *  or in place; then the same preview as Organise… */
export function ConvertDialog({ dat, platform, platformName, games }) {
  const info = ui.get().platforms?.find((p) => p.id === platform) || {};
  const formats = info.formats || [];
  const saved = ctx.platform?.store?.prefs?.get("gr-convert", {}) || {};
  const [to, setTo] = useState(formats[0] || "extract");
  const [place, setPlace] = useState(saved.place || "folder");
  const [out, setOut] = useState(saved.out || "");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let gone = false;
    api("formats").then((all) => {
      const p = all.find((x) => x.platform === platform)?.preferred;
      if (!gone && p) setTo((t) => (t === formats[0] ? p : t));
    }).catch(() => {});
    return () => { gone = true; };
  }, [platform]);
  const what = games?.length === 1 ? games[0] : games?.length ? plural(games.length, "game", "games") : `${platformName || "this system"}'s games`;
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder("The folder the converted games go to");
    if (p) setOut(p);
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      ctx.platform?.store?.prefs?.set("gr-convert", { place, out });
      setPlan(await api("convert_plan", { dat, to, games: games?.length ? games : null, out: place === "folder" ? out.trim() : null }));
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.title || plan.label} id="convert-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} busy=${plan.short} submitLabel=${nothing ? "OK" : "Run"}
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      ${nothing ? html`<p id="organise-nothing">Nothing to do: ${plan.in_place ? "everything is in that format already." : "no files of these games are here."}</p>` : html`<p class="muted">Each result is checked against the files it came from before anything is replaced, and the change can be undone.</p>`}
    <//>`;
  }
  const disc = !!info.disc;
  const extractHelp = disc ? "Back to the files Redump lists: cue/bin for CDs, an ISO for DVDs, GameCube, Wii and PSP." : "Unpacked from their ZIPs into loose files.";
  return html`<${Dialog} title=${`Convert ${what}`} id="convert-dialog" onSubmit=${preview} busy=${busy || (place === "folder" && !out.trim())} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <label class="field-block"><span class="field-label">To</span>
      <select id="convert-to" class="compact" value=${to} onChange=${(e) => setTo(e.target.value)}>
        ${formats.map((f) => html`<option key=${f} value=${f}>${CONVERT_FORMATS[f]?.[0] || f.toUpperCase()}</option>`)}
        <option value="extract">${disc ? "Unpack (cue/bin or ISO)" : "Loose files"}</option>
      </select>
      <small class="muted" id="convert-help">${to === "extract" ? extractHelp : CONVERT_FORMATS[to]?.[1]}</small></label>
    <div class="field-block" role="radiogroup" aria-label="Where">
      <label class="check-row"><input type="radio" name="convert-place" value="folder" checked=${place === "folder"} onChange=${() => setPlace("folder")} /> Into a folder (your files stay as they are)</label>
      <label class="check-row"><input type="radio" name="convert-place" value="here" checked=${place === "here"} onChange=${() => setPlace("here")} /> Where they are (the originals are set aside, not deleted)</label>
    </div>
    ${place === "folder" ? html`<div class="field-block"><span class="field-label">Into</span>
      <div class="pick-row">
        <input type="text" id="convert-out" placeholder="A folder for the converted games" value=${out} onInput=${(e) => setOut(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="convert-pick" onClick=${pick}>Choose…</button>` : null}
      </div></div>` : null}
    <p class="muted">Names follow Settings › Naming. Nothing changes until you've seen the preview and pressed Run.</p>
  <//>`;
}
