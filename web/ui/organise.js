// Organising a set by its DAT (docs/PLAN.md, "Phase 2 design", 2a): the Organise…
// dialog (how, where and in what format), its preview from the index alone, then
// the change as a job whose message has Undo. "Rename these" (Misnamed) and "Set
// aside duplicates" (Duplicates) open the same preview. The history of changes is
// in Settings (settings.js), from the commands here.
import { html, useState, useEffect } from "../lib/html.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { api, toast, followJob, undoable, isDesktop, plural, size } from "./library.js";
import { refreshGames } from "./games.js";
import { Dialog, close } from "./dialogs.js";

const PAGE = 100;

export const MODES = [
  ["copy", "Copy to a folder", "Your files stay as they are; the sorted set is written to a folder you choose (hardlinked when it's on the same drive, so it takes no space)."],
  ["rename", "Rename where they are", "Files are renamed in their folders to their DAT names. Extra copies and replaced archives are set aside, not deleted."],
  ["move", "Move into the library", "Files are moved into a folder for the platform in the library, named by the DAT. What they replace is set aside, not deleted."],
];

export const FORMATS = [
  ["keep", "As they are"],
  ["zip", "A ZIP for each game (TorrentZip)"],
  ["loose", "Loose files"],
];

const OPS = { rename: "Rename", move: "Move", copy: "Copy", link: "Hardlink", zip: "Write ZIP", extract: "Unpack", aside: "Set aside" };

/** Open the Organise… dialog for a DAT: `{ dat, platform, platformName }`. */
export const organiseDialog = (opts) => ui.set({ dialog: { type: "organise", ...opts } });

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
    toast(`Couldn't plan it: ${e.message || e}`, 8000);
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
  return r.problems?.length ? `Put back ${plural(r.undone, "step", "steps")}; ${r.problems.length} couldn't be: ${r.problems[0]}` : `Undone: ${r.label}.`;
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
      ms: 30000, actions: [{ label: "Finish it", run: () => run(finishChange) }, ...(c.can_undo ? [{ label: "Put it back", run: () => run(undoChange) }] : [])],
    });
  } catch { /* Settings › History offers it too */ }
}

/** Run a planned change, then say what it did, with Undo. */
async function runPlan(plan) {
  try {
    const { job } = await api("organise_run", { id: plan.id });
    const done = await followJob(job, plan.kind === "aside" ? "Setting aside duplicates" : "Organising");
    await refreshGames();
    ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    const failed = r.failed?.length ? ` ${plural(r.failed.length, "step", "steps")} failed: ${r.failed[0].error}` : "";
    const text = `${plan.label}: ${r.done} of ${plural(r.ops, "step", "steps")} done${r.stopped ? " (stopped)" : ""}.${failed}`;
    undoable(text, () => undoChange(r.id), 15000, [{ label: "History", run: () => { location.hash = "#/settings"; setTimeout(() => document.getElementById("settings-history")?.scrollIntoView(), 200); } }]);
  } catch (e) {
    toast(`Couldn't finish it: ${e.message || e}`, 10000);
  }
}

/** The totals of a plan: what each kind of step does, and what's left alone. */
function Totals({ plan }) {
  return html`<div class="organise-totals" id="organise-totals">
    <p><b>${plural(plan.games, "game", "games")}</b> to change${plan.in_place ? html`, <span class="muted">${plan.in_place} already in place</span>` : null}.</p>
    ${plan.by_op.length ? html`<ul class="ls-list">${plan.by_op.map((o) => html`<li key=${o.op} data-op=${o.op}><span>${OPS[o.op] || o.op}: ${plural(o.count, "file", "files")}</span><span class="muted">${size(o.bytes)}</span></li>`)}</ul>` : null}
    ${plan.set_aside ? html`<p class="muted">${plural(plan.set_aside, "file is", "files are")} set aside in <code>.gamerepo-set-aside</code> at the top of ${plan.set_aside === 1 ? "its" : "their"} folder, until you empty it (Settings › History).</p>` : null}
    ${plan.out ? html`<p class="muted">Into <span class="op-to">${plan.out}</span>${plan.new_root ? ", which becomes a folder with games" : ""}.</p>` : null}
    ${plan.space.filter((s) => s.short).map((s) => html`<p class="warn-note" role="alert" key=${s.drive}>Not enough space on ${s.drive}: ${size(s.needed)} needed, ${size(s.free)} free.</p>`)}
  </div>`;
}

/** The preview: the totals, then each list a page at a time. */
function Preview({ plan }) {
  const tabs = [["all", "Steps", plan.ops], ["conflicts", "In the way", plan.conflicts], ["duplicates", "Duplicates", plan.duplicates], ["left", "Left alone", plan.left]].filter(([k, , n]) => k === "all" || n);
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
    if (what === "duplicates") return html`<li key=${i}><span class="op-paths"><span>${it.path}</span><span class="muted">another copy of ${it.rom}</span></span></li>`;
    return html`<li key=${i}><span class="op-paths"><span>${it.path}</span><span class="muted">${it.why}</span></span></li>`;
  };
  return html`<${Totals} plan=${plan} />
    ${tabs.length > 1 ? html`<div class="seg audit-tabs" role="tablist" id="organise-tabs">${tabs.map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${what === k ? "true" : "false"} onClick=${() => setWhat(k)}>${label} <span class="tab-count">${n}</span></button>`)}</div>` : null}
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    <ul class="ls-list organise-list" id="organise-list" data-total=${list.total}>${list.items.map(row)}</ul>
    ${list.items.length < list.total ? html`<div><button type="button" class="ghost" id="organise-more" onClick=${more}>Show more (${list.total - list.items.length} left)</button></div>` : null}`;
}

/** Organise…: the choices, then the preview; Run starts the change. Opened with a
 *  `plan` it goes straight to the preview (Rename these, Set aside duplicates). */
export function OrganiseDialog({ dat, platformName, plan: given }) {
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
    const p = await ctx.platform.library.pickFolder(mode === "copy" ? "The folder the sorted set goes to" : "The folder in the library the set goes to");
    if (p) setOut(p);
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      ctx.platform?.store?.prefs?.set("gr-organise", { mode, format, out: mode === "copy" ? out : "" });
      setPlan(await api("organise_plan", { dat, mode, out: out.trim() || null, format, partial, always_copy: alwaysCopy }));
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.label} id="organise-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} busy=${plan.short} submitLabel=${nothing ? "OK" : plan.kind === "aside" ? "Set them aside" : "Run"}
      cancelLabel=${given ? "Cancel" : "Back"} onCancel=${given ? close : () => setPlan(null)}>
      <${Preview} plan=${plan} />
      ${nothing ? html`<p id="organise-nothing">Nothing to do: ${plan.in_place ? "everything is in place already." : "no files match this DAT yet."}</p>` : html`<p class="muted">Every file written is read back and checked before anything is set aside. The change can be undone from the message afterwards or from Settings › History.</p>`}
    <//>`;
  }
  const modeHelp = MODES.find(([m]) => m === mode)?.[2];
  return html`<${Dialog} title=${`Organise ${platformName || "this set"}`} id="organise-dialog" onSubmit=${preview} busy=${busy || (mode === "copy" && !out.trim())} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <div class="field-block" role="radiogroup" aria-label="How">
      ${MODES.map(([m, label]) => html`<label class="check-row" key=${m}><input type="radio" name="organise-mode" value=${m} checked=${mode === m} onChange=${() => setMode(m)} /> ${label}</label>`)}
      <small class="muted" id="organise-mode-help">${modeHelp}</small>
    </div>
    ${mode !== "rename" ? html`<div class="field-block"><span class="field-label">${mode === "copy" ? "Copy to" : "Move to (optional)"}</span>
      <div class="pick-row">
        <input type="text" id="organise-out" placeholder=${mode === "copy" ? "A folder for the sorted set" : `The library's ${platformName || "platform"} folder`} value=${out} onInput=${(e) => setOut(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="organise-pick" onClick=${pick}>Choose…</button>` : null}
      </div></div>` : null}
    <label class="field-block"><span class="field-label">Format</span>
      <select id="organise-format" class="compact" value=${format} onChange=${(e) => setFormat(e.target.value)}>
        ${FORMATS.map(([v, label]) => html`<option key=${v} value=${v}>${label}</option>`)}
      </select>
      <small class="muted">ZIPs are written the TorrentZip way, so the same game gives the same bytes as other collectors' tools. Names follow Settings › Naming.</small></label>
    <label class="check-row"><input type="checkbox" id="organise-partial" checked=${partial} onChange=${(e) => setPartial(e.target.checked)} /> Include partly-there games</label>
    ${mode === "copy" ? html`<label class="check-row"><input type="checkbox" id="organise-copy" checked=${alwaysCopy} onChange=${(e) => setAlwaysCopy(e.target.checked)} /> Always copy (no hardlinks)</label>` : null}
    <p class="muted">Nothing changes until you've seen the preview and pressed Run.</p>
  <//>`;
}
