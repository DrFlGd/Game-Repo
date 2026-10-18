// Collections (docs/PLAN.md, "Phase 3 design"): a collection is rules (systems, which
// games, 1G1R and its region order, languages, flags left out, a search, DATs as
// sources) plus games added or left out by hand, kept in the library and worked out
// from the index when shown. It's saved as DATs (one per system, D23) or exported to
// ES-DE or a plain folder, as a change with a preview and Undo; each export is
// remembered, so it can be updated and checked.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, SearchNote } from "./layout.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { Icon } from "./icons.js";
import { CarryChoices, NO_CARRY, scrapeDialog } from "./metadata.js";

const PAGE = 200;

/** Card sizes for a size limit, as their labels say them (64 GB: 64,000,000,000 bytes). */
const CARDS = [4, 8, 16, 32, 64, 128, 256, 512, 1000];
const GB = 1e9;
const cardName = (n) => (n >= 1000 ? `${n / 1000} TB` : `${n} GB`);

/** The flags a collection can leave out, as the DATs' names say them. */
export const FLAGS = [
  ["beta", "Betas"], ["proto", "Prototypes"], ["demo", "Demos"], ["sample", "Samples"], ["unl", "Unlicensed"],
  ["pirate", "Pirate"], ["aftermarket", "Aftermarket"], ["bios", "BIOS"], ["program", "Programs"], ["kiosk", "Kiosk"],
  ["debug", "Debug"], ["virtual console", "Virtual Console"], ["homebrew", "Homebrew"], ["hack", "Hacks"], ["alt", "Alternates"],
];

const prefs = () => ctx.platform?.store?.prefs;
const list = (s) => s.split(",").map((x) => x.trim()).filter(Boolean);
const err = (e) => e.message || String(e);

/** Save a collection (a new one without an id); resolves with `{ collection, summary }`. */
async function save(c) {
  const r = await api("collection_save", { collection: c });
  ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
  return r;
}

// ------------------------------------------------------------ dialogs

function NameDialog({ title, value = "", button, run }) {
  const [name, setName] = useState(value);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await run(name.trim());
      close();
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${title} id="collection-name-dialog" onSubmit=${submit} busy=${busy || !name.trim()} error=${error} submitLabel=${button}>
    <label class="field-block"><span class="field-label">Name</span>
      <input type="text" id="collection-name" maxlength="120" placeholder="e.g. Best of Game Boy" value=${name} onInput=${(e) => setName(e.target.value)} /></label>
  <//>`;
}

/** Add games by hand from a system's DAT. */
function AddGamesDialog({ c, systems, onDone }) {
  const [platform, setPlatform] = useState(systems[0]?.id || "");
  const [q, setQ] = useState("");
  const [found, setFound] = useState(null);
  const [added, setAdded] = useState([]);
  useEffect(() => {
    if (!platform) return;
    let gone = false;
    const t = setTimeout(() => {
      api("collection_candidates", { id: c.id, platform, search: q, limit: 100 })
        .then((r) => { if (!gone) setFound(r); })
        .catch((e) => { if (!gone) setFound({ error: err(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [platform, q]);
  const add = (g) => setAdded((a) => (a.includes(g.key) ? a.filter((k) => k !== g.key) : [...a, g.key]));
  const submit = async () => {
    const cur = (await api("collection_get", { id: c.id })).collection;
    const removed = cur.removed.filter((k) => !added.includes(k));
    await save({ ...cur, added: [...new Set([...cur.added, ...added])], removed });
    toast(`Added ${plural(added.length, "game", "games")} to ${c.name}.`);
    close();
    onDone?.();
  };
  return html`<${Dialog} title=${`Add games to ${c.name}`} id="add-games-dialog" wide=${true} onSubmit=${submit} busy=${!added.length} submitLabel=${added.length ? `Add ${plural(added.length, "game", "games")}` : "Add"}>
    <div class="pick-row">
      <select id="add-games-platform" class="compact" aria-label="System" value=${platform} onChange=${(e) => { setPlatform(e.target.value); setFound(null); }}>
        ${systems.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <input type="search" id="add-games-search" class="compact" placeholder="Search its DAT" aria-label="Search its DAT" value=${q} onInput=${(e) => setQ(e.target.value)} />
    </div>
    ${found?.error ? html`<p class="warn-note" role="alert">${found.error}</p>` : null}
    <ul class="ls-list organise-list" id="add-games-list">${(Array.isArray(found) ? found : []).map((g) => html`<li key=${g.key} data-name=${g.name}>
      <label class="check-row"><input type="checkbox" checked=${added.includes(g.key)} onChange=${() => add(g)} /> ${g.name}</label>
      <span class="muted">${[g.regions, g.status === "have" ? "you have it" : "not in the library"].filter(Boolean).join(" · ")}</span></li>`)}</ul>
    ${Array.isArray(found) && !found.length ? html`<p class="muted">No other games match.</p>` : null}
  <//>`;
}

/** Save as DATs: one per system, into a folder. */
function SaveDatsDialog({ c }) {
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder("The folder the DATs go to");
    if (p) setTo(p);
  };
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await api("collection_dats", { id: c.id, to: to.trim() || null });
      close();
      toast(`Saved ${plural(r.files.length, "DAT", "DATs")} of ${c.name} in ${r.files[0]?.rel ? r.dir.slice(r.dir.indexOf("_library")) : r.dir}.`, { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.files[0].path) }] : [] });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`Save ${c.name} as DATs`} id="save-dats-dialog" onSubmit=${submit} busy=${busy} error=${error} submitLabel="Save">
    <p>One Logiqx DAT for each system, <code>${c.name} - <i>System</i>.dat</code>, with every game and its ROMs as its own DAT lists them. RomVault, clrmamepro and igir can build the set from them.</p>
    <div class="field-block"><span class="field-label">Into</span>
      <div class="pick-row">
        <input type="text" id="save-dats-to" placeholder=${`The library's _library/collection-dats/${c.name}`} value=${to} onInput=${(e) => setTo(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="save-dats-pick" onClick=${pick}>Choose…</button>` : null}
      </div></div>
  <//>`;
}

const WAYS = [
  ["rules", "By rules", "Systems, regions, one per title, a DAT of your own: worked out from the full sets as they change."],
  ["folders", "From folders", "A folder, a folder of folders, or several, from anywhere: the games in them, matched by hash to your full sets."],
  ["dat", "From a DAT", "A DAT listing the collection's games (a best-of, another tool's list), read in place and matched by hash."],
];

/** What a collection was made from, in a few words. */
export function madeFromText(m) {
  if (!m) return "by rules";
  if (m.kind === "dat") return `from ${baseName(m.path || "a DAT")}`;
  const n = m.paths?.length || 0;
  return n === 1 ? `from ${baseName(m.paths[0])}` : `from ${n} folders`;
}
const baseName = (p) => String(p).replace(/[\\/]+$/, "").split(/[\\/]/).pop();

/** New collection: by rules (a name), from folders or from a DAT (read, then reviewed).
 *  With `again` (a collection made from folders or a DAT) it reads them again and goes
 *  straight to the review, which replaces the collection's DATs. */
function NewCollectionDialog({ way: given = "rules", again, onMade }) {
  const [way, setWay] = useState(again ? again.made_from?.kind || "folders" : given);
  const [name, setName] = useState(again?.name || "");
  const [folders, setFolders] = useState(again?.made_from?.kind === "folders" ? again.made_from.paths || [] : []);
  const [typed, setTyped] = useState("");
  const [datPath, setDatPath] = useState(again?.made_from?.kind === "dat" ? again.made_from.path || "" : "");
  const [review, setReview] = useState(null);
  const [off, setOff] = useState(new Set());
  const [bring, setBring] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const named = (n) => setName((v) => v || n);
  const addFolder = async () => {
    const p = await ctx.platform.library.pickFolder("A folder with the collection's games");
    if (p && !folders.includes(p)) { setFolders((f) => [...f, p]); named(baseName(p)); }
  };
  const addTyped = () => {
    const p = typed.trim();
    if (p && !folders.includes(p)) { setFolders((f) => [...f, p]); named(baseName(p)); }
    setTyped("");
  };
  const pickDat = async () => {
    const p = await ctx.platform.library.pickFile("A DAT listing the collection's games (.dat, .xml or .zip)", ["dat", "xml", "zip"]);
    if (p) { setDatPath(p); named(baseName(p).replace(/\.(dat|xml|zip)$/i, "")); }
  };
  const read = async () => {
    setBusy(true);
    setError("");
    try {
      let r;
      if (way === "folders") {
        const paths = typed.trim() && !folders.includes(typed.trim()) ? [...folders, typed.trim()] : folders;
        if (typed.trim()) { setFolders(paths); named(baseName(typed.trim())); setTyped(""); }
        const { job } = await api("collection_read_folders", { paths });
        const done = await followJob(job, "Reading folders");
        if (done.error) throw new Error(done.error);
        r = done.result;
      } else {
        r = await api("collection_read_dat", { path: datPath.trim() });
        named(baseName(datPath.trim()).replace(/\.(dat|xml|zip)$/i, ""));
      }
      setOff(new Set());
      setReview(r);
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => { if (again) read(); }, []);
  const make = async () => {
    setBusy(true);
    setError("");
    try {
      if (way === "rules") {
        const r = await save({ id: "", name: name.trim(), rules: {} });
        close();
        onMade?.(r.collection, "rules");
        return;
      }
      const games = review.systems.flatMap((x) => x.games).map((g) => g.id).filter((id) => !off.has(id));
      const r = await api("collection_from", { name: name.trim(), id: again?.id || null, games, made_from: review.made_from });
      ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
      const lacking = bring ? missing() : [];
      if (lacking.length) {
        // the games the full set lacks, brought in after a preview
        const plan = await api("collection_bring_plan", { games: lacking.map((g) => ({ id: g.id, files: g.files })) });
        onMade?.(r.collection, "games");
        ui.set({ dialog: { type: "bring", plan } });
        return;
      }
      close();
      toast(again ? `Read ${again.name} again: ${plural(games.length, "game", "games")}.` : `Made ${r.collection.name}: ${plural(games.length, "game", "games")}, kept as its own DATs in the library.`, 8000);
      onMade?.(r.collection, "games");
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  // the games ticked that the full set doesn't have at all, with the files that are them
  const missing = () => (review?.systems || []).flatMap((x) => x.games).filter((g) => !off.has(g.id) && g.status === "missing" && g.files?.length);
  if (review) {
    const picked = review.games - [...off].length;
    const lacking = way === "folders" ? missing().length : 0;
    const tick = (id) => setOff((s0) => { const s1 = new Set(s0); s1.has(id) ? s1.delete(id) : s1.add(id); return s1; });
    const have = review.systems.reduce((n, x) => n + x.games.filter((g) => !off.has(g.id) && g.status === "have").length, 0);
    return html`<${Dialog} title=${again ? `Read ${again.name} again` : way === "dat" ? "The DAT's games" : "The folders' games"} id="subset-review" wide=${true} onSubmit=${make}
      busy=${busy || !picked || !name.trim()} error=${error} submitLabel=${again ? `Replace its games (${picked})` : `Make the collection (${plural(picked, "game", "games")})`}
      cancelLabel=${again ? "Cancel" : "Back"} onCancel=${again ? close : () => setReview(null)}>
      <p id="subset-totals">${way === "dat" ? `The DAT lists ${plural(review.read, "game", "games")}` : `Read ${plural(review.read, "file", "files")}`}: <b>${plural(review.games, "game", "games")}</b> found in your full sets${review.games ? html`, <b>${have}</b> of those ticked you have` : null}${review.unknown.length ? `; ${plural(review.unknown.length, way === "dat" ? "game isn't" : "file isn't", way === "dat" ? "games aren't" : "files aren't")}` : ""}.</p>
      ${!again ? html`<label class="field-block"><span class="field-label">Name</span>
        <input type="text" id="collection-name" maxlength="120" placeholder="e.g. Best of Game Boy" value=${name} onInput=${(e) => setName(e.target.value)} /></label>` : null}
      <div class="subset-systems" id="subset-systems">${review.systems.map((x) => html`<section key=${x.platform} class="subset-system" data-platform=${x.platform}>
        <h3>${x.name} <span class="muted">${plural(x.games.length, "game", "games")}, ${x.have} you have</span></h3>
        <ul class="ls-list organise-list">${x.games.map((g) => html`<li key=${g.id} data-name=${g.name} data-status=${g.status}>
          <label class="check-row"><input type="checkbox" checked=${!off.has(g.id)} onChange=${() => tick(g.id)} /> ${g.name}</label>
          <span class="muted" title=${g.from.join("\n")}>${g.status === "have" ? "" : g.status === "partial" ? "partly in your set · " : "not in your set · "}${g.from.length > 1 ? plural(g.from.length, "file", "files") : g.from[0]}</span></li>`)}</ul>
      </section>`)}</div>
      ${review.unknown.length ? html`<details class="field-block subset-unknown" id="subset-unknown"><summary>${plural(review.unknown.length, "file", "files")} not in any audit DAT</summary>
        <ul class="ls-list">${review.unknown.slice(0, 500).map((u, i) => html`<li key=${i} data-path=${u.path}><span class="op-to">${u.path}</span> <span class="muted">${u.why}</span></li>`)}</ul>
        <small class="muted">Import the DAT of a system they belong to on the Audit page, then read them again.</small></details>` : null}
      ${lacking ? html`<label class="check-row bring-row"><input type="checkbox" id="subset-bring" checked=${bring} onChange=${(e) => setBring(e.target.checked)} />
        <span>Also bring the ${plural(lacking, "game", "games")} your full set doesn't have into the library: copied into its folder with games and named as your audit DAT names ${lacking === 1 ? "it" : "them"} (with a preview first, and Undo)</span></label>` : null}
      <p class="muted">The collection is kept as DATs of its own (one per system, with your full sets' names and hashes) in the library. Its games are exported from your full sets; ${way === "dat" ? "the DAT" : "the folders"} aren't changed.</p>
    <//>`;
  }
  const can = way === "rules" ? !!name.trim() : way === "folders" ? folders.length || typed.trim() : datPath.trim();
  return html`<${Dialog} title=${again ? `Reading ${again.name} again` : "A new collection"} id="collection-new-dialog" onSubmit=${way === "rules" ? make : read}
    busy=${busy || !can || !!again} error=${error} submitLabel=${busy ? "Reading…" : way === "rules" ? "Make it" : "Read"}>
    ${!again ? html`<div class="field-block way-pick" id="collection-way" role="radiogroup" aria-label="How it's made">${WAYS.map(([k, label, about]) => html`<label key=${k} class="way" data-way=${k}>
      <input type="radio" name="collection-way" value=${k} checked=${way === k} onChange=${() => { setWay(k); setError(""); }} />
      <span><b>${label}</b><small class="muted">${about}</small></span></label>`)}</div>` : null}
    ${way === "rules" ? html`<label class="field-block"><span class="field-label">Name</span>
      <input type="text" id="collection-name" maxlength="120" placeholder="e.g. Best of Game Boy" value=${name} onInput=${(e) => setName(e.target.value)} /></label>` : null}
    ${way === "folders" && !again ? html`<div class="field-block"><span class="field-label">Folders</span>
      ${folders.length ? html`<ul class="ls-list" id="new-folders">${folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${f} off`} onClick=${() => setFolders((v) => v.filter((x) => x !== f))}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="new-folder-path" placeholder="A folder's path" value=${typed} onInput=${(e) => setTyped(e.target.value)} onKeyDown=${(e) => { if (e.key === "Enter" && typed.trim()) { e.preventDefault(); addTyped(); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="new-add-folder" onClick=${addFolder}>Add a folder…</button>` : null}
      </div>
      <small class="muted">Every file in them and their folders is read (it may take a while for big folders). They're not added as folders with games, and nothing in them changes.</small></div>` : null}
    ${way === "dat" && !again ? html`<div class="field-block"><span class="field-label">DAT</span>
      <div class="pick-row">
        <input type="text" id="new-dat-path" placeholder="A .dat, .xml or .zip file" value=${datPath} onInput=${(e) => setDatPath(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="new-pick-dat" onClick=${pickDat}>Choose…</button>` : null}
      </div>
      <small class="muted">It's read where it is and not imported: the Audit page's DATs stay your full sets.</small></div>` : null}
    ${again && !error ? html`<p class="muted">Reading ${madeFromText(again.made_from).replace(/^from /, "")}…</p>` : null}
  <//>`;
}

/** The preview of bringing a collection's missing games into the library; Run starts it. */
function BringDialog({ plan }) {
  const nothing = !plan.ops;
  return html`<${Dialog} title=${plan.title || "Bring the games into the library"} id="bring-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} submitLabel=${nothing ? "OK" : "Run"} cancelLabel=${nothing ? null : "Not now"}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to bring in: ${plan.left?.length ? "see what's left alone." : "your full set has them all."}</p>`
      : html`<p class="muted">The files are hardlinked where the folder with games is on the same drive (they take no space), else copied, and checked; the folders you read aren't changed. The games are scanned afterwards, so your full set has them. It can be undone from the message afterwards or from Settings › History.</p>`}
  <//>`;
}

/** Where an export's places were last, by layout (Phase 3's `target`, `out`, `folder` and `esde` too). */
function savedPlaces(saved, id) {
  const p = { ...(saved.places?.[id] || {}) };
  if (id === "esde") { p.roms = p.roms || saved.out || ""; p.esde = p.esde || saved.esde || ""; }
  if (id === "folder") p.out = p.out || saved.folder || "";
  return p;
}

/** Export…: a layout (ES-DE, a device's firmware, a plain folder: Settings › Export layouts),
 *  where its places are, what it carries, then the preview; Run starts the change.
 *  Opened with `plan` (Update) it goes straight to the preview. */
function ExportDialog({ c, sum, plan: given, only, count }) {
  const saved = prefs()?.get("gr-export", {}) || {};
  const [layouts, setLayouts] = useState(null);
  const [layout, setLayout] = useState(saved.layout || saved.target || "esde");
  const [places, setPlaces] = useState(() => savedPlaces(saved, saved.layout || saved.target || "esde"));
  const [defaults, setDefaults] = useState({});
  const [alwaysCopy, setAlwaysCopy] = useState(false);
  const [shortNames, setShortNames] = useState(true);
  const [carry, setCarry] = useState({ ...NO_CARRY, ...(saved.carry || {}) });
  const [plan, setPlan] = useState(given || null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (given) return;
    api("layouts").then((l) => {
      setLayouts(l);
      if (!l.some((x) => x.id === layout)) setLayout("esde");
    }).catch((e) => setError(`Couldn't read the layouts: ${err(e)}`));
    api("export_defaults").then(setDefaults).catch(() => {});
  }, []);
  const l = layouts?.find((x) => x.id === layout);
  const value = (p) => places[p.id] ?? "";
  const shown = (p) => value(p) || (layout === "esde" ? defaults[p.id] || "" : "");
  const ready = !!l && l.places.every((p) => p.optional || shown(p).trim());
  const choose = (id) => {
    const next = layouts.find((x) => x.id === id);
    setLayout(id);
    setPlaces(savedPlaces(saved, id));
    if (next) setCarry((v) => ({ ...v, image_size: next.pictures || 0 }));
  };
  const pick = (p) => async () => {
    const d = await ctx.platform.library.pickFolder(p.label);
    if (d) setPlaces((v) => ({ ...v, [p.id]: d }));
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      const at = Object.fromEntries(l.places.map((p) => [p.id, shown(p).trim()]).filter(([, v]) => v));
      prefs()?.set("gr-export", { layout, places: { ...(saved.places || {}), [layout]: at }, carry });
      const can = l.carries;
      const carried = { ...carry, fields: carry.fields.filter((f) => can.fields.includes(f)), media: carry.media.filter((m) => can.media.includes(m)),
        manuals: carry.manuals && can.manuals, docs: carry.docs && can.docs, bios: !!carry.bios && can.bios };
      setPlan(await api("export_plan", { collection: c.id, layout, places: at, always_copy: alwaysCopy, short_names: shortNames, carry: carried, only: only || [] }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.label} id="export-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} busy=${plan.short} submitLabel=${nothing ? "OK" : "Run"}
      cancelLabel=${given ? "Cancel" : "Back"} onCancel=${given ? close : () => setPlan(null)}>
      ${plan.layout_name ? html`<p class="muted" id="export-layout-used">In the layout ${plan.layout_name}.</p>` : null}
      <${Preview} plan=${plan} />
      ${nothing ? html`<p id="organise-nothing">Nothing to do: ${plan.in_place ? "the export is up to date." : "none of the collection's games are in the library."}</p>`
        : html`<p class="muted">Files are hardlinked where the folder is on the same drive as the library's games (they take no space), else copied, and every one written is read back and checked. Files the export placed before that the collection no longer has are set aside, never deleted. The change can be undone from the message afterwards or from Settings › History.</p>`}
    <//>`;
  }
  return html`<${Dialog} title=${only?.length ? `Export ${plural(count, "game", "games")} of ${c.name}` : `Export ${c.name}`} id="export-dialog" onSubmit=${preview} busy=${busy || !ready} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <label class="field-block"><span class="field-label">To</span>
      <select id="export-layout" class="compact" value=${layout} disabled=${!layouts} onChange=${(e) => choose(e.target.value)}>
        ${(layouts || [{ id: layout, name: "…" }]).map((x) => html`<option key=${x.id} value=${x.id}>${x.name}${x.builtin === false ? " (yours)" : ""}</option>`)}
      </select>
      <small class="muted" id="export-help">${l?.about || ""} <a href=${routeHash("layouts")} id="export-edit-layouts" onClick=${close}>Export layouts…</a></small></label>
    ${(l?.places || []).map((p) => html`<div class="field-block" key=${`${layout}-${p.id}`} data-place=${p.id}><span class="field-label">${p.label}${p.optional ? " (optional)" : ""}</span>
      <div class="pick-row"><input type="text" id=${`export-place-${p.id}`} placeholder=${p.example ? `e.g. ${p.example}` : ""} value=${shown(p)} onInput=${(e) => setPlaces((v) => ({ ...v, [p.id]: e.target.value }))} />
        ${isDesktop() ? html`<button type="button" class="ghost" id=${`export-place-${p.id}-pick`} onClick=${pick(p)}>Choose…</button>` : null}</div>
      ${p.note ? html`<small class="muted">${p.note}</small>` : null}</div>`)}
    ${l?.gamelist ? html`<label class="check-row"><input type="checkbox" id="export-short" checked=${shortNames} onChange=${(e) => setShortNames(e.target.checked)} /> Name games in the gamelist by their title (Alpha), not the full name (Alpha (USA))</label>` : null}
    ${l ? html`<${CarryChoices} value=${carry} onChange=${setCarry} carries=${l.carries} layout=${l} />` : null}
    <label class="check-row"><input type="checkbox" id="export-copy" checked=${alwaysCopy} onChange=${(e) => setAlwaysCopy(e.target.checked)} /> Always copy (no hardlinks)</label>
    ${only?.length ? html`<p class="muted" id="export-part">Only the ${plural(count, "game", "games")} ticked. Update… keeps to them.</p>` : null}
    ${!only?.length && c.size_limit && sum && sum.size > c.size_limit ? html`<p class="warn-note" id="export-over">${c.name} is ${size(sum.size)}, ${size(sum.size - c.size_limit)} more than its size limit (${size(c.size_limit)}).</p>` : null}
    <p class="muted">Games are exported in the library's format (Convert… changes it first). Nothing changes until you've seen the preview and pressed Run.</p>
  <//>`;
}

export function CollectionDialogs({ dialog }) {
  if (dialog.type === "collection-name") return html`<${NameDialog} ...${dialog} />`;
  if (dialog.type === "collection-new") return html`<${NewCollectionDialog} ...${dialog} />`;
  if (dialog.type === "bring") return html`<${BringDialog} ...${dialog} />`;
  if (dialog.type === "add-games") return html`<${AddGamesDialog} ...${dialog} />`;
  if (dialog.type === "save-dats") return html`<${SaveDatsDialog} ...${dialog} />`;
  if (dialog.type === "export") return html`<${ExportDialog} key=${dialog.plan?.id || "new"} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ the rules

function Ticks({ id, items, value, onChange, none }) {
  return html`<div class="tick-grid" id=${id}>${items.map(([v, label]) => html`<label class="check-row" key=${v}>
    <input type="checkbox" value=${v} checked=${value.includes(v)} onChange=${(e) => onChange(e.target.checked ? [...value, v] : value.filter((x) => x !== v))} /> ${label}</label>`)}
    ${!items.length && none ? html`<span class="muted">${none}</span>` : null}</div>`;
}

function RulesEditor({ c, onSave, off }) {
  const dats = useStore(ui, (s) => s.dats) || [];
  const r = c.rules;
  const set = (patch) => onSave((x) => ({ ...x, rules: { ...x.rules, ...patch } }));
  const systems = [...new Map(dats.filter((d) => d.current).map((d) => [d.platform, d.platform_name])).entries()].sort((a, b) => a[1].localeCompare(b[1]));
  const datNames = [...new Set(dats.map((d) => d.name))].sort().map((n) => [n, n]);
  const more = [r.languages.length ? `languages ${r.languages.join(", ")}` : "", r.exclude.length ? `${r.exclude.length} left out` : "", r.search ? `names with "${r.search}"` : "", r.from_dats.length ? plural(r.from_dats.length, "DAT", "DATs") : ""].filter(Boolean).join(" · ");
  const text = (k, v) => html`<input type="text" id=${`rule-${k}`} class="compact" disabled=${off} value=${v} onChange=${(e) => set({ [k]: k === "search" ? e.target.value : list(e.target.value) })} />`;
  return html`<fieldset class="rules" id="collection-rules" disabled=${off}>
    <div class="field-block"><span class="field-label">Systems</span>
      <${Ticks} id="rule-platforms" items=${systems} value=${r.platforms} onChange=${(v) => set({ platforms: v })} none="Import a DAT first: a collection is made from DATs." />
      <small class="muted">${r.platforms.length ? "" : "None ticked: every system with a DAT."}</small></div>
    <label class="field-block"><span class="field-label">Games</span>
      <select id="rule-include" class="compact" value=${r.include} onChange=${(e) => set({ include: e.target.value })}>
        <option value="have">The games you have</option>
        <option value="all">Every game in the DAT (to see what to look for)</option>
        <option value="none">Only the games added by hand</option>
      </select></label>
    <label class="check-row"><input type="checkbox" id="rule-1g1r" checked=${r.one_per_title} onChange=${(e) => set({ one_per_title: e.target.checked })} /> One game per title (1G1R)</label>
    <div class="rule-pair">
      <label class="field-block"><span class="field-label">Regions, first preferred</span>${text("regions", r.regions.join(", "))}</label>
      <label class="check-row"><input type="checkbox" id="rule-only-regions" checked=${r.only_regions} onChange=${(e) => set({ only_regions: e.target.checked })} /> Only these regions</label>
    </div>
    <details class="rules-more" id="rules-more">
      <summary>More rules <span class="muted">${more || "languages, leave out, names, DATs"}</span></summary>
      <div class="rules-more-body">
      <label class="field-block"><span class="field-label">Languages (optional)</span>${text("languages", r.languages.join(", "))}
        <small class="muted">e.g. En, Fr: games that list none of them are left out (those that list no language are kept).</small></label>
      <div class="field-block"><span class="field-label">Leave out</span>
        <${Ticks} id="rule-exclude" items=${FLAGS} value=${r.exclude} onChange=${(v) => set({ exclude: v })} /></div>
      <label class="field-block"><span class="field-label">Names with (optional)</span>${text("search", r.search)}</label>
      <div class="field-block"><span class="field-label">Only games in these DATs (optional)</span>
        <${Ticks} id="rule-from-dats" items=${datNames} value=${r.from_dats} onChange=${(v) => set({ from_dats: v })} />
        <small class="muted">A DAT of your own (a best-of list, a RetroAchievements list) picks the games, by name or by any ROM's hashes.</small></div>
      </div>
    </details>
    ${detailsOf(r.details) ? html`<div class="field-block" id="rule-details"><span class="field-label">Details</span>
      <span>${detailsOf(r.details)}</span>
      <small class="muted">From the Library page's filters: only games with these details, as they're scraped. <button type="button" class="linkish" id="rule-details-clear" onClick=${() => set({ details: {} })}>Clear them</button></small></div>` : null}
  </fieldset>`;
}

/** A collection's rules by details, in words ("Genre Puzzle or Action; 1990s; 2+ players"). */
function detailsOf(d) {
  if (!d) return "";
  const parts = [];
  const or = (name, v) => { if (v?.length) parts.push(`${name} ${v.join(" or ")}`); };
  if (d.search) parts.push(`with "${d.search}"`);
  or("Genre", d.genre);
  if (d.decade?.length) parts.push(d.decade.map((x) => `${x}s`).join(" or "));
  or("Developer", d.developer);
  or("Publisher", d.publisher);
  if (d.players) parts.push(`${d.players}+ players`);
  if (d.rating) parts.push(`rated ${Math.round(d.rating * 100)}% or more`);
  if (d.has?.length) parts.push(`with ${d.has.join(", ")}`);
  if (d.look) parts.push("needing a look");
  return parts.join("; ");
}

// ------------------------------------------------------------ properties

/** How many games, systems and bytes, and whether it fits its size limit (a card). */
function Properties({ c, sum, off, onSave, onPick }) {
  const limit = c.size_limit || 0;
  const preset = !limit ? "" : CARDS.find((n) => n * GB === limit) ? String(limit / GB) : "other";
  const [other, setOther] = useState(preset === "other");
  const choice = other ? "other" : preset;
  const setLimit = (bytes) => onSave((x) => ({ ...x, size_limit: bytes > 0 ? Math.round(bytes) : null }));
  const pick = (v) => {
    setOther(v === "other");
    if (v !== "other") setLimit(v ? Number(v) * GB : 0);
  };
  const over = limit && sum.size > limit;
  const missing = sum.games - sum.have;
  const share = (n) => `width:${Math.min(100, sum.size ? (100 * n) / sum.size : 0)}%`;
  return html`<section class="ls-section" id="collection-properties">
    <h2>Properties</h2>
    <dl class="props" id="collection-props">
      <div><dt>Games</dt><dd id="prop-games">${sum.games}${missing ? html` <span class="muted">(${sum.have} you have)</span>` : null}</dd></div>
      <div><dt>Systems</dt><dd id="prop-systems">${sum.systems.length}</dd></div>
      <div><dt>Size on disk</dt><dd id="prop-size" data-bytes=${sum.size}>${size(sum.size)}</dd></div>
      ${missing ? html`<div><dt>Games you don't have</dt><dd id="prop-missing" data-bytes=${sum.missing_size}>${missing}, about ${size(sum.missing_size)} more <span class="muted">(unpacked, as their DATs list them)</span></dd></div>` : null}
      ${sum.removed ? html`<div><dt>Left out by hand</dt><dd>${sum.removed}</dd></div>` : null}
    </dl>
    <div class="field-block"><span class="field-label">Size limit</span>
      <div class="pick-row">
        <select id="collection-limit" class="compact" disabled=${off} value=${choice} onChange=${(e) => pick(e.target.value)}>
          <option value="">None</option>
          ${CARDS.map((n) => html`<option key=${n} value=${String(n)}>${cardName(n)} card</option>`)}
          <option value="other">Another size…</option>
        </select>
        ${choice === "other" ? html`<label class="field-inline"><input type="number" id="collection-limit-gb" class="compact" min="0.1" step="0.1" disabled=${off}
          value=${limit ? +(limit / GB).toFixed(2) : ""} onChange=${(e) => setLimit(Number(e.target.value) * GB)} /> GB</label>` : null}
      </div>
      ${limit ? html`<div class="size-limit" id="collection-fit" data-over=${over ? "true" : "false"}>
        <div class="audit-bar" role="img" aria-label=${`${size(sum.size)} of ${size(limit)}`}><span style=${`width:${Math.min(100, (100 * sum.size) / limit)}%`}></span></div>
        <p>${over ? html`<b>Over by ${size(sum.size - limit)}</b>: ${size(sum.size)} of ${size(limit)}. Sort the games biggest first to see what to leave out, or narrow the systems.`
          : html`<b>Fits</b>: ${size(sum.size)} of ${size(limit)}, ${size(limit - sum.size)} to spare.`}</p>
        <small class="muted">${size(limit)} is as Windows counts it${preset && preset !== "other" ? ` (a ${cardName(limit / GB)} card)` : ""}; the card's own file system takes a little of it, and a handheld's firmware may share it.${missing ? " Games you don't have aren't counted." : ""}</small>
      </div>` : null}
    </div>
    ${sum.systems.length > 1 ? html`<ul class="ls-list system-sizes" id="collection-system-sizes">${[...sum.systems].sort((a, b) => b.size - a.size).map((x) => html`<li key=${x.platform} data-platform=${x.platform}>
      <button type="button" class="linkish" onClick=${() => onPick(x.platform)}>${x.name}</button>
      <span class="muted">${plural(x.games, "game", "games")}</span>
      <span class="audit-bar" role="img" aria-label=${`${size(x.size)} of ${size(sum.size)}`}><span style=${share(x.size)}></span></span>
      <span class="sys-size">${size(x.size)}</span></li>`)}</ul>` : null}
  </section>`;
}

// ------------------------------------------------------------ exports

function Exports({ c, sum, exports, off, onChange }) {
  const [checks, setChecks] = useState({});
  const mine = exports.filter((e) => e.collection === c.id);
  const update = async (e) => {
    try {
      const plan = await api("export_plan", { export: e.id });
      ui.set({ dialog: { type: "export", c, plan } });
    } catch (x) {
      toast(`Couldn't plan the update: ${err(x)}`, 10000);
    }
  };
  const check = async (e) => {
    try {
      const { job } = await api("export_check", { id: e.id });
      const done = await followJob(job, "Checking an export");
      if (done.error) throw new Error(done.error);
      const r = done.result;
      setChecks((m) => ({ ...m, [e.id]: r }));
      toast(r.missing.length || r.changed.length ? `${plural(r.ok, "file is", "files are")} as they should be; ${r.missing.length} missing, ${r.changed.length} changed.` : `All ${plural(r.ok, "file", "files")} are as they should be.`, 8000);
      onChange();
    } catch (x) {
      toast(`Couldn't check it: ${err(x)}`, 10000);
    }
  };
  const forget = (e) => confirmDialog({
    title: "Forget this export?", text: `The app stops keeping ${e.out} in step with ${c.name}. Its files stay where they are.`, button: "Forget",
    run: async () => { await api("export_forget", { id: e.id }); onChange(); },
  });
  return html`<section class="ls-section" id="collection-exports">
    <h2>Exports</h2>
    ${mine.length ? html`<ul class="ls-list change-list" id="exports">${mine.map((e) => {
      const r = checks[e.id];
      return html`<li key=${e.id} data-export=${e.id} data-target=${e.target}>
        <span class="change-label"><span><b>${e.layout_name || (e.target === "esde" ? "ES-DE" : "Folder")}</b> <span class="op-to">${e.out}</span></span>
          <span class="muted">${e.only?.length ? "Part of it · " : ""}${plural(e.count, "file", "files")} · updated ${e.updated?.slice(0, 10)}${e.checked ? ` · checked ${e.checked.at.slice(0, 10)}: ${e.checked.missing + e.checked.changed ? `${e.checked.missing} missing, ${e.checked.changed} changed` : "all as they should be"}` : ""}</span>
          ${r ? html`<span class="export-check" data-ok=${r.ok} data-missing=${r.missing.length} data-changed=${r.changed.length}>${[...r.missing.map((f) => ["Missing", f]), ...r.changed.map((f) => ["Changed", f])].slice(0, 20).map(([k, f], i) => html`<span key=${i} class="warn-note">${k}: ${f.path}${f.why ? ` (${f.why})` : ""}</span>`)}</span>` : null}</span>
        <button type="button" class="ghost" data-action="update" disabled=${off} title="Export it again: new games added, those the collection no longer has set aside" onClick=${() => update(e)}>Update…</button>
        <button type="button" class="ghost" data-action="check" onClick=${() => check(e)}>Check</button>
        ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => ctx.platform.library.openPath(e.out)}>${Icon.folder(14)}</button>` : null}
        <button type="button" class="ghost" data-action="forget" disabled=${off} onClick=${() => forget(e)}>Forget</button>
      </li>`;
    })}</ul>` : html`<p class="muted" id="exports-none">Not exported yet.</p>`}
    <div class="home-actions"><button type="button" class="ghost" id="collection-export-2" disabled=${off} onClick=${() => ui.set({ dialog: { type: "export", c, sum } })}>Export…</button></div>
  </section>`;
}

// ------------------------------------------------------------ made from

/** What a collection from folders or a DAT was read from, and Read them again… */
function MadeFrom({ c, off, onRead }) {
  const m = c.made_from;
  const paths = m.kind === "dat" ? [m.path] : m.paths || [];
  return html`<section class="ls-section" id="collection-made-from">
    <h2>Made from</h2>
    <ul class="ls-list" id="made-from-paths">${paths.map((p) => html`<li key=${p}><span class="op-to">${p}</span>
      ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => reveal(p)}>${Icon.folder(14)}</button>` : null}</li>`)}</ul>
    <p class="muted">Read ${m.read?.slice(0, 10) || ""}: its games are kept as ${plural(c.rules.from_files?.length || 0, "DAT", "DATs")} of its own in the library (<code>_library/collections/${c.id}/</code>), with your full sets' names and hashes, so renamed files and newer DATs still find them. ${m.kind === "dat" ? "The DAT" : "The folders"} can be changed or gone: the collection doesn't need them.</p>
    <div class="home-actions"><button type="button" class="ghost" id="collection-read-again" disabled=${off} onClick=${onRead}>${m.kind === "dat" ? "Read it again…" : "Read them again…"}</button></div>
  </section>`;
}

// ------------------------------------------------------------ the page

export function Collections() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes, dats: st.dats, platforms: st.platforms }));
  const [all, setAll] = useState(null);
  const [id, setId] = useState(() => prefs()?.get("gr-collection", null));
  const [data, setData] = useState(null);
  const [exports, setExports] = useState([]);
  const [view, setView] = useState("games");
  const [tab, setTab] = useState("in");
  const [platform, setPlatform] = useState("");
  const [q, setQ] = useState("");
  const [sort, setSort] = useState("name");
  const [games, setGames] = useState({ total: 0, items: [], loading: true });
  const [picked, setPicked] = useState(() => new Set());
  const [tick, setTick] = useState(0);
  const off = !!s.library?.read_only;
  const again = () => setTick((t) => t + 1);
  const open = (next, v = "games") => { setId(next); setPlatform(""); setQ(""); setTab("in"); setView(v); setPicked(new Set()); };

  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    Promise.all([api("collections"), api("exports")]).then(([cs, ex]) => {
      if (gone) return;
      setAll(cs);
      setExports(ex);
      if (!cs.some((c) => c.id === id)) setId(cs[0]?.id || null);
    }).catch((e) => { if (!gone) setAll({ error: err(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, s.changes, tick]);

  useEffect(() => {
    if (!id) { setData(null); return; }
    prefs()?.set("gr-collection", id);
    let gone = false;
    api("collection_get", { id }).then((d) => { if (!gone) setData(d); }).catch((e) => { if (!gone) setData({ error: err(e) }); });
    return () => { gone = true; };
  }, [id, s.changes, s.dats, tick]);

  const c = data?.collection?.id === id ? data.collection : null;
  const sum = c ? data.summary : null;
  useEffect(() => {
    if (!c) return;
    let gone = false;
    setGames((g) => ({ ...g, loading: true }));
    const t = setTimeout(() => {
      api("collection_games", { id: c.id, platform, removed: tab === "out", search: q, sort, offset: 0, limit: PAGE })
        .then((r) => { if (!gone) setGames({ ...r, loading: false }); })
        .catch((e) => { if (!gone) setGames({ total: 0, items: [], loading: false, error: err(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [c?.id, c?.modified, sum, tab, platform, q, sort]);

  const more = async () => {
    const r = await api("collection_games", { id: c.id, platform, removed: tab === "out", search: q, sort, offset: games.items.length, limit: PAGE });
    setGames((g) => ({ ...g, items: [...g.items, ...r.items] }));
  };
  // changes are saved one after another, each made to the collection as the last left it
  // (so quick changes, a tick then another, don't undo each other)
  const latest = useRef(null);
  const pending = useRef(0);
  const chain = useRef(Promise.resolve());
  useEffect(() => { if (!pending.current) latest.current = c; }, [c]);
  const apply = (change) => {
    const next = typeof change === "function" ? change(latest.current) : change;
    latest.current = next;
    pending.current++;
    const saved = chain.current.then(() => save(next));
    chain.current = saved.then((r) => {
      if (!--pending.current) latest.current = r.collection;
      setData(r);
    }, (e) => {
      if (!--pending.current) again();
      toast(err(e), 8000);
    });
    return saved;
  };
  const quietly = (change) => apply(change).catch(() => {});
  const create = (way = "rules") => ui.set({ dialog: { type: "collection-new", way, onMade: (made, v) => { open(made.id, v); again(); } } });
  const readAgain = () => ui.set({ dialog: { type: "collection-new", again: c, onMade: () => again() } });
  const rename = () => ui.set({ dialog: { type: "collection-name", title: `Rename ${c.name}`, value: c.name, button: "Rename", run: (name) => apply((x) => ({ ...x, name })) } });
  const duplicate = () => ui.set({ dialog: { type: "collection-name", title: `Duplicate ${c.name}`, value: `${c.name} (copy)`, button: "Duplicate", run: async (name) => {
    const r = await save({ ...c, id: "", name });
    open(r.collection.id);
  } } });
  const remove = () => confirmDialog({
    title: `Delete ${c.name}?`, text: `The collection's rules${c.made_from ? ", its own DATs" : ""} and the games added or left out by hand are deleted. Its games, DATs saved from it and its exports aren't touched.`, button: "Delete",
    run: async () => { await api("collection_delete", { id: c.id }); setId(null); again(); },
  });
  const leaveOut = (g) => quietly((x) => (g.manual ? { ...x, added: x.added.filter((k) => k !== g.key) } : { ...x, removed: [...new Set([...x.removed, g.key])] }));
  const putBack = (g) => quietly((x) => ({ ...x, removed: x.removed.filter((k) => k !== g.key) }));
  const systemsOf = () => {
    const ids = c.rules.platforms.length ? c.rules.platforms : [...new Set((s.dats || []).filter((d) => d.current).map((d) => d.platform))];
    return ids.map((p) => ({ id: p, name: (s.platforms || []).find((x) => x.id === p)?.name || p })).sort((a, b) => a.name.localeCompare(b.name));
  };

  // ticked games ("system/name") and whole systems ("system/*"), for Export these…
  const whole = (p) => picked.has(`${p}/*`);
  const isPicked = (g) => whole(g.platform) || picked.has(g.key);
  const toggle = (k) => setPicked((x) => { const y = new Set(x); y.has(k) ? y.delete(k) : y.add(k); return y; });
  const pickedCount = !sum ? 0 : sum.systems.reduce((n, x) => n + (whole(x.platform) ? x.games : 0), 0)
    + [...picked].filter((k) => !k.endsWith("/*") && !whole(k.slice(0, k.indexOf("/")))).length;
  const scope = platform ? [platform] : (sum?.systems || []).map((x) => x.platform);
  const allTicked = scope.length > 0 && scope.every(whole);
  const tickAll = () => setPicked((x) => {
    const y = new Set(x);
    for (const p of scope) {
      if (allTicked) y.delete(`${p}/*`);
      else { y.add(`${p}/*`); for (const k of [...y]) if (k.startsWith(`${p}/`) && !k.endsWith("/*")) y.delete(k); }
    }
    return y;
  });
  const exportThese = () => ui.set({ dialog: { type: "export", c, sum, only: [...picked], count: pickedCount } });

  const menu = () => c ? [
    { id: "collection-rename", label: "Rename…", icon: "edit", disabled: off, run: rename },
    { id: "collection-duplicate", label: "Duplicate…", icon: "copy", disabled: off, run: duplicate },
    { id: "collection-save-dats", label: "Save as DATs…", icon: "download", run: () => ui.set({ dialog: { type: "save-dats", c } }) },
    { id: "collection-scrape", label: "Scrape its games…", icon: "scrape", disabled: off, run: () => scrapeDialog({ collection: c.id, platformName: c.name, gameName: c.name }) },
    { id: "collection-browse", label: "Show in the Library", icon: "grid", run: () => { location.hash = `#/library?collection=${encodeURIComponent(c.id)}`; } },
    { sep: true },
    { id: "collection-delete", label: "Delete…", icon: "trash", disabled: off, run: remove },
  ] : [];
  const head = html`<${CommandBar} id="collections-head" title="Collections" count=${Array.isArray(all) && all.length ? plural(all.length, "collection", "collections") : null}
    primary=${html`<span class="cmd-buttons">
      ${c ? html`<button type="button" class="ghost compact" id="collection-dats" title="One DAT per system, for RomVault, clrmamepro or igir" onClick=${() => ui.set({ dialog: { type: "save-dats", c } })}>${Icon.download(15)} Save as DATs…</button>
        <button type="button" class="ghost compact" id="collection-export" disabled=${off} title="The whole collection, to ES-DE, a device or a plain folder, with a preview and Undo" onClick=${() => ui.set({ dialog: { type: "export", c, sum } })}>${Icon.stack(15)} Export…</button>` : null}
      <button type="button" class="primary compact" id="collection-new" disabled=${off} onClick=${() => create()}>${Icon.plus(15)} New collection…</button></span>`}
    more=${c ? { id: "collection-menu", title: "Rename, duplicate, save as DATs, scrape, delete", items: menu } : null} />`;

  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!all) return html`${head}<div class="pages"><p class="muted">Reading the collections…</p></div>`;
  if (all.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${all.error}</p></div>`;
  if (!all.length) {
    return html`${head}<div class="pages"><div class="library-settings" id="collections-empty">
      <p class="page-lead">${Icon.collection(18)} A collection is a part of your full sets: picked by rules, or the games in folders you already keep, or a DAT's. Export all or part of it for ES-DE, a handheld or a folder, or save it as DATs.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <p>Collections are made from the Audit page's DATs (your full sets) and what the scan found. ${s.dats?.length ? "" : html`Import a DAT first (<a href=${routeHash("audit")}>Audit</a>).`}</p>
        <div class="home-actions">
          <button type="button" class="primary" id="collections-first" disabled=${off} onClick=${() => create("rules")}>By rules…</button>
          <button type="button" class="ghost" id="collections-first-folders" disabled=${off} onClick=${() => create("folders")}>From folders…</button>
          <button type="button" class="ghost" id="collections-first-dat" disabled=${off} onClick=${() => create("dat")}>From a DAT…</button>
        </div>
      </section>
    </div></div>`;
  }
  const limit = c?.size_limit || 0;
  const views = [["games", "Games"], ["rules", c?.made_from ? "Made from" : "Rules"], ["size", "Size"], ["exports", "Exports", exports.filter((e) => e.collection === c?.id).length]];
  const sysTabs = sum && sum.systems.length > 1 ? [["", "All", sum.games], ...sum.systems.map((x) => [x.platform, x.name, x.games])] : [];
  const sysName = platform && sum?.systems.find((x) => x.platform === platform)?.name;
  return html`${head}<div class="pages collections-page" id="collections-page">
    <div class="collections-layout">
      <nav class="collection-side" aria-label="Collections">
        <ul id="collection-side-list">${all.map((x) => html`<li key=${x.id}>
          <button type="button" class="collection-item" data-id=${x.id} aria-current=${x.id === id ? "true" : null} onClick=${() => x.id !== id && open(x.id)}>
            <b>${x.name}</b><small class="muted">${madeFromText(x.made_from)}</small></button></li>`)}</ul>
      </nav>
      <div class="collection-main" id="collection-main">
        ${data?.error ? html`<p class="warn-note" role="alert">${data.error}</p>` : null}
        ${!c ? html`<p class="muted">Reading the collection…</p>` : html`
        <div class="collection-top">
          <h2 id="collection-title">${c.name}</h2>
          <p id="collection-totals"><b id="collection-games">${sum.games}</b> games, <b id="collection-have">${sum.have}</b> you have${sum.removed ? html`, ${sum.removed} left out by hand` : null}${` · ${plural(sum.systems.length, "system", "systems")} · ${size(sum.size)}`}${limit ? html` <span class=${sum.size > limit ? "fit over" : "fit"} id="collection-fit-line">${sum.size > limit ? `over its ${size(limit)} limit by ${size(sum.size - limit)}` : `of ${size(limit)}, fits`}</span>` : null}</p>
          ${sum.no_dat.map((x) => html`<p class="warn-note" key=${x.platform}>${x.name} has no DAT, so its games aren't known.</p>`)}
          ${sum.missing_dats.map((x) => html`<p class="warn-note" key=${x}>The DAT ${x} isn't in the library any more.</p>`)}
        </div>
        <div class="seg audit-tabs" role="tablist" id="collection-view">${views.map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-view=${k} aria-selected=${view === k ? "true" : "false"} onClick=${() => setView(k)}>${label}${n ? html` <span class="tab-count">${n}</span>` : null}</button>`)}</div>
        ${view === "games" ? html`<section class="collection-games" id="collection-result">
          ${sysTabs.length ? html`<div class="seg audit-tabs system-tabs" role="tablist" id="collection-systems">${sysTabs.map(([k, label, n]) => html`<button type="button" role="tab" key=${k || "all"} data-platform=${k} aria-selected=${platform === k ? "true" : "false"} onClick=${() => setPlatform(k)}>${label} <span class="tab-count">${n}</span></button>`)}</div>` : null}
          <div class="audit-tools">
            <div class="seg audit-tabs" role="tablist" id="collection-tabs">${[["in", "In it", sum.games], ["out", "Left out by hand", sum.removed]].map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${tab === k ? "true" : "false"} onClick=${() => setTab(k)}>${label} <span class="tab-count">${n}</span></button>`)}</div>
            <input type="search" id="collection-search" class="compact" placeholder="Search the games" aria-label="Search the games" value=${q} onInput=${(e) => setQ(e.target.value)} />
            <select id="collection-sort" class="compact" aria-label="Order" value=${sort} onChange=${(e) => setSort(e.target.value)}>
              <option value="name">By name</option><option value="size">Biggest first</option></select>
            <button type="button" class="ghost compact" id="collection-add" disabled=${off} onClick=${() => ui.set({ dialog: { type: "add-games", c, systems: systemsOf(), onDone: again } })}>Add games…</button>
          </div>
          ${tab === "in" && sum.games ? html`<div class="pick-bar" id="collection-picked" data-count=${pickedCount}>
            <label class="check-row"><input type="checkbox" id="collection-tick-all" checked=${allTicked} onChange=${tickAll} /> ${sysName ? `All of ${sysName}` : sum.systems.length > 1 ? "Every system" : "All of them"}</label>
            ${pickedCount ? html`<span><b>${plural(pickedCount, "game", "games")}</b> ticked</span>
              <button type="button" class="primary compact" id="collection-export-these" disabled=${off} onClick=${exportThese}>Export these…</button>
              <button type="button" class="linkish" id="collection-untick" onClick=${() => setPicked(new Set())}>Untick them</button>`
              : html`<span class="muted">Tick games or whole systems to export only those.</span>`}
          </div>` : null}
          <${SearchNote} q=${q} found=${games.loading ? null : plural(games.total, "match", "matches")} onClear=${() => setQ("")} />
          ${games.error ? html`<p class="warn-note" role="alert">${games.error}</p>` : null}
          ${!games.loading && !games.items.length ? html`<p class="muted" id="collection-none">${q ? "No game here matches the search." : tab === "out" ? "No games left out by hand." : "No games: loosen the rules, or add games by hand."}</p>` : null}
          <ul class="audit-list" id="collection-list" data-total=${games.total} aria-busy=${games.loading ? "true" : "false"}>
            ${games.items.map((g) => html`<li class="audit-row" key=${g.key} data-name=${g.name} data-status=${g.status}>
              ${tab === "in" ? html`<input type="checkbox" class="game-tick" aria-label=${`Tick ${g.name}`} checked=${isPicked(g)} disabled=${whole(g.platform)} title=${whole(g.platform) ? "Its whole system is ticked" : null} onChange=${() => toggle(g.key)} />` : null}
              <div class="audit-main"><span class="audit-name">${g.name}</span>
                <span class="muted audit-meta">${[sum.systems.length > 1 && !platform ? g.platform_name : null, g.regions, g.status === "have" ? null : g.status === "partial" ? "partly there" : "not in the library", g.manual ? "added by hand" : null].filter(Boolean).join(" · ")}</span></div>
              <span class="muted game-size" data-bytes=${g.size || g.dat_size}>${g.size ? size(g.size) : g.dat_size ? `about ${size(g.dat_size)}` : ""}</span>
              ${tab === "in" ? html`<button type="button" class="ghost compact" data-action="leave-out" disabled=${off} onClick=${() => leaveOut(g)}>Leave out</button>`
                : html`<button type="button" class="ghost compact" data-action="put-back" disabled=${off} onClick=${() => putBack(g)}>Put back</button>`}
            </li>`)}
          </ul>
          ${games.items.length < games.total ? html`<div class="home-actions"><button type="button" class="ghost" id="collection-more" onClick=${more}>Show ${Math.min(PAGE, games.total - games.items.length)} more (${games.total - games.items.length} left)</button></div>` : null}
        </section>` : null}
        ${view === "rules" ? html`<div class="collection-rules-view">
          ${c.made_from ? html`<${MadeFrom} c=${c} off=${off} onRead=${readAgain} />` : null}
          <section class="ls-section collection-rules-section">
            <h2>${c.made_from ? "Rules on top" : "Rules"}</h2>
            ${c.made_from ? html`<p class="muted">Every game of its own DATs is in it. Rules narrow it further: one per title, regions, flags left out.</p>` : null}
            <${RulesEditor} key=${c.id} c=${c} off=${off} onSave=${quietly} />
          </section></div>` : null}
        ${view === "size" ? html`<${Properties} c=${c} sum=${sum} off=${off} onSave=${quietly} onPick=${(p) => { setPlatform(p); setTab("in"); setView("games"); }} />` : null}
        ${view === "exports" ? html`<${Exports} c=${c} sum=${sum} exports=${exports} off=${off} onChange=${again} />` : null}
        `}
      </div>
    </div>
  </div>`;
}
