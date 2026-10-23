// Collections (docs/PLAN.md, "Phase 3 design"): a collection is rules (systems, which
// games, 1G1R and its region order, languages, flags removed, a search, DATs as
// sources) plus games added or removed by hand, kept in the library and worked out
// from the index when shown. It can hold groups (D75): each a part of it with a name and
// its own way of choosing games (rules, a DAT or folders), shown as a section of its
// games; the first group is the collection's own fields. It's saved as DATs (one per
// system of each group, D23) or exported to ES-DE or a plain folder, as a change with a
// preview and Undo; each export is remembered, so it can be updated and checked.
import { html, useState, useEffect, useRef, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, SearchNote, ViewSwitch, SortMenu, DayGroups } from "./layout.js";
import { usePageKeys, selectAllKey, openMenu } from "./actions.js";
import { Games, useGames, useSelection, blank, VIEWS, opened, gameHash } from "./browse.js";
import { useSidePanel, useRouteArg, argHash } from "./panels.js";
import { Dialog, More, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { Icon } from "./icons.js";
import { CarryChoices, NO_CARRY, scrapeDialog } from "./metadata.js";
import { reviewUnknown } from "./variants.js";

/** Card sizes for a size limit, as their labels say them (64 GB: 64,000,000,000 bytes). */
const CARDS = [4, 8, 16, 32, 64, 128, 256, 512, 1000];
const GB = 1e9;
const cardName = (n) => (n >= 1000 ? `${n / 1000} TB` : `${n} GB`);

/** The flags whose games a collection can remove, as the DATs' names say them. */
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

// ------------------------------------------------------------ its groups

/** A collection's groups in order (D75): the first is its own fields (id ""), then the others. */
function groupsOf(c) {
  const first = { id: "", name: c.group_name || c.name, rules: c.rules, added: c.added, removed: c.removed, made_from: c.made_from, waiting: c.waiting || [] };
  return [first, ...(c.groups || [])];
}

/** The collection with one of its groups (by id; "" is the first) changed by `f`. */
function withGroup(c, id, f) {
  if (id) return { ...c, groups: (c.groups || []).map((g) => (g.id === id ? f(g) : g)) };
  const g = f(groupsOf(c)[0]);
  return { ...c, rules: g.rules, added: g.added, removed: g.removed, made_from: g.made_from, waiting: g.waiting || [], group_name: g.name === c.name ? "" : g.name };
}

/** The collection without one of its groups: the first's going makes the next the first. */
function withoutGroup(c, id) {
  if (id) return { ...c, groups: (c.groups || []).filter((g) => g.id !== id) };
  const [next, ...rest] = c.groups || [];
  if (!next) return c;
  return { ...c, rules: next.rules, added: next.added || [], removed: next.removed || [], made_from: next.made_from, waiting: next.waiting || [], group_name: next.name, groups: rest };
}

// ------------------------------------------------------------ dialogs

function NameDialog({ title, value = "", button, placeholder = "e.g. Best of Game Boy", run }) {
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
      <input type="text" id="collection-name" maxlength="120" placeholder=${placeholder} value=${name} onInput=${(e) => setName(e.target.value)} /></label>
  <//>`;
}

/** Add games by hand from a system's DAT (the group's own, when it has one), to one of its groups. */
function AddGamesDialog({ c, groups = [], group: given = "", systemsOf, onDone }) {
  const [group, setGroup] = useState(given);
  const systems = systemsOf(group);
  const [platform, setPlatform] = useState(systems[0]?.id || "");
  const [q, setQ] = useState("");
  const [found, setFound] = useState(null);
  const [added, setAdded] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!platform) return;
    let gone = false;
    const t = setTimeout(() => {
      api("collection_candidates", { id: c.id, group, platform, search: q, limit: 100 })
        .then((r) => { if (!gone) setFound(r); })
        .catch((e) => { if (!gone) setFound({ error: err(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [platform, q, group]);
  const pickGroup = (id) => {
    const next = systemsOf(id);
    setGroup(id);
    setAdded([]);
    setFound(null);
    if (!next.some((x) => x.id === platform)) setPlatform(next[0]?.id || "");
  };
  const add = (g) => setAdded((a) => (a.includes(g.key) ? a.filter((k) => k !== g.key) : [...a, g.key]));
  const into = groups.length > 1 ? groups.find((g) => g.id === group)?.name || c.name : c.name;
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await api("collection_hand", { id: c.id, add: added, group });
      ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
      toast(`Added ${plural(added.length, "game", "games")} to ${into}.`);
      close();
      onDone?.();
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`Add games to ${c.name}`} id="add-games-dialog" wide=${true} onSubmit=${submit} busy=${busy || !added.length} error=${error} submitLabel=${added.length ? `Add ${plural(added.length, "game", "games")}` : "Add"}>
    <div class="pick-row">
      ${groups.length > 1 ? html`<select id="add-games-group" class="compact" aria-label="Group" value=${group} onChange=${(e) => pickGroup(e.target.value)}>
        ${groups.map((g) => html`<option key=${g.id || "first"} value=${g.id}>${g.name}</option>`)}
      </select>` : null}
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

/** Save as DATs: one per system of each group, into a folder. */
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
    <p>One Logiqx DAT for each system, <code>${c.name} - <i>System</i>.dat</code>, with every game and its files.</p>
    <${More} id="save-dats-more">${c.groups?.length ? html`<p>A group after the first's is <code>${c.name} - <i>Group</i> - <i>System</i>.dat</code>.</p>` : null}
      <p>Each game's files are as its own DAT lists them, so RomVault, clrmamepro and igir can build the set from them.</p><//>
    <div class="field-block"><span class="field-label">Into</span>
      <div class="pick-row">
        <input type="text" id="save-dats-to" placeholder=${`The library's _library/collection-dats/${c.name}`} value=${to} onInput=${(e) => setTo(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="save-dats-pick" onClick=${pick}>Choose…</button>` : null}
      </div></div>
  <//>`;
}

const WAYS = [
  ["rules", "By rules", "Systems, regions, one per title, a DAT of your own: worked out from your audited games as they change."],
  ["folders", "From folders", "A folder, a folder of folders, or several, from anywhere: the games in them, matched by hash to your audited games."],
  ["dat", "From a DAT", "A DAT listing the games (a best-of, another tool's list), read in place and matched by hash."],
  ["empty", "Empty", "Just a name: games are added later with Add to a collection… in the Library or on a game's page."],
];

/** A collection's source, in a few words. */
export function sourceText(m, rules) {
  if (!m) return rules?.include === "none" ? "games added by hand" : "by rules";
  if (m.kind === "dat") return `from ${baseName(m.path || "a DAT")}`;
  const n = m.paths?.length || 0;
  const from = n === 1 ? `from ${baseName(m.paths[0])}` : `from ${n} folders`;
  return m.kind === "import" ? `imported ${from}` : from;
}
/** Read again, folders imported into it (D140) are read as folders. */
const readKind = (m) => (m?.kind === "dat" ? "dat" : "folders");
const baseName = (p) => String(p).replace(/[\\/]+$/, "").split(/[\\/]/).pop();

/** New collection: by rules or empty (a name), from folders or from a DAT (read, then reviewed).
 *  With `group` (a collection) it makes a new group of it the same ways (D75). With
 *  `again` ({ id, name, made_from, group }: a collection or group made from folders or a
 *  DAT) it reads them again and goes straight to the review, which replaces its DATs. */
function NewCollectionDialog({ way: given = "rules", again, group: into, onMade }) {
  const [way, setWay] = useState(again ? readKind(again.made_from) : given);
  const [name, setName] = useState(again?.name || "");
  const [folders, setFolders] = useState(again && readKind(again.made_from) === "folders" ? again.made_from.paths || [] : []);
  const [typed, setTyped] = useState("");
  const [datPath, setDatPath] = useState(again?.made_from?.kind === "dat" ? again.made_from.path || "" : "");
  const [review, setReview] = useState(null);
  const [off, setOff] = useState(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const what = into ? "group" : "collection";
  const example = into ? "e.g. Translations" : "e.g. Best of Game Boy";
  const named = (n) => setName((v) => v || n);
  const addFolder = async () => {
    const p = await ctx.platform.library.pickFolder(`A folder with the ${what}'s games`);
    if (p && !folders.includes(p)) { setFolders((f) => [...f, p]); named(baseName(p)); }
  };
  const addTyped = () => {
    const p = typed.trim();
    if (p && !folders.includes(p)) { setFolders((f) => [...f, p]); named(baseName(p)); }
    setTyped("");
  };
  const pickDat = async () => {
    const p = await ctx.platform.library.pickFile(`A DAT listing the ${what}'s games (.dat, .xml or .zip)`, ["dat", "xml", "zip"]);
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
      if (way === "rules" || way === "empty") {
        // empty: only the games added by hand (D141)
        const only = way === "empty" ? { include: "none" } : {};
        let r;
        if (into) {
          // a new group by rules, of the systems the collection has, after its others
          const cur = (await api("collection_get", { id: into.id })).collection;
          r = await save({ ...cur, groups: [...(cur.groups || []), { id: "", name: name.trim(), rules: { platforms: cur.rules.platforms, ...only } }] });
          toast(`Added the group ${name.trim()} to ${into.name}.`);
        } else {
          r = await save({ id: "", name: name.trim(), rules: only });
        }
        close();
        onMade?.(r.collection, way === "empty" ? "games" : "rules");
        return;
      }
      const games = review.systems.flatMap((x) => x.games).map((g) => g.id).filter((id) => !off.has(id));
      const r = await api("collection_from", {
        name: into ? "" : name.trim(), id: again?.id || into?.id || null, group: again?.group || "", ...(into ? { group_name: name.trim() } : {}),
        games, made_from: again?.made_from?.kind === "import" ? { ...review.made_from, kind: "import" } : review.made_from,
      });
      ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
      close();
      toast(again ? `Read ${again.name} again: ${plural(games.length, "game", "games")}.`
        : into ? `Added the group ${name.trim()} to ${into.name}: ${plural(games.length, "game", "games")}, kept as its own DATs in the library.`
          : `Made ${r.collection.name}: ${plural(games.length, "game", "games")}, kept as its own DATs in the library.`, 8000);
      onMade?.(r.collection, "games");
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  // the games ticked that your audited games don't have at all, with the files that are them
  const missing = () => (review?.systems || []).flatMap((x) => x.games).filter((g) => !off.has(g.id) && g.status === "missing" && g.files?.length);
  if (review) {
    const picked = review.games - [...off].length;
    const lacking = way === "folders" ? missing().length : 0;
    const tick = (id) => setOff((s0) => { const s1 = new Set(s0); s1.has(id) ? s1.delete(id) : s1.add(id); return s1; });
    const have = review.systems.reduce((n, x) => n + x.games.filter((g) => !off.has(g.id) && g.status === "have").length, 0);
    const label = again ? `Replace its games (${picked})` : into ? `Add the group (${plural(picked, "game", "games")})` : `Make the collection (${plural(picked, "game", "games")})`;
    // a stray click or Esc doesn't lose what was read and ticked (review M35)
    return html`<${Dialog} title=${again ? `Read ${again.name} again` : way === "dat" ? "The DAT's games" : "The folders' games"} id="subset-review" wide=${true} sticky=${true} onSubmit=${make}
      busy=${busy || !picked || !name.trim()} error=${error} submitLabel=${label}
      cancelLabel=${again ? "Cancel" : "Back"} onCancel=${again ? close : () => setReview(null)}>
      <p id="subset-totals">${way === "dat" ? `The DAT lists ${plural(review.read, "game", "games")}` : `Read ${plural(review.read, "file", "files")}`}: <b>${plural(review.games, "game", "games")}</b> found in your audited games${review.games ? html`, <b>${have}</b> of those ticked you have` : null}${review.unknown.length ? `; ${plural(review.unknown.length, way === "dat" ? "game isn't" : "file isn't", way === "dat" ? "games aren't" : "files aren't")}` : ""}.</p>
      ${!again ? html`<label class="field-block"><span class="field-label">Name</span>
        <input type="text" id="collection-name" maxlength="120" placeholder=${example} value=${name} onInput=${(e) => setName(e.target.value)} /></label>` : null}
      <div class="subset-systems" id="subset-systems">${review.systems.map((x) => html`<section key=${x.platform} class="subset-system" data-platform=${x.platform}>
        <h3>${x.name} <span class="muted">${plural(x.games.length, "game", "games")}, ${x.have} you have</span></h3>
        <ul class="ls-list organise-list">${x.games.map((g) => html`<li key=${g.id} data-name=${g.name} data-status=${g.status}>
          <label class="check-row"><input type="checkbox" checked=${!off.has(g.id)} onChange=${() => tick(g.id)} /> ${g.name}</label>
          <span class="muted" title=${g.from.join("\n")}>${g.status === "have" ? "" : g.status === "partial" ? "partly in your audited games · " : "not in your audited games · "}${g.from.length > 1 ? plural(g.from.length, "file", "files") : g.from[0]}</span></li>`)}</ul>
      </section>`)}</div>
      ${review.unknown.length ? html`<details class="field-block subset-unknown" id="subset-unknown"><summary>${plural(review.unknown.length, "file no DAT knows", "files no DAT knows")}</summary>
        <ul class="ls-list">${review.unknown.slice(0, 500).map((u, i) => html`<li key=${i} data-path=${u.path}><span class="op-to">${u.path}</span> <span class="muted">${u.why}</span></li>`)}</ul>
        <small class="muted">Import the DAT of a system they belong to (<a href=${argHash("import", "settings-dats")} onClick=${close}>Import › DAT files</a>), then read them again. Or import the folders into a collection (<a href=${argHash("import", "import-copy")} onClick=${close}>Import › Games into the library</a>): files no DAT knows wait on it until you add them to their system's custom DAT on Audit › Unknown.</small></details>` : null}
      ${lacking ? html`<p class="muted bring-row" id="subset-lacking">${plural(lacking, "game", "games")} in the folders ${lacking === 1 ? "isn't" : "aren't"} in your audited games yet: <a href=${routeHash("import")} onClick=${close}>Import › Games into the library</a> brings ${lacking === 1 ? "it" : "them"} in.</p>` : null}
      <p class="muted">Its games are exported from your audited games; ${way === "dat" ? "the DAT isn't" : "the folders aren't"} changed.</p>
      <${More} id="subset-more"><p>The ${what} is kept in the library as DATs of its own: one per system, with the Audit page's DATs' names and hashes.</p><//>
    <//>`;
  }
  const byName = way === "rules" || way === "empty";
  const can = byName ? !!name.trim() : way === "folders" ? folders.length || typed.trim() : datPath.trim();
  const submitLabel = busy ? "Reading…" : byName ? (into ? "Add the group" : "Make the collection") : way === "dat" ? "Read the DAT" : "Read the folders";
  return html`<${Dialog} title=${again ? `Reading ${again.name} again` : into ? `A new group of ${into.name}` : "A new collection"} id="collection-new-dialog" sticky=${busy || folders.length > 0} onSubmit=${byName ? make : read}
    busy=${busy || !can || !!again} error=${error} submitLabel=${submitLabel}>
    ${into ? html`<p class="muted" id="collection-new-group">A group is a part of ${into.name} with its own way of choosing games.</p>
      <${More} id="collection-group-more"><p>A game an earlier group has isn't in it again. Exported, each group is a collection of its own too.</p><//>` : null}
    ${!again ? html`<div class="field-block way-pick" id="collection-way" role="radiogroup" aria-label="How it's made">${WAYS.map(([k, label, about]) => html`<label key=${k} class="way" data-way=${k}>
      <input type="radio" name="collection-way" value=${k} checked=${way === k} onChange=${() => { setWay(k); setError(""); }} />
      <span><b>${label}</b><small class="muted">${about}</small></span></label>`)}</div>` : null}
    ${byName ? html`<label class="field-block"><span class="field-label">Name</span>
      <input type="text" id="collection-name" maxlength="120" placeholder=${example} value=${name} onInput=${(e) => setName(e.target.value)} /></label>` : null}
    ${way === "folders" && !again ? html`<div class="field-block"><span class="field-label">Folders</span>
      ${folders.length ? html`<ul class="ls-list" id="new-folders">${folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${f} off`} onClick=${() => setFolders((v) => v.filter((x) => x !== f))}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="new-folder-path" placeholder="A folder's path" value=${typed} onInput=${(e) => setTyped(e.target.value)} onKeyDown=${(e) => { if (e.key === "Enter" && typed.trim()) { e.preventDefault(); addTyped(); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="new-add-folder" onClick=${addFolder}>Choose a folder…</button>` : null}
      </div>
      <small class="muted">Every file in them and their folders is read, and nothing in them changes.</small></div>` : null}
    ${way === "dat" && !again ? html`<div class="field-block"><span class="field-label">DAT</span>
      <div class="pick-row">
        <input type="text" id="new-dat-path" placeholder="A .dat, .xml or .zip file" value=${datPath} onInput=${(e) => setDatPath(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="new-pick-dat" onClick=${pickDat}>Choose…</button>` : null}
      </div>
      <small class="muted">It's read where it is and not imported: the Audit page's DATs stay as they are.</small></div>` : null}
    ${again && !error ? html`<p class="muted">Reading ${sourceText(again.made_from).replace(/^(imported )?from /, "")}…</p>` : null}
  <//>`;
}

/** Where an export's places were last, by layout (Phase 3's `target`, `out`, `folder` and `esde` too). */
function savedPlaces(saved, id) {
  const p = { ...(saved.places?.[id] || {}) };
  if (id === "esde") { p.roms = p.roms || saved.out || ""; p.esde = p.esde || saved.esde || ""; }
  if (id === "folder") p.out = p.out || saved.folder || "";
  return p;
}

/** How an export can place a collection's groups after the first (D75), as the layout offers. */
const PLACEMENTS = {
  collections: ["Collections only", "Every game in its system's folder; each group is a collection there too."],
  subfolders: ["Subfolders", "The first group in the system's folder, each other in a folder of its own inside it (roms/snes/Translations)."],
  systems: ["Separate systems", "Each other group a system of its own beside its system's (roms/snes-translations)."],
};
/** "Collections only" for a layout with no collection files (ROCKNIX's): its groups aren't kept apart (D81). */
const NO_COLLECTIONS = ["Together", "Every game in its system's folder, the groups not kept apart (the layout has no collections)."];
const placement = (k, l) => (k === "collections" && l && !l.collection ? NO_COLLECTIONS : PLACEMENTS[k]);

/** Export…: a layout (ES-DE, a device's firmware, a plain folder: Settings › Export layouts),
 *  where its places are, what it carries, where its groups go, then the preview; Export
 *  starts the change. Opened with `plan` (Update) it goes straight to the preview. */
function ExportDialog({ c, sum, plan: given, only, count }) {
  const saved = prefs()?.get("gr-export", {}) || {};
  const [layouts, setLayouts] = useState(null);
  const [layout, setLayout] = useState(saved.layout || saved.target || "esde");
  const [places, setPlaces] = useState(() => savedPlaces(saved, saved.layout || saved.target || "esde"));
  const [defaults, setDefaults] = useState({});
  const [alwaysCopy, setAlwaysCopy] = useState(false);
  // RetroArch finds its thumbnails (and downloads them) by the full name
  const [shortNames, setShortNames] = useState(() => (saved.layout || saved.target) !== "retroarch");
  const [carry, setCarry] = useState({ ...NO_CARRY, ...(saved.carry || {}) });
  const [placing, setPlacing] = useState(saved.groups || "collections");
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
  // the groups' placing, when there are groups and the layout offers more than one
  const offered = (sum?.groups?.length || 0) > 1 ? l?.placements || ["collections"] : ["collections"];
  const groupsTo = offered.includes(placing) ? placing : "collections";
  const choose = (id) => {
    const next = layouts.find((x) => x.id === id);
    setLayout(id);
    setPlaces(savedPlaces(saved, id));
    setShortNames(id !== "retroarch");
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
      prefs()?.set("gr-export", { layout, places: { ...(saved.places || {}), [layout]: at }, carry, groups: offered.length > 1 ? groupsTo : saved.groups || "collections" });
      const can = l.carries;
      const carried = { ...carry, fields: carry.fields.filter((f) => can.fields.includes(f)), media: carry.media.filter((m) => can.media.includes(m)),
        manuals: carry.manuals && can.manuals, docs: carry.docs && can.docs, bios: !!carry.bios && can.bios };
      setPlan(await api("export_plan", { collection: c.id, layout, places: at, always_copy: alwaysCopy, short_names: shortNames, carry: carried, only: only || [], groups: groupsTo }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.label} id="export-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} busy=${plan.short} submitLabel=${nothing ? "OK" : "Export"}
      cancelLabel=${given ? "Cancel" : "Back"} onCancel=${given ? close : () => setPlan(null)}>
      ${plan.layout_name ? html`<p class="muted" id="export-layout-used">In the layout ${plan.layout_name}${plan.groups && plan.groups !== "collections" ? `, its groups as ${PLACEMENTS[plan.groups]?.[0].toLowerCase() || plan.groups}` : ""}.</p>` : null}
      ${plan.notes?.length ? html`<div class="warn-note" id="export-notes">${plan.notes.map((n, i) => html`<p key=${i}>${n}</p>`)}</div>` : null}
      <${Preview} plan=${plan} />
      ${nothing ? html`<p id="organise-nothing">Nothing to do: ${plan.in_place ? "the export is up to date." : "none of the collection's games are in the library."}</p>`
        : html`<p class="muted">Every file written is checked, and the change can be undone.</p>
          <${More} id="export-more"><p>Files are hardlinked where the folder is on the same drive as the library's games (they take no space), else copied. Files the export placed before that the collection no longer has are set aside, never deleted. Undo is in the message afterwards and in History.</p><//>`}
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
        ${isDesktop() && !p.device ? html`<button type="button" class="ghost" id=${`export-place-${p.id}-pick`} onClick=${pick(p)}>Choose…</button>` : null}</div>
      ${p.note ? html`<small class="muted">${p.note}</small>` : null}</div>`)}
    ${offered.length > 1 ? html`<fieldset class="field-block export-groups" id="export-groups"><legend class="field-label">Its groups after the first</legend>
      ${offered.map((k) => html`<label class="check-row" key=${k} data-placing=${k}><input type="radio" name="export-groups" value=${k} checked=${groupsTo === k} onChange=${() => setPlacing(k)} />
        <span><b>${placement(k, l)?.[0] || k}</b> <small class="muted">${placement(k, l)?.[1] || ""}</small></span></label>`)}
      <small class="muted">Update… places them the same way.</small></fieldset>` : null}
    ${l?.gamelist ? html`<label class="check-row"><input type="checkbox" id="export-short" checked=${shortNames} onChange=${(e) => setShortNames(e.target.checked)} /> Name games in the gamelist by their title (Alpha), not the full name (Alpha (USA))${layout === "retroarch" ? html` <small class="muted">(RetroArch's thumbnails are named by the full name)</small>` : null}</label>` : null}
    ${l ? html`<${CarryChoices} value=${carry} onChange=${setCarry} carries=${l.carries} layout=${l} />` : null}
    <label class="check-row"><input type="checkbox" id="export-copy" checked=${alwaysCopy} onChange=${(e) => setAlwaysCopy(e.target.checked)} /> Always copy (no hardlinks)</label>
    ${only?.length ? html`<p class="muted" id="export-part">Only the ${plural(count, "game", "games")} picked. Update… keeps to them.</p>` : null}
    ${!only?.length && c.size_limit && sum && sum.size > c.size_limit ? html`<p class="warn-note" id="export-over">${c.name} is ${size(sum.size)}, ${size(sum.size - c.size_limit)} more than its size limit (${size(c.size_limit)}).</p>` : null}
    <p class="muted">Nothing changes until you've seen the preview and pressed Export.</p>
    <${More} id="export-more"><p>Games are exported in the library's format: Convert… changes it first.</p><//>
  <//>`;
}

export function CollectionDialogs({ dialog }) {
  if (dialog.type === "collection-name") return html`<${NameDialog} ...${dialog} />`;
  if (dialog.type === "collection-new") return html`<${NewCollectionDialog} ...${dialog} />`;
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

/** A group's rules. `prefix` names its fields' ids (rule-platforms, rule-1-platforms…), so
 *  each group's are its own; `onSet` takes the rules changed. */
function RulesEditor({ rules: r, onSet, off, prefix = "rule", id = "collection-rules" }) {
  const dats = useStore(ui, (s) => s.dats) || [];
  const set = (patch) => onSet(patch);
  // (BIOS files aren't a system of games: low finding)
  const systems = [...new Map(dats.filter((d) => d.current && d.platform !== "bios").map((d) => [d.platform, d.platform_name])).entries()].sort((a, b) => a[1].localeCompare(b[1]));
  const datNames = [...new Set(dats.map((d) => d.name))].sort().map((n) => [n, n]);
  const own = r.dats || [];
  const more = [own.length ? `games of ${own.join(", ")}` : "", r.languages.length ? `languages ${r.languages.join(", ")}` : "", r.exclude.length ? `${r.exclude.length} removed` : "", r.search ? `names with "${r.search}"` : "", r.tags?.length ? `tags ${r.tags.join(", ")}` : "", r.from_dats.length ? plural(r.from_dats.length, "DAT", "DATs") : ""].filter(Boolean).join(" · ");
  const text = (k, v) => html`<input type="text" id=${`${prefix}-${k}`} class="compact" disabled=${off} value=${v} onChange=${(e) => set({ [k]: k === "search" ? e.target.value : list(e.target.value) })} />`;
  return html`<fieldset class="rules" id=${id} disabled=${off}>
    <div class="field-block"><span class="field-label">Systems</span>
      <${Ticks} id=${`${prefix}-platforms`} items=${systems} value=${r.platforms} onChange=${(v) => set({ platforms: v })} none="Import a DAT first: a collection is made from DATs." />
      <small class="muted">${r.platforms.length ? "" : "None ticked: every system with a DAT."}</small></div>
    <label class="field-block"><span class="field-label">Games</span>
      <select id=${`${prefix}-include`} class="compact" value=${r.include} onChange=${(e) => set({ include: e.target.value })}>
        <option value="have">The games you have</option>
        <option value="all">Every game in the DAT (to see what to look for)</option>
        <option value="none">Only the games added by hand</option>
      </select></label>
    <label class="field-block"><span class="field-label">Variants</span>
      <select id=${`${prefix}-variants`} class="compact" value=${r.variants || ""} title="Translations, hacks and the like, kept in each system's custom DAT" onChange=${(e) => set({ variants: e.target.value })}>
        <option value="">With the other games</option>
        <option value="without">Remove them</option>
        <option value="only">Only variants</option>
      </select></label>
    <label class="check-row"><input type="checkbox" id=${`${prefix}-1g1r`} checked=${r.one_per_title} onChange=${(e) => set({ one_per_title: e.target.checked })} /> One game per title (1G1R)</label>
    <label class="check-row"><input type="checkbox" id=${`${prefix}-addons`} checked=${r.with_addons !== false} onChange=${(e) => set({ with_addons: e.target.checked })} /> With their updates and DLC <span class="muted">(the ones you have, exported beside their game)</span></label>
    <div class="rule-pair">
      <label class="field-block"><span class="field-label">Regions, first preferred</span>${text("regions", r.regions.join(", "))}</label>
      <label class="check-row"><input type="checkbox" id=${`${prefix}-only-regions`} checked=${r.only_regions} onChange=${(e) => set({ only_regions: e.target.checked })} /> Only these regions</label>
    </div>
    <details class="rules-more" id=${prefix === "rule" ? "rules-more" : `${prefix}-more`}>
      <summary>More rules <span class="muted">${more || "another DAT, languages, what to remove, names, tags, DATs"}</span></summary>
      <div class="rules-more-body">
      <div class="field-block"><span class="field-label">Its games from these DATs, not from your audited games (optional)</span>
        <${Ticks} id=${`${prefix}-dats`} items=${datNames} value=${own} onChange=${(v) => set({ dats: v })} />
        <small class="muted">A system's translations, hacks or homebrew in a DAT of their own: the group's games are that DAT's, picked by the rules here.</small></div>
      <label class="field-block"><span class="field-label">Languages (optional)</span>${text("languages", r.languages.join(", "))}
        <small class="muted">e.g. En, Fr: games that list none of them are removed (those that list no language are kept).</small></label>
      <div class="field-block"><span class="field-label">Remove from it</span>
        <${Ticks} id=${`${prefix}-exclude`} items=${FLAGS} value=${r.exclude} onChange=${(v) => set({ exclude: v })} /></div>
      <label class="field-block"><span class="field-label">Names with (optional)</span>${text("search", r.search)}</label>
      <label class="field-block"><span class="field-label">Only variants with these tags (optional)</span>${text("tags", (r.tags || []).join(", "))}
        <small class="muted">e.g. English Translated, Hack: the variants with none of them are removed; the other games stay.</small></label>
      <div class="field-block"><span class="field-label">Only games in these DATs (optional)</span>
        <${Ticks} id=${`${prefix}-from-dats`} items=${datNames} value=${r.from_dats} onChange=${(v) => set({ from_dats: v })} />
        <small class="muted">A DAT of your own (a best-of list, a RetroAchievements list) picks the games, by name or by any file's hashes.</small></div>
      </div>
    </details>
    ${detailsOf(r.details) ? html`<div class="field-block" id=${`${prefix}-details`}><span class="field-label">Details</span>
      <span>${detailsOf(r.details)}</span>
      <small class="muted">From the Library page's filters: only games with these details, as they're scraped. <button type="button" class="linkish" id=${`${prefix}-details-clear`} onClick=${() => set({ details: {} })}>Clear them</button></small></div>` : null}
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

/** How many games, groups, systems and bytes, and whether it fits its size limit (a card). */
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
  const groups = sum.groups || [];
  return html`<section class="ls-section" id="collection-properties">
    <h2>Properties</h2>
    <dl class="props" id="collection-props">
      <div><dt>Games</dt><dd id="prop-games">${sum.games}${missing ? html` <span class="muted">(${sum.have} you have)</span>` : null}</dd></div>
      ${groups.length > 1 ? html`<div><dt>Groups</dt><dd id="prop-groups">${groups.map((g) => `${g.name} (${g.games})`).join(", ")}</dd></div>` : null}
      <div><dt>Systems</dt><dd id="prop-systems">${sum.systems.length}</dd></div>
      <div><dt>Size on disk</dt><dd id="prop-size" data-bytes=${sum.size}>${size(sum.size)}</dd></div>
      ${missing ? html`<div><dt>Games you don't have</dt><dd id="prop-missing" data-bytes=${sum.missing_size}>${missing}, about ${size(sum.missing_size)} more <span class="muted">(unpacked, as their DATs list them)</span></dd></div>` : null}
      ${sum.removed ? html`<div><dt>Removed by hand</dt><dd>${sum.removed}</dd></div>` : null}
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
        <p>${over ? html`<b>Over by ${size(sum.size - limit)}</b>: ${size(sum.size)} of ${size(limit)}. Sort the games biggest first to see what to remove, or narrow the systems.`
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

const HOW = { made: "Made", hand: "By hand", rules: "Rules changed", read: "Read again", undo: "Undone" };

/** A collection's history (D67): each change to what's in it, newest first, with Undo. */
function History({ c, off, onChange }) {
  const [steps, setSteps] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  useEffect(() => { api("collection_history", { id: c.id }).then((r) => setSteps(r.steps)).catch((e) => setError(err(e))); }, [c.id, c.modified]);
  const undo = async (s) => {
    setBusy(s.id);
    setError("");
    try {
      setSteps((await api("collection_undo", { id: c.id, step: s.id })).steps);
      onChange();
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy("");
    }
  };
  // a few games are named in the line; more are behind it, opened to see them all (D78)
  const names = (cls, said, list, n) => {
    const shown = (list || []).map((k) => k.split("/").slice(1).join("/"));
    const rest = n > shown.length ? `and ${(n - shown.length).toLocaleString()} more` : "";
    if (n <= 5) return html`<span class=${`muted ${cls}`}>${said} ${plural(n, "game", "games")}: ${[...shown, rest].filter(Boolean).join(", ")}</span>`;
    return html`<details class=${`muted history-names ${cls}`}><summary>${said} ${plural(n, "game", "games")}</summary>
      <div class="history-names-list">${shown.map((x) => html`<div key=${x}>${x}</div>`)}${rest ? html`<div>${rest}</div>` : null}</div></details>`;
  };
  const sum = (ss) => plural(ss.length, "change", "changes");
  return html`<section class="ls-section" id="collection-history"><h2>History</h2>
    <p class="muted">Each change to what's in it, a day at a time. Undo puts a change back as a change of its own; a change of its rules or groups, or reading it again, can be undone newest first.</p>
    ${error ? html`<p class="warn-note" role="alert">${error}</p>` : null}
    ${!steps ? html`<p class="muted">Reading…</p>` : !steps.length ? html`<p class="muted" id="collection-history-none">No changes kept yet.</p>` : html`<${DayGroups} id="collection-history-list" items=${steps} at=${(s) => s.at} sum=${sum}
      row=${(s) => html`<li key=${s.id} data-how=${s.how} data-undone=${s.undone ? "yes" : "no"}>
        <span class="op-paths"><span><b>${HOW[s.how] || s.how}</b> <span class="muted">${new Date(s.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}${s.undone ? " · undone" : ""}</span></span>
          ${s.added_count ? names("history-added", "Added", s.added, s.added_count) : null}
          ${s.removed_count ? names("history-removed", "Removed", s.removed, s.removed_count) : null}
          ${!s.added_count && !s.removed_count ? html`<span class="muted">The same games.</span>` : null}</span>
        ${s.can_undo ? html`<button type="button" class="ghost compact" data-action="undo" disabled=${off || !!busy} onClick=${() => undo(s)}>${busy === s.id ? "Undoing…" : "Undo"}</button>` : null}
      </li>`} />`}
  </section>`;
}

const PLACED = { subfolders: "its groups in subfolders · ", systems: "its groups as systems · " };

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
      toast(`Couldn't check the export: ${err(x)}`, 10000);
    }
  };
  const forget = (e) => confirmDialog({
    title: "Forget this export?", text: `Game Repo stops keeping ${e.out} in step with ${c.name}. Its files stay where they are.`, button: "Forget",
    run: async () => { await api("export_forget", { id: e.id }); onChange(); },
  });
  return html`<section class="ls-section" id="collection-exports">
    <h2>Exported to</h2>
    ${mine.length ? html`<ul class="ls-list change-list" id="exports">${mine.map((e) => {
      const r = checks[e.id];
      return html`<li key=${e.id} data-export=${e.id} data-target=${e.target} data-groups=${e.groups || "collections"}>
        <span class="change-label"><span><b>${e.layout_name || (e.target === "esde" ? "ES-DE" : "Folder")}</b> <span class="op-to">${e.out}</span></span>
          <span class="muted">${e.only?.length ? "Part of it · " : ""}${PLACED[e.groups] || ""}${plural(e.count, "file", "files")} · updated ${e.updated?.slice(0, 10)}${e.checked ? ` · checked ${e.checked.at.slice(0, 10)}: ${e.checked.missing + e.checked.changed ? `${e.checked.missing} missing, ${e.checked.changed} changed` : "all as they should be"}` : ""}</span>
          ${r ? html`<span class="export-check" data-ok=${r.ok} data-missing=${r.missing.length} data-changed=${r.changed.length}>${[...r.missing.map((f) => ["Missing", f]), ...r.changed.map((f) => ["Changed", f])].slice(0, 20).map(([k, f], i) => html`<span key=${i} class="warn-note">${k}: ${f.path}${f.why ? ` (${f.why})` : ""}</span>`)}</span>` : null}</span>
        <button type="button" class="ghost" data-action="update" disabled=${off} title="Export it again: new games added, those the collection no longer has set aside" onClick=${() => update(e)}>Update…</button>
        <button type="button" class="ghost" data-action="check" onClick=${() => check(e)}>Check</button>
        ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => ctx.platform.library.openPath(e.out)}>${Icon.folder(14)}</button>` : null}
        <button type="button" class="ghost" data-action="forget" disabled=${off} onClick=${() => forget(e)}>Forget…</button>
      </li>`;
    })}</ul>` : html`<p class="muted" id="exports-none">Not exported yet.</p>`}
    <p class="muted">Export… (at the top) makes a new one.</p>
  </section>`;
}

// ------------------------------------------------------------ files waiting on it (D140)

/** The files imported into it that no DAT knows yet: each joins it once a DAT does (added to
 *  its system's custom DAT on Audit › Unknown); Leave it stops waiting for one. */
function Waiting({ c, off, onChange }) {
  const [error, setError] = useState("");
  const all = groupsOf(c).flatMap((g) => (g.waiting || []).map((w) => ({ ...w, group: g.id, groupName: g.name })));
  if (!all.length) return null;
  const many = groupsOf(c).length > 1;
  const leave = async (w) => {
    setError("");
    try {
      await api("collection_waiting_leave", { id: c.id, group: w.group, platform: w.platform, name: w.name });
      onChange();
    } catch (e) {
      setError(err(e));
    }
  };
  const n = all.length;
  return html`<details class="warn-note collection-waiting" id="collection-waiting">
    <summary>${plural(n, "file waits", "files wait")} on Audit › Unknown: ${n === 1 ? "it joins" : "they join"} the collection once a DAT knows ${n === 1 ? "it" : "them"} (Add there puts files in the system's custom DAT).</summary>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    <ul class="ls-list">${all.map((w) => html`<li key=${`${w.group}/${w.platform}/${w.name}`} data-name=${w.name}>
      <span class="op-to">${w.name}</span> <span class="muted">${(ui.get().platforms || []).find((p) => p.id === w.platform)?.name || w.platform}${many ? ` · ${w.groupName}` : ""}</span>
      <button type="button" class="linkish" data-action="waiting-review" onClick=${() => reviewUnknown(w.platform)}>Review on Audit</button>
      <button type="button" class="linkish" data-action="waiting-leave" disabled=${off} onClick=${() => leave(w)}>Leave it</button></li>`)}</ul>
  </details>`;
}

// ------------------------------------------------------------ its source

/** What a collection or group from folders or a DAT was read from, and Read them again…
 *  `n` is the group's place (0, the first, is the collection's own). */
function Source({ c, g, n = 0, inner = false, off, onRead }) {
  const m = g.made_from;
  const paths = m.kind === "dat" ? [m.path] : m.paths || [];
  const at = n ? `-${n}` : "";
  const body = html`
    <ul class="ls-list" id=${`made-from-paths${at}`}>${paths.map((p) => html`<li key=${p}><span class="op-to">${p}</span>
      ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => reveal(p)}>${Icon.folder(14)}</button>` : null}</li>`)}</ul>
    <p class="muted">Read ${m.read?.slice(0, 10) || ""}: its games are kept as ${plural(g.rules.from_files?.length || 0, "DAT", "DATs")} of its own in the library (<code>_library/collections/${c.id}/${g.id ? `${g.id}/` : ""}</code>), with the Audit page's DATs' names and hashes, so renamed files and newer DATs still find them. ${m.kind === "dat" ? "The DAT" : "The folders"} can be changed or gone: the ${n ? "group" : "collection"} doesn't need them.</p>
    <div class="home-actions"><button type="button" class="ghost" id=${`collection-read-again${at}`} disabled=${off} onClick=${onRead}>${m.kind === "dat" ? "Read it again…" : "Read them again…"}</button></div>`;
  if (inner) return html`<div class="group-source" id=${`collection-source${at}`}><h3>Source</h3>${body}</div>`;
  return html`<section class="ls-section" id="collection-source"><h2>Source</h2>${body}</section>`;
}

// ------------------------------------------------------------ the page

const LIB_SORTS = [["title", "Title"], ["system", "System"], ["year", "Year"], ["size", "Size on disk"]];

export function Collections() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes, dats: st.dats, platforms: st.platforms }));
  const [all, setAll] = useState(null);
  const arg = useRouteArg("collections");
  const [fallback, setFallback] = useState(() => prefs()?.get("gr-collection", null));
  const id = arg || fallback;
  const [data, setData] = useState(null);
  const [exports, setExports] = useState([]);
  // how many games the backlog has to play, for its place at the top of the ribbon (D157)
  const [backlogCount, setBacklogCount] = useState(null);
  const [view, setView] = useState("games");
  // the games in it, or those left out by hand (a filter of the one view, finding 6)
  const [show, setShow] = useState("in");
  const [platform, setPlatform] = useState("");
  const [q, setQ] = useState("");
  const [sort, setSort] = useState("title");
  const [desc, setDesc] = useState(false);
  const [look, setLook] = useState(() => prefs()?.get("gr-collection-view", "grid") || "grid");
  const [jump, setJump] = useState(null);
  const [ready, setReady] = useState(false);
  const [tick, setTick] = useState(0);
  const off = !!s.library?.read_only;
  const again = () => setTick((t) => t + 1);
  const scrollRef = useRef(null);
  const open = (next, v = "games") => {
    setView(v);
    if (next === id) return;
    if (location.hash !== argHash("collections", next)) location.hash = argHash("collections", next);
    else setFallback(next);
  };
  // a new collection: its games, every system, the whole of it
  useEffect(() => { setPlatform(""); setQ(""); setShow("in"); setJump(null); }, [id]);

  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    Promise.all([api("collections"), api("exports"), api("backlog").catch(() => null)]).then(([cs, ex, bl]) => {
      if (gone) return;
      setAll(cs);
      setExports(ex);
      setBacklogCount(bl?.games?.length ?? null);
      if (!cs.some((c) => c.id === id)) {
        if (arg) history.replaceState(null, "", argHash("collections", cs[0]?.id || ""));
        setFallback(cs[0]?.id || null);
      }
    }).catch((e) => { if (!gone) setAll({ error: err(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, s.changes, tick, arg]);
  // the Library's rows, which the games are shown from
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("browse").then(() => { if (!gone) setReady(true); }).catch(() => {});
    return () => { gone = true; };
  }, [s.library?.path, s.changes]);

  useEffect(() => {
    if (!id) { setData(null); return; }
    prefs()?.set("gr-collection", id);
    let gone = false;
    api("collection_get", { id }).then((d) => { if (!gone) setData(d); }).catch((e) => { if (!gone) setData({ error: err(e) }); });
    return () => { gone = true; };
  }, [id, s.changes, s.dats, tick]);

  const c = data?.collection?.id === id ? data.collection : null;
  const sum = c ? data.summary : null;
  const groups = sum?.groups || [];
  const many = groups.length > 1;
  // the games in it (or left out of it), as the Library shows them: a section for each group
  const lq = useMemo(() => ({ ...blank(), collection: c?.id || "", all: true, platform: platform ? [platform] : [], search: q, sort, desc, left_out: show === "out", group: many ? "collection" : "" }),
    [c?.id, platform, q, sort, desc, show, many]);
  const games = useGames(lq, `${s.changes}|${c?.modified}|${sum?.games}|${sum?.removed}`, ready && !!c && view === "games");
  const picked = useSelection(games);
  useEffect(() => picked.clear(), [c?.id, platform, q, show]);

  // changes are saved one after another, each made to the collection as the last left it
  // (so quick changes, a tick then a game left out, don't undo each other)
  const latest = useRef(null);
  const pending = useRef(0);
  const chain = useRef(Promise.resolve());
  useEffect(() => { if (!pending.current) latest.current = c; }, [c]);
  const queue = (run) => {
    pending.current++;
    const done = chain.current.then(run);
    chain.current = done.then((r) => {
      latest.current = r.collection;
      pending.current--;
      setData(r);
    }, (e) => {
      if (!--pending.current) again();
      toast(err(e), 8000);
    });
    return done;
  };
  const apply = (change) => queue(() => save(typeof change === "function" ? change(latest.current) : change));
  const quietly = (change) => apply(change).catch(() => {});
  /** Games removed, put back or added by hand: each in the group that has them (D75). */
  const hand = (what) => queue(async () => {
    const r = await api("collection_hand", { id: latest.current.id, ...what });
    ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
    return r;
  }).catch(() => {});
  const create = (way = "rules") => ui.set({ dialog: { type: "collection-new", way, onMade: (made, v) => { open(made.id, v); again(); } } });
  const addGroup = () => ui.set({ dialog: { type: "collection-new", group: c, onMade: (_, v) => { setView(v === "rules" ? "rules" : "games"); again(); } } });
  const readAgain = (g) => ui.set({ dialog: { type: "collection-new", again: { id: c.id, name: g.id ? g.name : c.name, made_from: g.made_from, group: g.id }, onMade: () => again() } });
  const rename = () => ui.set({ dialog: { type: "collection-name", title: `Rename ${c.name}`, value: c.name, button: "Rename", run: (name) => apply((x) => ({ ...x, name })) } });
  const renameGroup = (g) => ui.set({ dialog: { type: "collection-name", title: `Rename the group ${g.name}`, value: g.name, button: "Rename", placeholder: "e.g. Translations",
    run: (name) => apply((x) => withGroup(x, g.id, (y) => ({ ...y, name }))) } });
  const removeGroup = (g, i) => confirmDialog({
    title: `Remove the group ${g.name}?`, button: "Remove",
    text: `Its rules and the games added to it or removed from it by hand go from ${c.name}, and with them its games no other group has.${i === 0 ? ` ${groupsOf(c)[1]?.name} becomes the first group.` : ""} History can undo it.`,
    undoable: true, run: () => apply((x) => withoutGroup(x, g.id)),
  });
  const duplicate = () => ui.set({ dialog: { type: "collection-name", title: `Duplicate ${c.name}`, value: `${c.name} (copy)`, button: "Duplicate", run: async (name) => {
    const r = await save({ ...c, id: "", name });
    open(r.collection.id);
    again();
  } } });
  const remove = () => confirmDialog({
    title: `Delete ${c.name}?`, text: `The collection's rules${groupsOf(c).some((g) => g.made_from) ? ", its own DATs" : ""}${many ? ", its groups" : ""} and the games added or removed by hand are deleted. Its games, DATs saved from it and its exports aren't touched.`, button: "Delete",
    // (its address goes too, so the page doesn't ask for it again)
    run: async () => { await api("collection_delete", { id: c.id }); setFallback(null); if (arg) location.hash = routeHash("collections"); else again(); },
  });
  const removeGames = (keys) => hand({ leave_out: keys });
  const putBack = (keys) => hand({ put_back: keys });
  /** A group's systems (by id; "" is the first): those its rules name, else every one with a DAT. */
  const systemsOf = (gid = "") => {
    const g = groupsOf(c).find((x) => x.id === gid) || groupsOf(c)[0];
    const ids = g.rules.platforms?.length ? g.rules.platforms : [...new Set((s.dats || []).filter((d) => d.current && d.platform !== "bios").map((d) => d.platform))];
    return ids.map((p) => ({ id: p, name: (s.platforms || []).find((x) => x.id === p)?.name || p })).sort((a, b) => a.name.localeCompare(b.name));
  };

  /** The games picked, every one (asked of the core when it's all of them). */
  const chosen = async () => (picked.all ? await api("browse_keys", { query: lq }) : [...picked.sel.values()]);
  const exportThese = async () => {
    // every game of a system as the system (its update takes in games added to it later); after
    // a search, only the games it found (review H16)
    const only = !picked.all ? [...picked.sel.keys()]
      : q.trim() ? (await chosen()).map((g) => g.key)
      : platform ? [`${platform}/*`] : sum.systems.map((x) => `${x.platform}/*`);
    ui.set({ dialog: { type: "export", c, sum, only, count: picked.all && q.trim() ? only.length : picked.count } });
  };
  const openGame = (g, i) => {
    Object.assign(opened, { query: { ...lq }, keys: null, index: i, total: games.total, key: g.key, name: c.name });
    location.hash = gameHash(g.key);
  };
  const gameActions = show === "out" ? [
    { id: "open", label: "Open", icon: "external", one: true, run: () => picked.one && openGame(picked.one, picked.anchor) },
    { id: "put-back", label: "Put back", icon: "plus", disabled: off, run: async () => { putBack((await chosen()).map((g) => g.key)); picked.clear(); } },
  ] : [
    { id: "open", label: "Open", icon: "external", one: true, run: () => picked.one && openGame(picked.one, picked.anchor) },
    { id: "export-these", label: "Export these…", icon: "stack", disabled: off, run: exportThese },
    { id: "remove-from", label: "Remove from collection", icon: "close", disabled: off, run: async () => { removeGames((await chosen()).map((g) => g.key)); picked.clear(); } },
  ];
  const barItems = () => gameActions.filter((a) => picked.count === 1 || !a.one);
  const onMenu = (e, g, i) => {
    e.preventDefault();
    const single = !picked.selected(g.key) || picked.count <= 1;
    picked.pickForMenu(g, i);
    const one = {
      "open": () => openGame(g, i), "remove-from": () => removeGames([g.key]), "put-back": () => putBack([g.key]),
      "export-these": () => ui.set({ dialog: { type: "export", c, sum, only: [g.key], count: 1 } }),
    };
    openMenu(e, gameActions.filter((a) => single || !a.one).map((a) => ({ ...a, run: a.id === "open" || single ? one[a.id] : a.run })));
  };
  usePageKeys((e) => {
    if (view !== "games") return;
    if (e.key === "Escape" && picked.count) { picked.clear(); return; }
    if (selectAllKey(e)) { e.preventDefault(); picked.selectAll(); return; }
    if (e.key === "Enter" && picked.one) openGame(picked.one, picked.anchor);
  });
  const pickLook = (v) => { setLook(v); prefs()?.set("gr-collection-view", v); };
  /** The games view at a group's section. */
  const toGroup = (i) => { setView("games"); setShow("in"); setPlatform(""); setJump({ value: i }); };

  // the collections, in the ribbon beside the menu; the current one's groups and systems
  // under it (D171), a system's a click to its games in the collection; the backlog at its
  // top (D157)
  const systems = sum?.systems?.length > 1 ? sum.systems : [];
  const toSystem = (p) => { setView("games"); setShow("in"); setJump(null); setPlatform(p); scrollRef.current?.scrollTo({ top: 0 }); };
  const under = [
    ...(many ? [...(systems.length ? [{ head: "Groups" }] : []), ...groups.map((g, i) => ({ id: g.id || "first", label: g.name, count: g.games, run: () => toGroup(i) }))] : []),
    ...(systems.length ? [...(many ? [{ head: "Systems" }] : []), ...systems.map((x) => ({ id: `system-${x.platform}`, label: x.name, count: x.games, active: view === "games" && platform === x.platform, run: () => toSystem(platform === x.platform && view === "games" ? "" : x.platform) }))] : []),
  ];
  useSidePanel(Array.isArray(all) ? {
    id: "collection-panel", route: "collections", title: "Collections", icon: "collection", count: all.length, current: id,
    top: [{ id: "backlog", label: "Backlog", icon: "list", href: routeHash("backlog"), count: backlogCount }],
    items: all.map((x) => ({
      id: x.id, label: x.name, sub: `${sourceText(x.made_from, x.rules)}${x.groups?.length ? ` · ${x.groups.length + 1} groups` : ""}`, href: argHash("collections", x.id),
      children: x.id === c?.id && under.length ? under : null,
      // (the one shown, clicked again: every system's games)
      run: x.id === c?.id && platform ? () => toSystem("") : null,
    })),
    actions: [{ id: "collection-panel-new", label: "New collection…", icon: "plus", disabled: off, run: () => create() }],
  } : null);

  const menu = () => c ? [
    { id: "collection-new-menu", label: "New collection…", icon: "plus", disabled: off, run: () => create() },
    { sep: true },
    { id: "collection-add-group", label: "Add a group…", icon: "layers", disabled: off, run: addGroup },
    { id: "collection-rename", label: "Rename…", icon: "edit", disabled: off, run: rename },
    { id: "collection-duplicate", label: "Duplicate…", icon: "copy", disabled: off, run: duplicate },
    { id: "collection-scrape", label: "Scrape its games…", icon: "scrape", disabled: off, run: () => scrapeDialog({ collection: c.id, platformName: c.name, gameName: c.name }) },
    { id: "collection-browse", label: "Show in the Library", icon: "grid", run: () => { location.hash = `#/library?collection=${encodeURIComponent(c.id)}`; } },
    { sep: true },
    { id: "collection-delete", label: "Delete…", icon: "trash", disabled: off, run: remove },
  ] : [];
  const selecting = view === "games" && picked.count;
  const head = html`<${CommandBar} id="collections-head" title=${html`<h1 id="collection-title">${c ? c.name : "Collections"}</h1>`} count=${c && sum ? `${plural(sum.games, "game", "games")} · ${size(sum.size)}` : Array.isArray(all) && all.length ? plural(all.length, "collection", "collections") : null}
    primary=${html`<span class="cmd-buttons">
      ${c ? html`<button type="button" class="ghost compact" id="collection-dats" title="One DAT per system of each group, for RomVault, clrmamepro or igir" onClick=${() => ui.set({ dialog: { type: "save-dats", c } })}>${Icon.download(15)} Save as DATs…</button>
        <button type="button" class="primary compact" id="collection-export" disabled=${off} title="The whole collection, to ES-DE, a device or a plain folder, with a preview and Undo" onClick=${() => ui.set({ dialog: { type: "export", c, sum } })}>${Icon.stack(15)} Export…</button>`
        : html`<button type="button" class="primary compact" id="collection-new" disabled=${off} onClick=${() => create()}>${Icon.plus(15)} New collection…</button>`}</span>`}
    more=${c ? { id: "collection-menu", title: "New, add a group, rename, duplicate, scrape, delete", items: menu } : null}
    selection=${selecting ? { count: `${picked.count.toLocaleString()} selected`, onClear: picked.clear, items: barItems, buttons: 3 } : null} />`;

  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!all) return html`${head}<div class="pages"><p class="muted">Reading the collections…</p></div>`;
  if (all.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${all.error}</p></div>`;
  if (!all.length) {
    return html`${head}<div class="pages"><div class="library-settings" id="collections-empty">
      <p class="page-lead">${Icon.collection(18)} A collection is a part of your audited games: picked by rules, or the games in folders you already keep, or a DAT's. Export all or part of it for ES-DE, a handheld or a folder, or save it as DATs.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <p>Collections are made from the Audit page's DATs and what the scan found. ${s.dats?.length ? "" : html`Import a DAT first (<a href=${argHash("import", "settings-dats")} id="collections-empty-import">Import › DAT files</a>).`}</p>
        <div class="home-actions">
          <button type="button" class="primary" id="collections-first" disabled=${off} onClick=${() => create("rules")}>New collection by rules…</button>
          <button type="button" class="ghost" id="collections-first-folders" disabled=${off} onClick=${() => create("folders")}>New collection from folders…</button>
          <button type="button" class="ghost" id="collections-first-dat" disabled=${off} onClick=${() => create("dat")}>New collection from a DAT…</button>
        </div>
      </section>
    </div></div>`;
  }
  const limit = c?.size_limit || 0;
  const views = [["games", "Games"], ["rules", c?.made_from ? "Source" : "Rules"], ["size", "Size"], ["exports", "Exported to", exports.filter((e) => e.collection === c?.id).length], ["history", "History"]];
  const sysTabs = sum && sum.systems.length > 1 ? [["", "All", sum.games], ...sum.systems.map((x) => [x.platform, x.name, x.games])] : [];
  const top = c ? html`<div class="collection-top">
    <p id="collection-totals"><b id="collection-games">${sum.games}</b> ${sum.games === 1 ? "game" : "games"}, <b id="collection-have">${sum.have}</b> you have${sum.removed ? html`, ${sum.removed} removed by hand` : null}${sum.addons ? html`, <span id="collection-addons">${plural(sum.addons, "update or DLC", "updates and DLC")}</span>` : null}${many ? html` · <span id="collection-group-count">${plural(groups.length, "group", "groups")}</span>` : null}${` · ${plural(sum.systems.length, "system", "systems")} · ${size(sum.size)}`}${limit ? html` <span class=${sum.size > limit ? "fit over" : "fit"} id="collection-fit-line">${sum.size > limit ? `over its ${size(limit)} limit by ${size(sum.size - limit)}` : `of ${size(limit)}, fits`}</span>` : null}
      <span class="muted"> · ${sourceText(c.made_from, c.rules)}</span></p>
    ${sum.no_dat.map((x) => html`<p class="warn-note" key=${x.platform}>${x.name} has no DAT, so its games aren't known.</p>`)}
    ${sum.missing_dats.map((x) => html`<p class="warn-note" key=${x}>The DAT ${x} isn't in the library any more.</p>`)}
    <${Waiting} c=${c} off=${off} onChange=${again} />
    <div class="seg audit-tabs" role="tablist" id="collection-view">${views.map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-view=${k} aria-selected=${view === k ? "true" : "false"} onClick=${() => setView(k)}>${label}${n ? html` <span class="tab-count">${n}</span>` : null}</button>`)}</div>
  </div>` : null;
  if (!c) return html`${head}<div class="pages collections-page" id="collections-page">${data?.error ? html`<p class="warn-note" role="alert">${data.error}</p>` : html`<p class="muted">Reading the collection…</p>`}</div>`;

  if (view === "games") {
    // made empty, or only of games added by hand (D141)
    const byHand = groupsOf(c).every((g) => !g.made_from && g.rules?.include === "none");
    const hint = show === "out" ? "These were removed by hand. Click a game to pick it (Ctrl or Shift for more, Ctrl+A for all of them), then Put back; double-click opens its page."
      : "Click a game to pick it (Ctrl or Shift for more, Ctrl+A for all of them), then Export these… or Remove from collection; double-click opens its page.";
    const tools = html`<div class="audit-tools collection-tools">
      ${sysTabs.length ? html`<select id="collection-systems" class="compact" aria-label="System" value=${platform} onChange=${(e) => setPlatform(e.target.value)}>
        ${sysTabs.map(([k, label, n]) => html`<option key=${k || "all"} value=${k}>${k ? label : "Every system"} (${n})</option>`)}</select>` : null}
      <select id="collection-show" class="compact" aria-label="Show" value=${show} onChange=${(e) => setShow(e.target.value)}>
        <option value="in">In it (${sum.games})</option>
        <option value="out">Removed by hand (${sum.removed})</option>
      </select>
      <input type="search" id="collection-search" class="compact" placeholder="Search the games" aria-label="Search the games" value=${q} onInput=${(e) => setQ(e.target.value)} />
      <${ViewSwitch} id="collection-look" views=${VIEWS} value=${look} onChange=${pickLook} />
      <${SortMenu} id="collection-sort" options=${LIB_SORTS} value=${sort} onChange=${(v) => { setSort(v); setDesc(false); }} />
      <button type="button" class="ghost compact" id="collection-desc" title=${desc ? "Reversed: back to the usual order" : "Reverse the order"} aria-pressed=${desc ? "true" : "false"} onClick=${() => setDesc(!desc)}>${desc ? "↑" : "↓"}</button>
      <button type="button" class="ghost compact" id="collection-add" disabled=${off} onClick=${() => ui.set({ dialog: { type: "add-games", c, groups, systemsOf, onDone: again } })}>${Icon.plus(14)} Add games…</button>
      <span class="collection-hint" id="collection-hint" tabindex="0" role="img" title=${hint} aria-label=${hint}>${Icon.help(16)}</span>
    </div>`;
    return html`${head}<div class="collection-games-page" id="collections-page">
      <div class="collection-games-head">${top}${tools}
        <${SearchNote} q=${q} found=${games.loading ? null : plural(games.total, "match", "matches")} onClear=${() => setQ("")} />
        ${games.error ? html`<p class="warn-note" role="alert">${games.error}</p>` : null}</div>
      <div class="lib-scroll collection-scroll" id="collection-result" data-show=${show} ref=${scrollRef} aria-busy=${games.loading ? "true" : "false"} onClick=${(e) => { if (e.target === e.currentTarget || e.target.classList.contains("lib-canvas")) picked.clear(); }}>
        ${!games.loading && !games.total ? (!q && show === "in" && byHand ? html`<div class="lib-none" id="collection-none" data-empty="hand">
          <p>No games yet. Add them here with <b>Add games…</b>, or pick games in the Library (or open a game's page) and choose <b>Add to a collection…</b>.</p>
          <div class="home-actions"><a class="button ghost" id="collection-to-library" href=${routeHash("library")}>Open the Library</a></div></div>`
          : html`<p class="muted lib-none" id="collection-none">${q ? "No game here matches the search." : show === "out" ? "No games removed by hand." : "No games: loosen the rules, or add games by hand."}</p>`) : null}
        ${ready ? html`<${Games} games=${games} q=${lq} set=${(p) => { if (p.sort) { setSort(p.sort); setDesc(!!p.desc); } }} view=${look} px="medium" selected=${picked.selected} onPick=${picked.pick} onOpen=${openGame} onMenu=${onMenu} scrollRef=${scrollRef} jump=${jump} />` : null}
      </div>
    </div>`;
  }
  const rulesOf = (g) => (patch) => quietly((x) => withGroup(x, g.id, (y) => ({ ...y, rules: { ...y.rules, ...patch } })));
  const ruled = groupsOf(c);
  return html`${head}<div class="pages collections-page" id="collections-page">${top}
    <div class="collection-main" id="collection-main">
      ${view === "rules" && !many ? html`<div class="collection-rules-view">
        ${c.made_from ? html`<${Source} c=${c} g=${ruled[0]} off=${off} onRead=${() => readAgain(ruled[0])} />` : null}
        <section class="ls-section collection-rules-section">
          <h2>${c.made_from ? "More rules" : "Rules"}</h2>
          ${c.made_from ? html`<p class="muted">Every game of its own DATs is in it. Rules narrow it further: one per title, regions, the flags removed.</p>` : null}
          <${RulesEditor} key=${c.id} rules=${c.rules} off=${off} onSet=${rulesOf(ruled[0])} />
        </section>
        <section class="ls-section" id="collection-groups-none">
          <h2>Groups</h2>
          <p class="muted">A group is a part of the collection with its own way of choosing games (a system's translations, a best-of DAT), shown as a section of its games; a game an earlier group has isn't in a later one. Exported, each group is a collection, a playlist or a folder of its own too.</p>
          <div class="home-actions"><button type="button" class="ghost" id="collection-add-group" disabled=${off} onClick=${addGroup}>${Icon.plus(14)} Add a group…</button></div>
        </section></div>` : null}
      ${view === "rules" && many ? html`<div class="collection-rules-view">
        ${ruled.map((g, i) => html`<section class="ls-section collection-group" key=${g.id || "first"} id=${`collection-group-${i}`} data-group=${g.id}>
          <div class="group-head"><h2>${g.name}</h2>
            <span class="muted">${plural(groups[i]?.games ?? 0, "game", "games")} · ${sourceText(g.made_from, g.rules)}</span>
            <button type="button" class="ghost compact" data-action="group-rename" disabled=${off} onClick=${() => renameGroup(g)}>Rename…</button>
            <button type="button" class="ghost compact" data-action="group-remove" disabled=${off} onClick=${() => removeGroup(g, i)}>Remove…</button></div>
          ${g.made_from ? html`<${Source} c=${c} g=${g} n=${i} inner=${true} off=${off} onRead=${() => readAgain(g)} />` : null}
          ${g.made_from ? html`<h3>More rules</h3><p class="muted">Every game of its own DATs is in it. Rules narrow it further: one per title, regions, the flags removed.</p>` : null}
          <${RulesEditor} key=${`${c.id}/${g.id}`} rules=${g.rules} off=${off} onSet=${rulesOf(g)} prefix=${i ? `rule-${i}` : "rule"} id=${i ? `collection-rules-${i}` : "collection-rules"} />
        </section>`)}
        <div class="home-actions"><button type="button" class="ghost" id="collection-add-group" disabled=${off} onClick=${addGroup}>${Icon.plus(14)} Add a group…</button></div>
      </div>` : null}
      ${view === "size" ? html`<${Properties} c=${c} sum=${sum} off=${off} onSave=${quietly} onPick=${(p) => { setPlatform(p); setShow("in"); setView("games"); }} />` : null}
      ${view === "exports" ? html`<${Exports} c=${c} sum=${sum} exports=${exports} off=${off} onChange=${again} />` : null}
      ${view === "history" ? html`<${History} c=${c} off=${off} onChange=${again} />` : null}
    </div>
  </div>`;
}
