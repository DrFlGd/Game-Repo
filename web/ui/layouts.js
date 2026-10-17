// Export layouts (docs/PLAN.md, "Phase 5b design", D36–D39): where an export puts the
// games, gamelists, pictures, descriptions, manuals and BIOS files, for ES-DE, each
// device's firmware or a plain folder. Each is a JSON file; the built-in ones are
// changed by saving a copy. The editor shows what an export would write for a few of
// the user's own games as a folder tree: a file or folder chosen in it is changed on
// the right, the tree follows at once, and the JSON can be edited as text.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, plural } from "./library.js";
import { CommandBar } from "./layout.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
const prefs = () => ctx.platform?.store?.prefs;
const clone = (v) => JSON.parse(JSON.stringify(v));

/** What each kind a layout can place is called. */
export const KINDS = {
  cover: "Box front", back: "Box back", box3d: "3D box", screenshot: "Screenshot", titlescreen: "Title screen",
  marquee: "Marquee (logo)", fanart: "Fan art", physical: "Cartridge or disc", video: "Video", manual: "Manual",
};
const FIELDS = { description: "Description", developer: "Developer", publisher: "Publisher", released: "Release date", genres: "Genres", players: "Players", rating: "Rating" };

/** The tokens each part of a layout can use, and what they stand for. */
const SYSTEM_TOKENS = ["folder", "system", "system_name", "dat_name"];
const GAME_TOKENS = ["file", "stem", "title"];
const TOKEN_HELP = {
  folder: "the system's folder in this layout", system: "the system's id (gb)", system_name: "the firmware's name for the system", dat_name: "the system's DAT name",
  file: "the game's file or folder", stem: "its name without the extension", title: "the game's title", ext: "the picture's own extension",
  kind: "the kind of picture", doc_file: "the document's own name", bios_file: "the BIOS file's name, with its folder",
};
function tokensFor(at) {
  const what = at.split(".")[0];
  if (what === "games") return [...SYSTEM_TOKENS, "file"];
  if (what === "gamelist") return SYSTEM_TOKENS;
  if (what === "bios") return [...SYSTEM_TOKENS, "bios_file"];
  if (what === "documents") return [...SYSTEM_TOKENS, ...GAME_TOKENS, "ext", "doc_file"];
  return [...SYSTEM_TOKENS, ...GAME_TOKENS, "ext", "kind", ...(at === "media.manual" ? ["doc_file"] : [])];
}

/** The part of a layout `at` names (games.path, media.cover, systems.gb, gamelist.path…). */
function partOf(l, at) {
  const [a, b] = at.split(".");
  if (a === "games") return l.games;
  if (a === "gamelist") return l.gamelist;
  if (a === "media" || a === "text") return l[a]?.[b];
  if (a === "documents" || a === "bios") return l[a];
  return null;
}

/** What a part of the layout is, in words. */
function partName(at) {
  const [a, b] = at.split(".");
  if (a === "games") return "The games";
  if (a === "gamelist") return "Each system's gamelist";
  if (a === "media") return KINDS[b] || b;
  if (a === "text") return `${FIELDS[b] || b} (as a text file)`;
  if (a === "documents") return "Every document";
  if (a === "bios") return "BIOS files";
  if (a === "systems") return "A system's folder";
  if (a === "collection") return "The collection file";
  return at;
}

/** The layout as a flat list of "a.b.c": value, to say what a copy changed. */
function flatten(v, at = "", out = {}) {
  if (v && typeof v === "object" && !Array.isArray(v)) {
    for (const [k, x] of Object.entries(v)) flatten(x, at ? `${at}.${k}` : k, out);
  } else out[at] = JSON.stringify(v);
  return out;
}

// ------------------------------------------------------------ the tree

/** The preview's files as a tree of folders, each place at the top. */
function buildTree(preview) {
  const root = { name: "", children: new Map(), path: "" };
  for (const it of preview.items || []) {
    const parts = it.path.split("/");
    let node = root;
    parts.forEach((p, i) => {
      const path = parts.slice(0, i + 1).join("/");
      if (!node.children.has(p)) node.children.set(p, { name: p, children: new Map(), path });
      node = node.children.get(p);
    });
    if (it.kind === "system") node.system = it;
    else node.item = it;
  }
  return root;
}

function TreeNode({ node, depth, places, chosen, onChoose, open, setOpen }) {
  const kids = [...node.children.values()].sort((a, b) => (a.children.size ? 0 : 1) - (b.children.size ? 0 : 1) || a.name.localeCompare(b.name));
  const folder = node.children.size > 0 && !node.item;
  const shut = open[node.path] === false;
  const it = node.item || node.system;
  const label = depth === 0 ? places[node.name] || node.name : node.name;
  const active = it && chosen === it.at;
  return html`<li class=${`lt-node${it?.faint ? " faint" : ""}${it?.clash ? " clash" : ""}`} data-path=${node.path} data-at=${it?.at || null}>
    <div class=${`lt-row${active ? " active" : ""}`} style=${`--lt-depth:${depth}`}>
      ${folder ? html`<button type="button" class="lt-fold" aria-label=${shut ? "Open" : "Close"} aria-expanded=${shut ? "false" : "true"} onClick=${() => setOpen({ ...open, [node.path]: shut })}>${shut ? "▸" : "▾"}</button>` : html`<span class="lt-fold"></span>`}
      <button type="button" class="lt-name" disabled=${!it} title=${it ? `${it.what}: change it on the right` : null} onClick=${() => it && onChoose(it)}>
        <span class="lt-icon" aria-hidden="true">${Icon[depth === 0 ? "archive" : folder ? "folder" : it?.kind === "media" ? "image" : it?.kind === "game" ? "gamepad" : "file"](14)}</span>
        <span class="lt-file">${label}</span>
        ${it ? html`<span class="muted lt-what">${it.what}${it.faint ? " · none of these games has one" : ""}${it.clash ? " · the same file as another" : ""}</span>` : null}
      </button>
    </div>
    ${folder && !shut ? html`<ul class="lt-list">${kids.map((k) => html`<${TreeNode} key=${k.path} node=${k} depth=${depth + 1} places=${places} chosen=${chosen} onChoose=${onChoose} open=${open} setOpen=${setOpen} />`)}</ul>` : null}
  </li>`;
}

// ------------------------------------------------------------ the right-hand side

/** A path with its tokens as buttons to put in where the cursor is, and what it comes to. */
function PathField({ id, value, at, onChange, example }) {
  const ref = useRef(null);
  const put = (t) => {
    const el = ref.current;
    const [a, b] = el ? [el.selectionStart ?? value.length, el.selectionEnd ?? value.length] : [value.length, value.length];
    onChange(value.slice(0, a) + `{${t}}` + value.slice(b));
    requestAnimationFrame(() => { if (el) { el.focus(); el.setSelectionRange(a + t.length + 2, a + t.length + 2); } });
  };
  return html`<div class="field-block"><span class="field-label">Where it goes</span>
    <input type="text" id=${id} ref=${ref} class="lt-path" spellcheck="false" value=${value} onInput=${(e) => onChange(e.target.value)} />
    <div class="lt-tokens" aria-label="Put in">${tokensFor(at).map((t) => html`<button type="button" key=${t} class="ghost compact" data-token=${t} title=${TOKEN_HELP[t]} onClick=${() => put(t)}>{${t}}</button>`)}</div>
    ${example ? html`<small class="muted" id="lt-example">For ${example.game || "the sample"}: <code>${example.path}</code></small>` : null}
  </div>`;
}

function Inspector({ draft, at, preview, setDraft, choose }) {
  const set = (fn) => { const next = clone(draft); fn(next); setDraft(next); };
  const items = (preview?.items || []).filter((i) => i.at === at);
  const example = items.find((i) => !i.faint) || items[0];
  const problems = (preview?.problems || []).filter((p) => p.at === at || p.at.startsWith(`${at}.`));
  const showProblems = problems.length ? html`<ul class="lt-problems">${problems.map((p, i) => html`<li key=${i} class="warn-note">${p.message}</li>`)}</ul>` : null;
  if (at && at.startsWith("systems.")) {
    const sys = at.slice(8);
    const cur = draft.systems?.[sys];
    const off = cur === null;
    const folder = cur == null ? sys : typeof cur === "string" ? cur : cur.folder;
    const name = cur && typeof cur === "object" ? cur.name || "" : "";
    const put = (patch) => set((l) => {
      l.systems = l.systems || {};
      const f = patch.folder ?? folder;
      const n = patch.name ?? name;
      l.systems[sys] = patch.off ? null : n ? { folder: f, name: n } : f;
      if (!patch.off && f === sys && !n) delete l.systems[sys];
    });
    return html`<div class="lt-inspect" id="lt-inspect" data-at=${at}>
      <h3>${example?.what || "A system's folder"}</h3>
      <label class="check-row"><input type="checkbox" id="lt-system-off" checked=${off} onChange=${(e) => put({ off: e.target.checked })} /> Not on this device (its games are left out)</label>
      ${off ? null : html`
        <label class="field-block"><span class="field-label">Its folder</span>
          <input type="text" id="lt-system-folder" spellcheck="false" value=${folder} onInput=${(e) => put({ folder: e.target.value })} /></label>
        <label class="field-block"><span class="field-label">The firmware's name for it <span class="muted">({system_name}, optional)</span></span>
          <input type="text" id="lt-system-name" spellcheck="false" placeholder="As Game Repo names it" value=${name} onInput=${(e) => put({ name: e.target.value })} /></label>`}
      ${showProblems}
    </div>`;
  }
  if (at === "collection") {
    return html`<div class="lt-inspect" id="lt-inspect" data-at=${at}><h3>The collection file</h3>
      <label class="check-row"><input type="checkbox" id="lt-collection" checked=${draft.collection === "esde-cfg"} onChange=${(e) => set((l) => { l.collection = e.target.checked ? "esde-cfg" : null; })} /> Write the collection as an ES-DE custom collection (in <code>{esde}/collections/</code>)</label></div>`;
  }
  const part = at ? partOf(draft, at) : null;
  if (part) {
    const [a, b] = at.split(".");
    const leave = () => set((l) => {
      if (a === "media" || a === "text") delete l[a][b];
      else if (a === "gamelist" || a === "documents" || a === "bios") l[a] = null;
    });
    const isGames = a === "games";
    return html`<div class="lt-inspect" id="lt-inspect" data-at=${at}>
      <h3>${partName(at)}</h3>
      <${PathField} id="lt-path" at=${at} value=${part.path || ""} example=${example} onChange=${(v) => set((l) => { partOf(l, at).path = v; })} />
      ${isGames ? html`<label class="field-block"><span class="field-label">A game of several files</span>
        <select id="lt-style" class="compact" value=${part.style || "frontend"} onChange=${(e) => set((l) => { l.games.style = e.target.value; })}>
          <option value="frontend">A folder named as its cue sheet (as ES-DE and the firmwares show it)</option>
          <option value="naming">As Settings › Naming says</option>
        </select></label>` : null}
      ${a === "media" && draft.gamelist ? html`<label class="field-block"><span class="field-label">Its gamelist tag <span class="muted">(optional: the path is written in the game's entry)</span></span>
        <input type="text" id="lt-tag" spellcheck="false" placeholder="none" value=${part.tag || ""} onInput=${(e) => set((l) => { const v = e.target.value.trim(); if (v) partOf(l, at).tag = v; else delete partOf(l, at).tag; })} /></label>` : null}
      ${a === "gamelist" ? html`<div class="field-block"><span class="field-label">The tag for each detail</span>
        <div class="lt-fields">${Object.entries(FIELDS).map(([k, n]) => html`<label key=${k} class="field-inline"><span>${n}</span>
          <input type="text" class="compact" data-field=${k} spellcheck="false" placeholder="not written" value=${part.fields?.[k] || ""}
            onInput=${(e) => set((l) => { const v = e.target.value.trim(); l.gamelist.fields = l.gamelist.fields || {}; if (v) l.gamelist.fields[k] = v; else delete l.gamelist.fields[k]; })} /></label>`)}</div></div>` : null}
      ${showProblems}
      ${isGames ? null : html`<div class="home-actions"><button type="button" class="ghost" id="lt-leave" onClick=${() => { leave(); choose(null); }}>Leave it out</button></div>`}
    </div>`;
  }
  return html`<${Overview} draft=${draft} set=${set} choose=${choose} />`;
}

/** Nothing chosen: the layout's name, its pictures, and the kinds it can have added. */
function Overview({ draft, set, choose }) {
  const gamesDir = (draft.games?.path || "{card}/{folder}/{file}").replace(/\/\{file\}$/, "");
  const missing = Object.keys(KINDS).filter((k) => !draft.media?.[k]);
  const add = (kind) => set((l) => {
    l.media = l.media || {};
    l.media[kind] = { path: kind === "manual" ? `${gamesDir}/manuals/{stem}.pdf` : `${gamesDir}/${kind}/{stem}.{ext}` };
  });
  return html`<div class="lt-inspect" id="lt-inspect" data-at="">
    <h3>The layout</h3>
    <label class="field-block"><span class="field-label">Name</span>
      <input type="text" id="lt-name" maxlength="80" value=${draft.name || ""} onInput=${(e) => set((l) => { l.name = e.target.value; })} /></label>
    <label class="field-block"><span class="field-label">What it's for</span>
      <textarea id="lt-about" rows="3" onInput=${(e) => set((l) => { l.about = e.target.value; })}>${draft.about || ""}</textarea></label>
    <div class="field-block"><span class="field-label">Pictures</span>
      <div class="pick-row">
        <select id="lt-pictures-max" class="compact" aria-label="Their size" value=${String(draft.pictures?.max || 0)} onChange=${(e) => set((l) => { l.pictures = { ...(l.pictures || {}), max: Number(e.target.value) }; })}>
          <option value="0">As they are</option>
          ${[320, 480, 640, 800, 1080].map((n) => html`<option key=${n} value=${String(n)}>At most ${n} pixels</option>`)}
        </select>
        <select id="lt-pictures-as" class="compact" aria-label="Their format" value=${draft.pictures?.as || "keep"} onChange=${(e) => set((l) => { l.pictures = { ...(l.pictures || {}), as: e.target.value }; })}>
          <option value="keep">In their own format</option><option value="png">As PNG</option><option value="jpg">As JPEG</option>
        </select></div>
      <small class="muted">The size the Export dialog starts from for this device; it can be changed there.</small></div>
    <div class="field-block"><span class="field-label">Add to it</span>
      <div class="lt-tokens" id="lt-add">
        ${missing.map((k) => html`<button type="button" key=${k} class="ghost compact" data-add=${k} onClick=${() => { add(k); choose({ at: `media.${k}` }); }}>${Icon.plus(13)} ${KINDS[k]}</button>`)}
        ${draft.gamelist ? null : html`<button type="button" class="ghost compact" data-add="gamelist" onClick=${() => { set((l) => { l.gamelist = { kind: "es-xml", path: `${gamesDir}/gamelist.xml`, fields: { description: "desc", developer: "developer", publisher: "publisher", released: "releasedate", genres: "genre", players: "players", rating: "rating" } }; }); choose({ at: "gamelist.path" }); }}>${Icon.plus(13)} A gamelist</button>`}
        ${draft.text?.description ? null : html`<button type="button" class="ghost compact" data-add="text" onClick=${() => { set((l) => { l.text = { ...(l.text || {}), description: { path: `${gamesDir}/text/{stem}.txt` } }; }); choose({ at: "text.description" }); }}>${Icon.plus(13)} The description as text</button>`}
        ${draft.documents ? null : html`<button type="button" class="ghost compact" data-add="documents" onClick=${() => { set((l) => { l.documents = { path: `${gamesDir}/docs/{stem}/{doc_file}` }; }); choose({ at: "documents" }); }}>${Icon.plus(13)} Every document</button>`}
        ${draft.bios ? null : html`<button type="button" class="ghost compact" data-add="bios" onClick=${() => { const place = (draft.games?.path || "{card}").match(/^\{[^}]+\}/)?.[0] || "{card}"; set((l) => { l.bios = { path: `${place}/bios/{bios_file}` }; }); choose({ at: "bios" }); }}>${Icon.plus(13)} BIOS files</button>`}
      </div></div>
    <p class="muted">Choose a file or folder in the tree to change where it goes. The places a layout asks for (${(draft.places || []).filter((p) => !p.from).map((p) => `{${p.id}}`).join(", ")}) can be changed in its JSON.</p>
  </div>`;
}

// ------------------------------------------------------------ dialogs

function SaveAsDialog({ draft, onSaved }) {
  const [name, setName] = useState(draft.copied_from || !draft.name?.endsWith("(mine)") ? `${draft.name} (mine)`.replace(/ \(mine\) \(mine\)$/, " (mine)") : draft.name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await api("layout_save", { layout: { ...draft, name: name.trim() }, as_new: true });
      close();
      onSaved(r);
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Save as a new layout" id="layout-save-dialog" onSubmit=${submit} busy=${busy || !name.trim()} error=${error} submitLabel="Save">
    <label class="field-block"><span class="field-label">Its name</span>
      <input type="text" id="layout-save-name" maxlength="80" value=${name} onInput=${(e) => setName(e.target.value)} /></label>
    <p class="muted">It's kept in the library (_library/layouts/), so it goes where the library goes, and it's offered in the Export dialog.</p>
  <//>`;
}

export function LayoutDialogs({ dialog }) {
  if (dialog.type === "layout-save") return html`<${SaveAsDialog} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ the page

export function Layouts() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [list, setList] = useState(null);
  const [id, setId] = useState(prefs()?.get("gr-layout", "") || "");
  const [base, setBase] = useState(null);
  const [draft, setDraft] = useState(null);
  const [preview, setPreview] = useState(null);
  const [collections, setCollections] = useState([]);
  const [collection, setCollection] = useState(prefs()?.get("gr-layout-sample", "") || "");
  const [chosen, setChosen] = useState(null);
  const [open, setOpen] = useState({});
  const [tab, setTab] = useState("tree");
  const [text, setText] = useState("");
  const [textError, setTextError] = useState("");
  const [compare, setCompare] = useState(null);
  const load = async (want) => {
    const l = await api("layouts");
    setList(l);
    const pick = l.find((x) => x.id === want) ? want : l[0]?.id;
    if (pick) await select(pick, l);
  };
  const select = async (lid) => {
    const r = await api("layout_get", { id: lid });
    setId(lid);
    prefs()?.set("gr-layout", lid);
    setBase(r);
    setDraft(clone(r.layout));
    setText(JSON.stringify(r.layout, null, 2));
    setTextError("");
    setChosen(null);
    setCompare(null);
  };
  useEffect(() => {
    if (!s.library) return;
    load(id).catch((e) => toast(`Couldn't read the layouts: ${err(e)}`, 8000));
    api("collections").then(setCollections).catch(() => {});
  }, [s.library?.path]);
  useEffect(() => {
    if (!draft) return;
    let gone = false;
    const t = setTimeout(() => {
      api("layout_preview", { layout: draft, collection })
        .then((p) => { if (!gone) setPreview(p); })
        .catch((e) => { if (!gone) setPreview({ items: [], problems: [{ at: "", message: err(e) }], places: [] }); });
    }, 200);
    return () => { gone = true; clearTimeout(t); };
  }, [draft, collection]);
  const summary = list?.find((x) => x.id === id);
  const builtin = !!summary?.builtin;
  const dirty = !!(draft && base && JSON.stringify(draft) !== JSON.stringify(base.layout));
  const edit = (next) => { setDraft(next); setText(JSON.stringify(next, null, 2)); setTextError(""); };
  const editText = (v) => {
    setText(v);
    try {
      const parsed = JSON.parse(v);
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("A layout is a JSON object ({ … }).");
      setDraft(parsed);
      setTextError("");
    } catch (e) {
      setTextError(err(e));
    }
  };
  const saved = async (r) => {
    toast(`Saved the layout ${r.summary.name}.`);
    await load(r.summary.id);
  };
  const save = async () => {
    if (builtin) return ui.set({ dialog: { type: "layout-save", draft, onSaved: saved } });
    try {
      await saved(await api("layout_save", { layout: draft }));
    } catch (e) {
      toast(`Couldn't save it: ${err(e)}`, 10000);
    }
  };
  const importOne = async () => {
    const p = await ctx.platform.library.pickFile("A layout to bring in (.json)", ["json"]);
    if (!p) return;
    try {
      const r = await api("layout_import", { path: p });
      toast(`Brought in the layout ${r.name}.`);
      await load(r.id);
    } catch (e) {
      toast(`Couldn't bring it in: ${err(e)}`, 10000);
    }
  };
  const writeOne = async () => {
    const dir = await ctx.platform.library.pickFolder(`The folder ${summary?.name}'s file goes to`);
    if (!dir) return;
    try {
      const to = `${dir.replace(/[\\/]+$/, "")}${dir.includes("\\") ? "\\" : "/"}${id}.json`;
      const r = await api("layout_write", { id, to });
      toast(`Saved ${r.path}.`, 8000);
    } catch (e) {
      toast(`Couldn't save the file: ${err(e)}`, 10000);
    }
  };
  const showFolder = async () => {
    const r = await api("layout_folder");
    ctx.platform.library.openPath(r.path);
  };
  const remove = () => confirmDialog({
    title: `Remove ${summary?.name}?`, text: "The layout's file is deleted from the library. Exports made with it need another layout before they're updated.", button: "Remove",
    run: async () => { await api("layout_remove", { id }); prefs()?.set("gr-layout", ""); await load(""); },
  });
  const showCompare = async () => {
    if (compare) return setCompare(null);
    const from = summary?.copied_from?.id;
    if (!from) return;
    const b = (await api("layout_get", { id: from })).layout;
    const [x, y] = [flatten(b), flatten(draft)];
    const skip = new Set(["id", "name", "version", "copied_from.id", "copied_from.version", "format"]);
    const keys = [...new Set([...Object.keys(x), ...Object.keys(y)])].filter((k) => !skip.has(k) && x[k] !== y[k]).sort();
    setCompare({ name: summary.copied_from.name || from, rows: keys.map((k) => [k, x[k], y[k]]) });
  };
  const menu = () => [
    { id: "layout-import", label: "Import a layout…", icon: "download", disabled: off || !isDesktop(), run: importOne },
    { id: "layout-write", label: "Save as a file…", icon: "file", disabled: !isDesktop() || !id, run: writeOne },
    { id: "layout-folder", label: "Open the layouts folder", icon: "folder", disabled: !isDesktop(), run: showFolder },
    { sep: true },
    { id: "layout-save-as", label: "Save as a new layout…", icon: "copy", disabled: off || !draft, run: () => ui.set({ dialog: { type: "layout-save", draft, onSaved: saved } }) },
    { id: "layout-revert", label: "Undo the changes", icon: "refresh", disabled: !dirty, run: () => edit(clone(base.layout)) },
    { id: "layout-remove", label: "Remove this layout", icon: "trash", disabled: off || builtin || !id, run: remove },
  ];
  const head = html`<${CommandBar} id="layouts-head" up=${[["Settings", routeHash("settings")]]} title="Export layouts" count=${list ? plural(list.length, "layout", "layouts") : null}
    primary=${html`<button type="button" class="primary compact" id="layout-save" disabled=${off || !draft || (!dirty && !builtin) || !!textError || !!preview?.problems?.length}
      title=${preview?.problems?.length ? "Put the problems right first" : builtin ? "A built-in layout is kept as it is: your changes become a new layout" : "Save the changes"} onClick=${save}>${builtin ? "Save as new…" : "Save"}</button>`}
    more=${{ id: "layouts-menu", title: "Import, save as a file, remove", items: menu }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!list || !draft) return html`${head}<div class="pages"><p class="muted">Reading the layouts…</p></div>`;
  const places = Object.fromEntries((preview?.places || []).map((p) => [p.id, p.label]));
  const tree = preview ? buildTree(preview) : null;
  const choose = (it) => setChosen(it ? it.at : null);
  return html`${head}<div class="pages layouts-page" id="layouts-page">
    <p class="page-lead">${Icon.archive(18)} Where an export puts the games, gamelists, pictures, manuals and BIOS files for ES-DE, a device's firmware or a folder. Change one for a new firmware or frontend: a built-in layout is kept, and your change is saved as a new one.</p>
    <div class="layouts-body">
      <ul class="pc-list lt-layouts" id="layout-list">${list.map((l) => html`<li key=${l.id}>
        <button type="button" class=${`pc-row${l.id === id ? " active" : ""}`} data-id=${l.id} aria-pressed=${l.id === id ? "true" : "false"} onClick=${() => {
          if (l.id === id) return;
          if (dirty) confirmDialog({ title: "Leave your changes?", text: `The changes to ${summary?.name} aren't saved.`, button: "Leave them", run: () => select(l.id) });
          else select(l.id);
        }}>
          <span class="pc-row-main"><span class="pc-row-title">${l.name}</span>
            <span class="muted pc-row-meta">${l.builtin ? "built in" : l.copied_from ? `yours, from ${l.copied_from.name || l.copied_from.id}${l.copied_from.newer ? " (changed since)" : ""}` : "yours"}</span></span>
        </button></li>`)}</ul>
      <div class="lt-middle">
        <div class="audit-pick">
          <div class="seg" role="tablist" aria-label="Show">
            <button type="button" role="tab" id="lt-tab-tree" aria-selected=${tab === "tree" ? "true" : "false"} class=${tab === "tree" ? "active" : ""} onClick=${() => setTab("tree")}>On the card</button>
            <button type="button" role="tab" id="lt-tab-json" aria-selected=${tab === "json" ? "true" : "false"} class=${tab === "json" ? "active" : ""} onClick=${() => setTab("json")}>JSON</button>
          </div>
          <label class="field-inline"><span>Shown with</span>
            <select id="lt-sample" class="compact" value=${collection} onChange=${(e) => { setCollection(e.target.value); prefs()?.set("gr-layout-sample", e.target.value); }}>
              <option value="">Example games</option>
              ${collections.map((c) => html`<option key=${c.id} value=${c.id}>${c.name}'s first games</option>`)}
            </select></label>
          ${summary?.copied_from ? html`<button type="button" class="ghost compact" id="lt-compare" onClick=${showCompare}>${compare ? "Hide the comparison" : `Compare with ${summary.copied_from.name || "the built-in"}`}</button>` : null}
        </div>
        ${builtin ? html`<p class="muted" id="lt-builtin-note">Built in: it isn't changed. Change anything and save it as a new layout.</p>` : null}
        ${dirty ? html`<p class="warn-note" id="lt-dirty">Not saved yet.</p>` : null}
        ${compare ? html`<div class="field-block" id="lt-compare-list"><span class="field-label">What it changed from ${compare.name}</span>
          ${compare.rows.length ? html`<ul class="lt-diff">${compare.rows.map(([k, a, b]) => html`<li key=${k}><code>${k}</code>: ${a ?? "(none)"} → ${b ?? "(none)"}</li>`)}</ul>` : html`<p class="muted">Nothing: it's the same.</p>`}</div>` : null}
        ${tab === "json" ? html`<div class="field-block">
            <textarea id="lt-json" class="lt-json" spellcheck="false" rows="28" onInput=${(e) => editText(e.target.value)}>${text}</textarea>
            ${textError ? html`<p class="warn-note" id="lt-json-error" role="alert">${textError}</p>` : null}</div>`
          : html`<div class="lt-tree" id="lt-tree" aria-busy=${preview ? "false" : "true"}>
            ${preview?.stand_in ? html`<p class="muted" id="lt-stand-in">Shown with example games: choose a collection above to see your own games' names.</p>` : null}
            ${tree ? html`<ul class="lt-list lt-root">${[...tree.children.values()].map((n) => html`<${TreeNode} key=${n.path} node=${n} depth=${0} places=${places} chosen=${chosen} onChoose=${choose} open=${open} setOpen=${setOpen} />`)}</ul>` : null}
            ${(preview?.left || []).map((x) => html`<p key=${x.system} class="muted lt-left" data-system=${x.system}>${x.system_name}: not on this device. <button type="button" class="linkish" onClick=${() => setChosen(x.at)}>Change</button></p>`)}
          </div>`}
        ${preview?.problems?.length ? html`<ul class="lt-problems" id="lt-problems">${preview.problems.map((p, i) => html`<li key=${i} class="warn-note">
          <button type="button" class="linkish" onClick=${() => setChosen(p.at.split(".").slice(0, p.at.startsWith("gamelist.fields") ? 1 : 2).join(".").replace(/^gamelist$/, "gamelist.path"))}>${partName(p.at)}</button>: ${p.message}</li>`)}</ul>` : null}
      </div>
      <aside class="pc-panel lt-panel">
        ${chosen ? html`<button type="button" class="ghost compact" id="lt-back" onClick=${() => setChosen(null)}>‹ The layout</button>` : null}
        <${Inspector} draft=${draft} at=${chosen} preview=${preview} setDraft=${edit} choose=${choose} />
      </aside>
    </div>
  </div>`;
}
