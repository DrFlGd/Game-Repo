// PC mods (docs/PLAN.md, "Cheats and patches", D205): the Patches page's third tab. A mod is
// kept with its PC game in the library (its metadata folder's patches/mods/), as it came
// (an archive, a folder, an installer), with how it goes in: into the game's folder, into a
// folder of the game's, a file to run (never run by Game Repo), or by hand. Mods have an
// order. Install the mods puts the ticked ones where the game is installed on this computer,
// what they replace set aside first, as a change History can undo; never into the library.
// Check the game's files… compares where it's installed with the library's copy and puts it
// back, after mods put in by Game Repo or by hand ("Mods on installed games", D207–D208).
// A mod put in by hand is recorded as installed (what changed, as its), taken off by
// itself, and kept in the library as a mod that goes in by itself (D210). A mod with a FOMOD
// installer has its options chosen in a dialog like the installer's, one step at a time,
// and goes in as they say (D211).
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, isDesktop, libraryUrl, toast } from "./library.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, byName, askGame } from "./shelf.js";
import { reveal } from "./games.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
const size = (n) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round((n || 0) / 1e3))} KB`);
const HOWS = [["game", "Into the game's folder"], ["folder", "Into a folder of the game's"], ["run", "A file to run"], ["hand", "By hand"]];
const howName = (h) => HOWS.find((x) => x[0] === h)?.[1] || "By hand";
const installed = (p) => (p.installed?.to ? "Installed" : "Not installed");

/** Add a mod…: its archive, installer or file (or its folder, `folder`) for `key`'s PC game. */
export async function importMods({ key, folder = false, onDone = null }) {
  const lib = ctx.platform.library;
  const paths = folder ? [await lib.pickFolder("The mod's folder")].filter(Boolean) : (await lib.pickFiles("Mods: their archives (.zip, .7z), installers or files")) || [];
  if (!paths.length) return;
  ui.set({ dialog: { type: "mods-import", k: key, paths, onDone } });
}

function ImportDialog({ k, paths, onDone }) {
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("mods_import", { key: k, paths, how }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="mods-import-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Bring it in"
      cancelLabel="Back" onCancel=${() => setPlan(null)} onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
      <${Preview} plan=${plan} />
      <p class="muted">The mod goes with the game in the library (<code>patches/mods/</code>), with a readme found beside it. How it goes in is guessed: check it on the game's mods. History can undo it.</p>
    <//>`;
  }
  return html`<${Dialog} title=${paths.length === 1 ? "Add a mod" : `Add ${paths.length} mods`} id="mods-import-dialog" onSubmit=${preview} busy=${busy} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <ul class="ls-list" id="mods-import-paths">${paths.map((p) => html`<li key=${p}><code>${p}</code></li>`)}</ul>
    <fieldset class="field-block" id="mods-import-how"><legend class="field-label">How</legend>
      <label class="check-row"><input type="radio" name="mods-how" value="copy" checked=${how === "copy"} onChange=${() => setHow("copy")} /> Hardlinked on the same drive, else copied: the download stays where it is</label>
      <label class="check-row"><input type="radio" name="mods-how" value="move" checked=${how === "move"} onChange=${() => setHow("move")} /> Moved into the library</label>
    </fieldset>
  <//>`;
}

/** Install the mods: the preview of the ticked mods going where the game is installed. */
function InstallDialog({ plan, onDone, takeOff = false, hand = false }) {
  const skipped = hand ? [] : plan.skipped || [];
  return html`<${Dialog} title=${plan.title || plan.label} id="mods-install-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel=${takeOff ? "Take it off" : "Install them"}
    onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
    <${Preview} plan=${plan} />
    ${hand
      ? html`<p class="muted" id="mods-install-to">In <code>${plan.to}</code>: the files it changed or added as recorded are set aside, and the library's copy of each put back${plan.skipped?.length ? html` (not ${plan.skipped.join(", ")}: no copy of the game's own, left as it is)` : ""}. History's Undo puts it on again.</p>`
      : takeOff
      ? html`<p class="muted" id="mods-install-to">In <code>${plan.to}</code>: its files are set aside, and each file it had replaced is put back (another installed mod's, the library's copy, or the game's own the install set aside). History's Undo puts it on again.</p>`
      : html`<p class="muted" id="mods-install-to">Into <code>${plan.to}</code>, in their order: a later mod's file goes over an earlier one's. The game's files they replace are set aside, and History's Undo puts them back.</p>`}
    ${skipped.length ? html`<p class="warn-note" id="mods-install-skipped">Not by Game Repo: ${skipped.map((x) => `${x.name} (${howName(x.how).toLowerCase()})`).join(", ")}. Game Repo never runs a mod's installer: run it yourself, after these.</p>` : null}
  <//>`;
}

/** The files a check found, each ticked to be put back (D207–D208): the unticked are left
 *  as they are and kept as the game's own. Game Repo Go's page shows the same lists. */
export function CheckLists({ check, leave, setLeave, off = false }) {
  const groups = [["changed", "Changed since it was installed", "copied again from the library"], ["missing", "Gone", "copied again from the library"], ["added", "Added", "set aside, not deleted"]];
  const flip = (p, on) => setLeave(on ? leave.filter((x) => x !== p) : [...leave, p]);
  return html`<div class="pc-check-lists">${groups.filter(([k]) => check[k]?.length).map(([k, label, what]) => html`<fieldset class="field-block" key=${k} data-list=${k}>
    <legend class="field-label">${label} (${check[k].length}): ${what}</legend>
    <ul class="ls-list">${check[k].map((p) => html`<li key=${p}><label class="check-row"><input type="checkbox" data-file=${p} checked=${!leave.includes(p)} disabled=${off} onChange=${(e) => flip(p, e.target.checked)} /> <code>${p}</code></label></li>`)}</ul>
  </fieldset>`)}
  ${check.kept || check.played ? html`<p class="muted" data-kept=${check.kept || 0}>${[check.kept ? `${check.kept} kept as the game's own` : null, check.played ? `${check.played} the game made as it was played (logs, caches), not listed` : null].filter(Boolean).join("; ")}.</p>` : null}
  </div>`;
}

const checkSay = (c) => (c.state === "as_installed" ? (c.hand?.length ? `As it was installed, with ${c.hand.join(", ")} put in by hand.` : "As it was installed: nothing has changed.") : c.state === "not_there" ? "Its folder isn't there." : `Changed: ${[c.changed.length ? `${c.changed.length} changed` : null, c.missing.length ? `${c.missing.length} gone` : null, c.added.length ? `${c.added.length} added` : null].filter(Boolean).join(", ")}.`);

/** Check the game's files: where it's installed against the library's copy (D207), then
 *  Put it back… as a change with Undo (D208). */
function CheckDialog({ k, title, check, onDone }) {
  const [leave, setLeave] = useState([]);
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const all = [...check.changed, ...check.missing, ...check.added];
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="mods-put-back-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Put it back"
      cancelLabel="Back" onCancel=${() => setPlan(null)} onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
      <${Preview} plan=${plan} />
      <p class="muted">In <code>${plan.to}</code>: what changed or was added is set aside first, so History's Undo puts it all back as it is now.</p>
    <//>`;
  }
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("mods_put_back", { key: k, leave }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`${title}'s files`} id="mods-check-dialog" wide=${true} onSubmit=${all.length ? preview : close} busy=${busy || (all.length > 0 && leave.length === all.length)} error=${error}
    submitLabel=${all.length ? (busy ? "Planning…" : "Put it back…") : "Close"}>
    <p id="mods-check-state" data-state=${check.state}>${checkSay(check)}</p>
    <p class="muted">Compared with the library's copy, in <code>${check.folder}</code>. Untick a file to leave it as it is: it's kept as the game's own from then on.</p>
    <${CheckLists} check=${check} leave=${leave} setLeave=${setLeave} />
  <//>`;
}

const TYPE_SAY = { Required: "Required", Recommended: "Recommended", NotUsable: "Not usable", CouldBeUsable: "Could be usable" };

/** Choose its options… (D211): a FOMOD's steps one at a time, each group's options picked as
 *  the installer would ask (one, at most one, at least one, all, any), the questions on
 *  other mods' files first; the choices kept with the mod, and it goes in as they say. */
function FomodDialog({ k, name, first, onDone }) {
  const [r, setR] = useState(first);
  const [choices, setChoices] = useState(first.choices || { picks: {}, files: {} });
  const [at, setAt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const steps = r.steps.map((s, i) => ({ ...s, i })).filter((s) => s.visible && s.groups.some((g) => g.options.length));
  const step = steps[Math.min(at, steps.length - 1)];
  const last = at >= steps.length - 1;
  const again = async (c) => {
    setChoices(c);
    try {
      setR(await api("mods_fomod", { key: k, name, choices: c }));
      setError("");
    } catch (e) {
      setError(err(e));
    }
  };
  const pick = (g, o, on) => {
    const one = g.kind === "SelectExactlyOne" || g.kind === "SelectAtMostOne";
    const now = g.options.filter((x) => x.picked).map((x) => x.name);
    const next = one ? (on ? [o.name] : []) : on ? [...now.filter((x) => x !== o.name), o.name] : now.filter((x) => x !== o.name);
    again({ ...choices, picks: { ...choices.picks, [g.id]: next } });
  };
  const answer = (file, v) => again({ ...choices, files: { ...choices.files, [file]: v } });
  const finish = async () => {
    // every group's picks as shown, so the choices kept say all of them
    const picks = {};
    for (const s of r.steps) for (const g of s.groups) picks[g.id] = g.options.filter((o) => o.picked).map((o) => o.name);
    setBusy(true);
    setError("");
    try {
      const g = await api("mods_fomod_choose", { key: k, name, choices: { ...choices, picks } });
      close();
      onDone?.(g);
      toast(`${r.name || name} goes in as you chose: tick it and Install the ticked mods… to put it in.`, 8000);
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const pic = (p) => (p ? html`<img class="fomod-pic" src=${libraryUrl(p)} alt="" />` : null);
  return html`<${Dialog} title=${`${r.name || name}: its options`} id="mods-fomod-dialog" wide=${true} sticky=${true} busy=${busy || !!r.cannot} error=${error || r.cannot}
    submitLabel=${last ? (busy ? "Saving…" : "Use these options") : "Next"} onSubmit=${last ? finish : () => setAt(at + 1)}
    cancelLabel=${at > 0 ? "Back" : "Cancel"} onCancel=${at > 0 ? () => setAt(at - 1) : close}>
    ${at === 0 && r.image ? html`<div class="fomod-head">${pic(r.image)}</div>` : null}
    ${at === 0 && r.questions.length ? html`<fieldset class="field-block" id="fomod-questions"><legend class="field-label">It asks about other mods' files (Game Repo can't tell whether another mod's plugin is active)</legend>
      ${r.questions.map((q) => html`<label class="fomod-q" key=${q.file}><code>${q.file}</code>
        <select aria-label=${`Is ${q.file} there?`} data-file=${q.file} value=${q.answer} onChange=${(e) => answer(q.file, e.target.value)}><option value="Active">There and active</option><option value="Inactive">There, not active</option><option value="Missing">Not there</option></select></label>`)}
    </fieldset>` : null}
    ${step ? html`<p class="muted" id="fomod-step" data-step=${step.i}>Step ${Math.min(at, steps.length - 1) + 1} of ${steps.length}: <strong>${step.name}</strong></p>
      ${step.groups.filter((g) => g.options.length).map((g) => {
        const one = g.kind === "SelectExactlyOne" || g.kind === "SelectAtMostOne";
        const say = { SelectExactlyOne: "pick one", SelectAtMostOne: "pick one or none", SelectAtLeastOne: "pick at least one", SelectAll: "all of these", SelectAny: "pick any" }[g.kind] || "pick any";
        return html`<fieldset class="field-block fomod-group" key=${g.id} data-group=${g.id}><legend class="field-label">${g.name} <span class="muted">(${say})</span></legend>
          ${g.options.map((o) => {
            const fixed = o.type === "Required" || o.type === "NotUsable" || g.kind === "SelectAll";
            return html`<label class=${`fomod-option${o.picked ? " on" : ""}`} key=${o.name} data-option=${o.name} data-type=${o.type}>
              <input type=${one ? "radio" : "checkbox"} name=${`fomod-${g.id}`} checked=${o.picked} disabled=${fixed || busy}
                onClick=${one && o.picked && g.kind === "SelectAtMostOne" ? (e) => { e.preventDefault(); pick(g, o, false); } : null}
                onChange=${(e) => pick(g, o, e.target.checked)} />
              <span class="fomod-text"><strong>${o.name}</strong>${TYPE_SAY[o.type] ? html` <span class=${`fomod-type ${o.type}`}>${TYPE_SAY[o.type]}</span>` : null}
                ${o.description ? html`<span class="muted fomod-desc">${o.description}</span>` : null}</span>
              ${pic(o.image)}
            </label>`;
          })}
        </fieldset>`;
      })}` : html`<p class="muted">It asks nothing: its files go in as they are.</p>`}
    ${last ? html`<p class="muted" id="fomod-files">${r.files} ${r.files === 1 ? "file or folder goes" : "files or folders go"} in with these options.</p>` : null}
  <//>`;
}

/** Edit details…: a mod's title, author, version, address, notes and the folder in its archive. */
function EditDialog({ k, file, details, onDone }) {
  const [d, setD] = useState(details);
  const [error, setError] = useState("");
  const f = (key, label, ph = "") => html`<label class="field-block"><span class="field-label">${label}</span>
    <input type="text" data-field=${key} value=${d[key] || ""} placeholder=${ph} onInput=${(e) => setD({ ...d, [key]: e.target.value })} /></label>`;
  const save = () => api("mods_edit", { key: k, name: file, details: d }).then((g) => { close(); onDone?.(g); again(); }).catch((e) => setError(err(e)));
  return html`<${Dialog} title=${`Edit ${file}`} id="mods-edit-dialog" onSubmit=${save} error=${error} submitLabel="Save">
    ${f("title", "Title")}${f("author", "Author")}${f("version", "Version")}${f("url", "Where it came from", "https://…")}
    ${f("inside", "The folder in its archive its files are in", "Leave empty for the archive's top")}
    <label class="field-block"><span class="field-label">Notes</span><textarea data-field="notes" rows="3" value=${d.notes || ""} onInput=${(e) => setD({ ...d, notes: e.target.value })}></textarea></label>
  <//>`;
}

export function ModDialogs({ dialog }) {
  if (dialog.type === "mods-import") return html`<${ImportDialog} ...${dialog} />`;
  if (dialog.type === "mods-install") return html`<${InstallDialog} ...${dialog} />`;
  if (dialog.type === "mods-edit") return html`<${EditDialog} ...${dialog} />`;
  if (dialog.type === "mods-check") return html`<${CheckDialog} ...${dialog} />`;
  if (dialog.type === "mods-fomod") return html`<${FomodDialog} ...${dialog} />`;
  return null;
}

/** A PC game's mods in order: each ticked or not, how it goes in, moved up or down. */
export function ModsEditor({ k, off, focus = null }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const load = () => api("mods_game", { key: k }).then((x) => { setG(x); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { setG(null); load(); }, [k]);
  if (error) return html`<p class="warn-note">${error}</p>`;
  if (!g) return html`<p class="muted">Reading…</p>`;
  const list = g.mods || [];
  const set = (m, patch) => api("mods_edit", { key: k, name: m.name, details: { ...m.details, ...patch } }).then(setG).catch((e) => toast(err(e), 8000));
  const move = (i, by) => {
    const names = list.map((m) => m.name);
    const [x] = names.splice(i, 1);
    names.splice(i + by, 0, x);
    api("mods_order", { key: k, names }).then(setG).catch((e) => toast(err(e), 8000));
  };
  const where = async () => {
    const f = await ctx.platform.library.pickFolder(`Where ${g.title} is installed`);
    if (!f) return;
    try {
      await api("pc_update", { id: k.replace(/^pc\//, ""), patch: { installed: f } });
      load();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const install = async () => {
    try {
      ui.set({ dialog: { type: "mods-install", plan: await api("mods_install", { key: k }), onDone: load } });
    } catch (e) {
      toast(`Couldn't install them: ${err(e)}`, 10000);
    }
  };
  const takeOff = async (m) => {
    try {
      ui.set({ dialog: { type: "mods-install", plan: await api("mods_uninstall", { key: k, name: m.name }), onDone: load, takeOff: true } });
    } catch (e) {
      toast(`Couldn't take it off: ${err(e)}`, 10000);
    }
  };
  const record = async (m, again) => {
    try {
      const fp = await api("mods_record", { key: k, name: m.name });
      toast(`${again ? "Recorded again" : "Recorded"}: ${fp.files.length} changed or added${fp.gone.length ? `, ${fp.gone.length} taken away` : ""}, as ${m.details.title || m.name}'s.`, 8000);
      load();
    } catch (e) {
      toast(err(e), 10000);
    }
  };
  const takeOffHand = async (m) => {
    try {
      ui.set({ dialog: { type: "mods-install", plan: await api("mods_put_back", { key: k, leave: [], only: m.name }), onDone: load, takeOff: true, hand: true } });
    } catch (e) {
      toast(`Couldn't take it off: ${err(e)}`, 10000);
    }
  };
  const keepHand = async (m) => {
    try {
      const r = await api("mods_keep_recorded", { key: k, name: m.name });
      toast(`Kept as ${r.name}: it goes in by itself now (tick it to install it).`, 8000);
      load();
      again();
    } catch (e) {
      toast(err(e), 10000);
    }
  };
  const chooseFomod = async (m) => {
    try {
      ui.set({ dialog: { type: "mods-fomod", k, name: m.name, first: await api("mods_fomod", { key: k, name: m.name }), onDone: setG } });
    } catch (e) {
      toast(`Couldn't read its installer: ${err(e)}`, 10000);
    }
  };
  const check = async () => {
    try {
      ui.set({ dialog: { type: "mods-check", k, title: g.title, check: await api("mods_check", { key: k }), onDone: load } });
    } catch (e) {
      toast(`Couldn't check it: ${err(e)}`, 10000);
    }
  };
  const ticked = list.filter((m) => m.details.on && (m.details.how === "game" || m.details.how === "folder")).length;
  return html`<div class="patches-editor" id="mods-editor" data-key=${k}>
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" id="mods-add" disabled=${off || !isDesktop()} onClick=${() => importMods({ key: k, onDone: load })}>${Icon.plus(14)} Add a mod…</button>
      <button type="button" class="ghost compact" id="mods-add-folder" disabled=${off || !isDesktop()} onClick=${() => importMods({ key: k, folder: true, onDone: load })}>Add a mod's folder…</button>
    </div>
    <p class="muted" id="mods-where" data-found=${g.installed_found ? "yes" : "no"}>${g.installed_at
      ? html`Installed on this computer in <code>${g.installed_at}</code>${g.installed_found ? "" : html` <span class="warn-note">(not there now)</span>`}.`
      : "Mods go where the game is installed, never into the library's copy: say where it's installed on this computer to install them here."}
      ${isDesktop() && !off ? html` <button type="button" class="ghost compact" id="mods-where-choose" onClick=${where}>${g.installed_at ? "Change…" : "Where it's installed…"}</button>` : null}</p>
    ${list.length || g.installed_found ? html`<div class="lib-panel-actions">
      ${list.length ? html`<button type="button" class="primary compact" id="mods-install" disabled=${off || !ticked || !g.installed_found || !isDesktop()} onClick=${install}>Install the ticked mods…</button>` : null}
      ${g.installed_found ? html`<button type="button" class="ghost compact" id="mods-check" disabled=${off || !isDesktop()} title="Compare where it's installed with the library's copy, and put it back" onClick=${check}>Check the game's files…</button>` : null}
    </div>` : null}
    ${!list.length ? html`<p class="muted" id="mods-none">${g.title} has no mods yet. Add a mod… brings one in: its archive, its installer or its folder.</p>` : null}
    ${list.map((m, i) => html`<section class=${`patch-file${focus === m.name ? " on" : ""}`} key=${m.name} data-file=${m.name} data-how=${m.details.how}>
      <h3><label class="check-row"><input type="checkbox" data-action="on" aria-label=${`Install ${m.name}`} checked=${m.details.on} disabled=${off || m.details.how === "run" || m.details.how === "hand" || (m.details.fomod && !m.details.choices)} onChange=${(e) => set(m, { on: e.target.checked })} /> ${i + 1}. ${m.details.title || m.name}</label></h3>
      <p class="muted">${[m.name, m.archive ? "an archive" : m.folder ? "a folder" : "a file", size(m.size), m.details.version ? `v${m.details.version}` : null, m.details.author ? `by ${m.details.author}` : null].filter(Boolean).join(" · ")}</p>
      <span class="patch-found-row">
        <select class="compact" aria-label="How it goes in" data-action="how" value=${m.details.how} disabled=${off} onChange=${(e) => set(m, { how: e.target.value, on: e.target.value === "game" || (e.target.value === "folder" && !!m.details.folder) })}>${HOWS.map(([v, l]) => html`<option value=${v} key=${v}>${l}</option>`)}</select>
        ${m.details.how === "folder" ? html`<input type="text" class="compact" aria-label="The game's folder it goes into" data-action="folder" placeholder="Mods/" value=${m.details.folder} disabled=${off} onChange=${(e) => set(m, { folder: e.target.value, on: !!e.target.value })} />` : null}
      </span>
      ${m.details.fomod ? html`<p class=${m.details.choices ? "muted" : "warn-note"} data-fomod=${m.details.choices ? "chosen" : "to-choose"}>${m.details.choices ? `Its installer's options are chosen: ${(m.details.files || []).length} files or folders go in.` : "It has an installer (FOMOD): choose its options to put it in."}
        <button type="button" class="ghost compact" data-action="fomod" disabled=${off || !isDesktop()} onClick=${() => chooseFomod(m)}>${m.details.choices ? "Change the choices…" : "Choose its options…"}</button></p>` : null}
      ${m.details.how === "run" ? html`<p class="muted">Game Repo never runs it: run it yourself once the game is installed (and its other mods are in).</p>` : null}
      ${m.details.how === "hand" ? html`<p class="muted">Kept here: put it in as its readme says, then say <em>I've installed it</em> so what changed is recorded as its.</p>` : null}
      ${(m.details.how === "hand" || m.details.how === "run") && g.installed_found ? html`<div class="lib-panel-actions mods-hand" data-recorded=${g.hand?.[m.name] ? "yes" : "no"}>
        ${isDesktop() ? html`<button type="button" class="ghost compact" data-action="open-game" onClick=${() => reveal(g.installed_at)}>Open the game's folder</button>
          <button type="button" class="ghost compact" data-action="open-mod" onClick=${() => reveal(m.path)}>Show the mod</button>` : null}
        ${g.hand?.[m.name]
          ? html`<button type="button" class="ghost compact" data-action="record" disabled=${off || !isDesktop()} onClick=${() => record(m, true)}>I've changed it</button>
            <button type="button" class="ghost compact" data-action="take-off-hand" disabled=${off || !isDesktop()} onClick=${() => takeOffHand(m)}>Take it off…</button>
            <button type="button" class="ghost compact" data-action="keep" disabled=${off || !isDesktop()} title="Its files as recorded, kept as a mod of its own that goes in by itself" onClick=${() => keepHand(m)}>Keep these files in the library</button>`
          : html`<button type="button" class="primary compact" data-action="record" disabled=${off || !isDesktop()} title="Record what changed in the game's folder as this mod's" onClick=${() => record(m, false)}>I've installed it</button>`}
      </div>` : null}
      <p class="muted" data-installed=${m.installed?.to || g.hand?.[m.name] ? "yes" : "no"}>${m.installed?.to ? html`Installed in <code>${m.installed.to}</code>.` : g.hand?.[m.name] ? `Put in by hand, recorded ${g.hand[m.name].slice(0, 8).replace(/(\d{4})(\d{2})(\d{2})/, "$1-$2-$3")}.` : "Not installed here yet."}</p>
      ${m.details.url ? html`<p class="muted"><a href=${m.details.url} target="_blank" rel="noopener">${m.details.url}</a></p>` : null}
      ${m.details.notes ? html`<p>${m.details.notes}</p>` : null}
      ${m.readme ? html`<details><summary>Its readme</summary><pre class="patch-readme">${m.readme}</pre></details>` : null}
      <div class="lib-panel-actions">
        <button type="button" class="ghost compact" data-action="up" aria-label="Earlier" title="Earlier" disabled=${off || i === 0} onClick=${() => move(i, -1)}>↑</button>
        <button type="button" class="ghost compact" data-action="down" aria-label="Later" title="Later" disabled=${off || i === list.length - 1} onClick=${() => move(i, 1)}>↓</button>
        ${m.installed?.to ? html`<button type="button" class="ghost compact" data-action="take-off" disabled=${off || !isDesktop() || !g.installed_found} onClick=${() => takeOff(m)}>Take it off…</button>` : null}
        <button type="button" class="ghost compact" data-action="edit" disabled=${off} onClick=${() => ui.set({ dialog: { type: "mods-edit", k, file: m.name, details: m.details, onDone: setG } })}>Edit details…</button>
        <button type="button" class="ghost compact" data-action="remove" disabled=${off} onClick=${() => confirmDialog({ title: `Remove ${m.name}?`, text: "It's kept aside in the game's patches/mods folder (.removed). What it put where the game is installed stays: History undoes that.", button: "Remove it", run: () => api("mods_remove", { key: k, name: m.name }).then((x) => { setG(x); again(); }) })}>${Icon.trash(14)} Remove</button>
      </div>
    </section>`)}
  </div>`;
}

const pics = (p, px) => [thumb(p.picture, px)];
const noPic = (p) => ({ text: "PC", colour: systemColour("pc") });
const SORTS = [
  ["game", "Game", (a, b) => byName(a.game_title, b.game_title) || a.order - b.order],
  ["title", "Mod", (a, b) => byName(a.title, b.title)],
  ["size", "Size", (a, b) => (b.size || 0) - (a.size || 0)],
];
const GROUPS = [["", "None"], ["game", "Game"], ["how", "How it goes in"]];
const COLUMNS = [
  ["title", "Mod", (p) => p.title, "title"],
  ["game", "Game", (p) => p.game_title, "game"],
  ["how", "How", (p) => p.how_name, null],
  ["size", "Size", (p) => size(p.size), "size"],
  ["installed", "Installed", installed, null],
];
const COLS = "minmax(140px, 2.4fr) minmax(140px, 2fr) 170px 90px 110px";

/** The PC mods tab: every PC game's mods, a game's opened above them (`opened`). */
export function PcMods({ switches, opened, off }) {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("mods", { sort: "game", view: "list" });
  useEffect(() => {
    if (!s.library) return;
    setData(null);
    api("mods_list", {}).then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  const items = data?.items || [];
  const games = useMemo(() => new Map(items.map((p) => [p.game_key, p.game_title])), [data]);
  const fields = [
    { id: "game", name: "Game", of: (p) => [p.game_key], label: (v) => games.get(v) || v },
    { id: "how", name: "How it goes in", of: (p) => [p.how_name] },
    { id: "installed", name: "Installed", of: (p) => [installed(p)] },
  ];
  const add = (folder) => askGame({
    title: "Add a mod", button: folder ? "Choose its folder…" : "Choose the mod…", note: "The PC game the mod is for.",
    run: async (g) => {
      if (!g) return;
      if (!g.key.startsWith("pc/")) throw new Error(`${g.title} isn't a PC game: mods are for PC games (a ROM's hacks are ROM patches).`);
      close();
      await importMods({ key: g.key, folder });
    },
  });
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    return [
      { id: "open", label: "Its game's mods", icon: "check", one: true, run: () => one && (location.hash = argHash("patches", one.game_key)) },
      { id: "game", label: "Its game", icon: "gamepad", one: true, run: () => one && (location.hash = gameHash(one.game_key)) },
    ];
  };
  const more = () => [
    { id: "mods-add-folder-page", label: "Add a mod's folder…", icon: "inbox", disabled: off || !isDesktop(), run: () => add(true) },
  ];
  const primary = html`<button type="button" class="primary compact" id="mods-import" disabled=${off || !isDesktop()} onClick=${() => add(false)}>${Icon.inbox(15)} Add a mod…</button>`;
  const top = opened && opened.startsWith("pc/") ? html`<section class="cheats-top" id="patches-top" data-key=${opened}>
    <div class="video-player-head"><h2><a href=${gameHash(opened)}>${items.find((p) => p.game_key === opened)?.game_title || "Its mods"}</a></h2>
      <button type="button" class="ghost compact" id="patches-top-close" onClick=${() => { location.hash = routeHash("patches"); }}>${Icon.close(14)} Close</button></div>
    <${ModsEditor} k=${opened} off=${off} />
  </section>` : null;
  const empty = html`<div class="library-settings" id="mods-empty">
    <p class="page-lead">Mods for your PC games, kept with each game in the library, in an order, and installed where the game is.</p>
    <p class="muted">Add a mod… brings in a mod (its archive, its installer or its folder) for a PC game, with its readme. Each says how it goes in: into the game's folder, into a folder of the game's (such as Mods/), a file to run, or by hand. Install the ticked mods… puts them where the game is installed on this computer, never into the library's copy, and History can undo it.</p>
    <div class="home-actions">${primary}</div>
    <div class="lib-switches">${switches}</div>
  </div>`;
  return html`<${Shelf} page="mods" noun=${["mod", "mods"]} title="Patches" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} columns=${COLUMNS} cols=${COLS} ratio="1"
    text=${(p) => [p.title, p.file, p.game_title, p.author].join(" ")}
    tile=${(p) => ({ pics: pics(p, 512), none: noPic(p), badge: { text: p.how_name, colour: systemColour("pc") }, title: p.title, meta: [p.game_title, installed(p)].join(" · ") })}
    line=${(p) => ({ pics: pics(p, 256), none: noPic(p), badge: { text: `${p.order}`, colour: systemColour("pc") }, title: p.title, meta: [p.game_title, p.how_name, size(p.size), installed(p)].join(" · ") })}
    actions=${actions} onOpen=${(p) => { location.hash = argHash("patches", p.game_key); }} switches=${switches} top=${top}
    primary=${items.length ? null : primary} more=${more} moreTitle="Add a mod's folder, tile size" empty=${top ? null : empty}
    panel=${(p) => html`<div class="shelf-panel" id="mods-panel" data-key=${p.game_key} data-file=${p.file}>
      <${Pic} srcs=${pics(p, 512)} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
      <h2 id="mods-panel-title"><a href=${gameHash(p.game_key)}>${p.game_title}</a></h2>
      <${ModsEditor} k=${p.game_key} off=${off} focus=${p.file} />
    </div>`} />`;
}
