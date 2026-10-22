// Magazines (docs/PLAN.md, "Phase 8 design", D68, D154): each magazine in the library's
// Magazines/<Title>/ folder with its issues, their covers (the first page, or a picture
// chosen for it), and how they stand against the lists kept to audit them (a CSV of
// magazine, number and date, or a DAT of issues). Import magazines… reads a folder into a
// review list (each file's magazine, number and date worked out from its name, put right
// here) and brings them in as a change with a preview and Undo. The page is laid out as the
// Library is (shelf.js): every issue a tile, filtered by magazine, publisher, tag, year,
// system, text and what it has, grouped by magazine (each heading with its menu: rename,
// publisher, read its text, what's missing), and the missing issues as faded tiles when
// asked. Issues are moved to another magazine as a change with Undo, and tagged. An issue
// opens in the document viewer: PDFs as documents are, CBZ and CBR a page at a time. Their
// text is read page by page (D145: a PDF's own, else text recognition), searched on the
// Documents page, and the games in it are listed with each issue, ticked when sure, and on
// the games' pages. `#/magazines/<title>` shows that magazine's issues.
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, EditBox } from "./layout.js";
import { Dialog, More, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { importFrom } from "./importpage.js";
import { viewDoc, useDroppedFiles } from "./documents.js";
import { systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { useRouteArg } from "./panels.js";
import { Shelf, useShelfState, Pic, PanelActions, TagEdit, askValue, byName } from "./shelf.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const PICTURE = /\.(jpe?g|png|webp|gif|bmp)$/i;

/** Import magazines…: the files picked read into a review list, then planned and run. */
function ImportDialog({ paths }) {
  const [items, setItems] = useState(null);
  const [progress, setProgress] = useState("");
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let gone = false;
    (async () => {
      try {
        const { job } = await api("magazine_items", { paths });
        const done = await followJob(job, "Reading the magazines", (j) => {
          const p = j.progress || {};
          if (!gone && p.items) setProgress(`${p.item} of ${p.items}: ${p.name || ""}`);
        });
        if (gone) return;
        if (done.error) throw new Error(done.error);
        setItems(done.result.items);
      } catch (e) {
        if (!gone) setError(err(e));
      }
    })();
    return () => { gone = true; };
  }, []);
  const set = (path, patch) => setItems((all) => all.map((it) => (it.path === path ? { ...it, ...patch } : it)));
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("magazine_plan", { items, ...howArgs(how) }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="mag-import-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(again); }} busy=${plan.short || !plan.ops} submitLabel="Import"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each goes into its magazine's folder in the library. ${how !== "move" ? "Your files stay where they are." : "The files are moved; Undo puts them back."}</p>
      <${More} id="mag-import-more"><p>That's <code>Magazines/${"<magazine>"}/</code>. Each issue is named by its number and date, and listed in its magazine.json.</p><//>
    <//>`;
  }
  const included = (items || []).filter((i) => i.include).length;
  // a stray click or Esc doesn't lose the reading and what was put right (review M35)
  return html`<${Dialog} title="Import magazines" id="mag-import-dialog" wide=${true} sticky=${!items || items.length > 0} onSubmit=${preview} busy=${busy || !items || !included} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!items ? html`<p class="muted" id="mag-reading">Reading the files${progress ? html` (${progress})` : null}…</p>` : !items.length ? html`<p class="muted" id="mag-none-found">No PDF, CBZ or CBR files there.</p>` : html`
      <ul class="ls-list pc-items mag-items" id="mag-items">${items.map((it) => html`<li key=${it.path} class="pc-item" data-name=${it.name} data-include=${it.include ? "yes" : "no"}>
        <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Import ${it.name}`} checked=${it.include} onChange=${(e) => set(it.path, { include: e.target.checked })} /></label>
        <div class="pc-item-main">
          <span class="pc-item-name" title=${it.path}>${it.name} <span class="muted">${size(it.size)}</span>${it.have ? html` <span class="pc-badge">Have it</span>` : null}</span>
          <span class="pc-item-edit">
            <input type="text" class="compact mag-item-title" aria-label="Magazine" placeholder="Magazine" value=${it.title} disabled=${!it.include} onInput=${(e) => set(it.path, { title: e.target.value })} />
            <input type="text" class="compact mag-item-number" aria-label="Number" placeholder="Number" value=${it.number || ""} disabled=${!it.include} onInput=${(e) => set(it.path, { number: e.target.value || null })} />
            <input type="text" class="compact mag-item-date" aria-label="Date (YYYY-MM)" placeholder="YYYY-MM" value=${it.date || ""} disabled=${!it.include} onInput=${(e) => set(it.path, { date: e.target.value || null })} />
          </span>
        </div></li>`)}</ul>
      <${HowControl} id="mag-import-mode" value=${how} onChange=${setHow} />
      <p class="muted"><b>${plural(included, "issue", "issues")}</b> to import. The magazine, number and date come from each file's name: put them right before the preview.</p>`}
  <//>`;
}

/** Read the text of a magazine's issues (`title`), or every magazine's, that wasn't read
 *  since their files changed (`again`: all of them), and find the games in it (D145). */
export async function readMagazines(title = null, again = false) {
  try {
    const { job } = await api("magazine_read", { title, again });
    const done = await followJob(job, "Reading magazines' text");
    if (done.error) throw new Error(done.error);
    const r = done.result;
    if (!r.issues) {
      toast("Every issue's text was read already.", 5000);
      return r;
    }
    const by = r.recognised ? ` (${r.recognised} by text recognition)` : "";
    const left = r.unread ? ` ${plural(r.unread, "page", "pages")} couldn't be read: ${r.notes?.[0] || ""}` : "";
    toast(`Read ${plural(r.issues, "issue", "issues")}, ${plural(r.pages, "page", "pages")}${by}: ${r.games ? `${plural(r.games, "game", "games")} found` : "no games found"}.${left}`, r.unread ? 15000 : 8000);
    ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
    return r;
  } catch (e) {
    toast(`Couldn't read them: ${err(e)}`, 8000);
    return null;
  }
}

/** The games in an issue (D145): found in its text, ticked when sure; ticks changed and
 *  games added by hand, each with the page it's on. */
function IssueGames({ m, issue, off, onChange }) {
  const [q, setQ] = useState("");
  const [page, setPage] = useState("");
  const [found, setFound] = useState([]);
  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return; }
    let gone = false;
    const t = setTimeout(() => api("browse_query", { query: { search: q, all: true, one_per_title: true, limit: 8 } })
      .then((r) => { if (!gone) setFound(r.items || []); }).catch(() => { if (!gone) setFound([]); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q]);
  const set = async (key, ticked, pg = null) => {
    try {
      await api("magazine_game", { title: m.title, file: issue.file, key, ticked, page: pg });
      onChange();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const games = issue.games || [];
  const name = issue.number ? `No. ${issue.number}` : issue.title;
  return html`<section class="ls-section" id="mag-games" data-file=${issue.file}><h2>Games in ${name}</h2>
    <p class="muted">${!issue.text ? "Its text isn't read yet: the games in it are found once it is." : !games.length ? "No game's title is in its text." : "Found in its text. The ticked ones are listed on their games' pages; a tick you change stays when the issue is read again."}</p>
    ${games.length ? html`<ul class="ls-list mag-game-list">${games.map((g) => html`<li key=${g.key} data-key=${g.key} data-ticked=${g.ticked ? "yes" : "no"}>
      <label class="check-row"><input type="checkbox" checked=${g.ticked} disabled=${off} aria-label=${`${g.title} is in it`} onChange=${(e) => set(g.key, e.target.checked)} />${" "}<span><b>${g.title}</b>${" "}<span class="muted">${g.platform_name}</span></span></label>
      <span class="mag-game-pages">${g.pages.map((p) => html`<button type="button" class="linkish" key=${p} data-page=${p} disabled=${!issue.doc} title=${`Read page ${p}`} onClick=${() => viewDoc(issue.doc, { page: p, find: g.title })}>p. ${p}</button>`)}</span>
    </li>`)}</ul>` : null}
    <div class="pick-row" id="mag-game-add">
      <input type="search" class="compact" id="mag-game-search" aria-label="A game to add: its title" placeholder="Add a game: its title" value=${q} disabled=${off} onInput=${(e) => setQ(e.target.value)} />
      <input type="number" class="compact mag-game-page" id="mag-game-page" min="1" aria-label="The page it's on" placeholder="Page" value=${page} disabled=${off} onInput=${(e) => setPage(e.target.value)} />
    </div>
    ${found.length ? html`<ul class="ls-list" id="mag-game-found">${found.map((x) => html`<li key=${x.key} data-key=${x.key}><span>${x.title}${" "}<span class="muted">${x.platform_name}</span></span>
      <button type="button" class="ghost compact" data-action="add-game" disabled=${off} onClick=${async () => { await set(x.key, true, Number(page) || null); setQ(""); setPage(""); }}>Add</button></li>`)}</ul>` : null}
  </section>`;
}

/** Import magazines… from a folder (or `files`, picked one by one, or `given`, dropped). */
export async function importMagazines(files = false, given = null) {
  const lib = ctx.platform.library;
  const paths = given || (files ? (await lib.pickFiles("Magazine issues (PDF, CBZ, CBR)")) || [] : [await lib.pickFolder("A folder of magazines")].filter(Boolean));
  if (paths.length) ui.set({ dialog: { type: "mag-import", paths } });
}

/** The lists the magazines are checked against, each with Remove…, and Import a list…. */
function ListsDialog() {
  const [lists, setLists] = useState(null);
  const [error, setError] = useState("");
  const off = !!ui.get().library?.read_only;
  const load = () => api("magazines").then((d) => setLists(d.lists || [])).catch((e) => setError(err(e)));
  useEffect(() => { load(); }, []);
  const add = async () => {
    await addList();
    load();
  };
  return html`<${Dialog} title="Lists" id="mag-lists" onSubmit=${close} submitLabel="Done" cancelLabel=${null} error=${error}>
    <p class="muted">What the magazines are checked against: a CSV of magazine, number and date, or a DAT whose games are issues.</p>
    ${!lists ? html`<p class="muted">Reading…</p>` : lists.length ? html`<ul class="ls-list" id="mag-list-names">${lists.map((l) => html`<li key=${l.name} data-name=${l.name}><span>${l.name} <span class="muted">${plural(l.issues, "issue", "issues")}</span></span>
      <button type="button" class="ghost compact" disabled=${off} onClick=${() => removeList(l.name)}>Remove…</button></li>`)}</ul>` : html`<p class="muted" id="mag-lists-none">None yet.</p>`}
    <button type="button" class="ghost compact" id="mag-add-list-button" disabled=${off || !isDesktop()} onClick=${add}>${Icon.audit(14)} Import a list…</button>
    <${More} id="mag-lists-more"><p>They're kept in <code>_library/dats/magazines</code>. A DAT's issues are matched by their files, else by magazine and number.</p><//>
  <//>`;
}

export function MagazineDialogs({ dialog }) {
  if (dialog.type === "mag-import") return html`<${ImportDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  if (dialog.type === "mag-lists") return html`<${ListsDialog} />`;
  return null;
}

/** Import a list of a magazine's issues, to see which are missing. */
async function addList() {
  const p = await ctx.platform.library.pickFile("A list of issues (CSV of magazine, number, date; or a DAT)", ["csv", "txt", "dat", "xml", "zip"]);
  if (!p) return;
  try {
    const r = await api("magazine_list_import", { path: p });
    toast(`Imported the list ${r.name}: ${plural(r.issues, "issue", "issues")}.`, 6000);
    again();
  } catch (e) {
    toast(err(e), 8000);
  }
}

const removeList = (name) => confirmDialog({ title: `Remove the list ${name}?`, text: "Its issues are no longer looked for. The magazines you have stay.", button: "Remove it", run: async () => { await api("magazine_list_remove", { name }); again(); } });

/** Save a list of the issues missing from a magazine (`title`), or from every one. */
async function saveMissing(title = null) {
  try {
    const r = await api("magazine_missing", { title });
    toast(`Saved ${plural(r.issues, "missing issue", "missing issues")} as ${r.rel || r.path}.`, 8000);
  } catch (e) {
    toast(err(e), 8000);
  }
}

// ------------------------------------------------------------ the page (D154)

const num = (n) => { const x = parseFloat(String(n ?? "").replace(/[^\d.]/g, "")); return Number.isNaN(x) ? Infinity : x; };
const SORTS = [
  ["number", "Magazine and number", (a, b) => byName(a.mag, b.mag) || num(a.number) - num(b.number) || byName(a.date, b.date) || byName(a.title, b.title)],
  ["date", "Date", (a, b) => (a.date ? 0 : 1) - (b.date ? 0 : 1) || byName(a.date, b.date) || byName(a.mag, b.mag)],
  ["added", "Added", (a, b) => byName(b.added, a.added)],
  ["size", "Size", (a, b) => (b.size || 0) - (a.size || 0)],
];
const GROUPS = [["", "None"], ["magazine", "Magazine"], ["publisher", "Publisher"], ["tag", "Tag"], ["year", "Year"], ["decade", "Decade"]];
const TEXT = { read: "Read", partly: "Partly read", unread: "Not read yet" };
const HAS = { games: "Games found in it", cover: "A cover chosen" };
const COLUMNS = [
  ["magazine", "Magazine", (i) => i.mag, "number"],
  ["number", "Number", (i) => i.number || "", null],
  ["date", "Date", (i) => i.date || "", "date"],
  ["publisher", "Publisher", (i) => i.publisher || "", null],
  ["tags", "Tags", (i) => (i.tags || []).join(", "), null],
  ["games", "Games", (i) => (i.missing ? "" : String((i.games || []).filter((g) => g.ticked).length || "")), null],
  ["text", "Text", (i) => (i.missing ? "Missing" : TEXT[textOf(i)]), null],
  ["size", "Size", (i) => (i.size ? size(i.size) : ""), "size"],
];
const COLS = "minmax(140px, 2.4fr) 70px 84px minmax(80px, 1.4fr) minmax(80px, 1.4fr) 56px 96px 72px";
const textOf = (i) => (!i.text ? "unread" : i.text.unread ? "partly" : "read");
const ref = (i) => ({ title: i.mag, file: i.file });

/** One issue in the panel: its cover, number, date and tags changed here, its text and the games in it. */
function IssuePanel({ i, off, tags, actions, stamp, reading, read }) {
  const edit = async (patch) => {
    try {
      await api("magazine_edit", { title: i.mag, file: i.file, patch });
      again();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const tag = (patch) => api("magazine_issues_edit", { items: [ref(i)], patch }).then(again, (e) => toast(err(e), 8000));
  if (i.missing) {
    return html`<div class="shelf-panel" id="mag-issue" data-missing="1">
      <span class="lib-panel-cover lib-cover-none" style=${`background:${systemColour(i.mag)}`} aria-hidden="true">${i.number || "?"}</span>
      <h2>${i.title}</h2>
      <p class="muted">You don't have this issue: a list of ${i.mag}'s issues names it.</p>
      <div class="lib-panel-actions"><button type="button" class="ghost compact" id="mag-save-missing-one" disabled=${off} onClick=${() => saveMissing(i.mag)}>${Icon.download(14)} Save a list of what's missing</button></div>
    </div>`;
  }
  const t = i.text;
  return html`<div class="shelf-panel" id="mag-issue" data-file=${i.file}>
    <${Pic} srcs=${[i.gone ? null : `${libraryUrl(`~cover/${i.path}`)}?v=${stamp}`]} none=${{ text: i.number || "?", colour: systemColour(i.mag) }} cls="lib-panel-cover" />
    <h2 id="mag-issue-title">${i.title}</h2>
    <p class="muted">${[i.publisher, size(i.size), i.gone ? "its file is gone" : null].filter(Boolean).join(" · ")}</p>
    <${PanelActions} page="mag" actions=${actions} />
    <${EditBox} id="mag-issue-number" caption="Number" label="Number" value=${i.number || ""} off=${off} onSave=${(v) => edit({ number: v })} />
    <${EditBox} id="mag-issue-date" caption="Date" label="Date (YYYY-MM)" placeholder="YYYY-MM" value=${i.date || ""} off=${off} onSave=${(v) => edit({ date: v })} />
    <h3>Tags</h3>
    <${TagEdit} id="mag-tags" tags=${i.tags} all=${tags} off=${off} onAdd=${(x) => tag({ tag_add: x })} onRemove=${(x) => tag({ tag_remove: x })} />
    <p class="muted" id="mag-text" data-state=${textOf(i)}>${!t ? "Its text isn't read yet." : t.unread ? `${plural(t.pages - t.unread, "page", "pages")} of ${t.pages} read.` : "Its text is read: search the Documents page for it."}${" "}
      ${!t || t.unread ? html`<button type="button" class="linkish" id="mag-read" disabled=${off || reading} onClick=${() => read(i.mag)}>${reading ? "Reading…" : "Read its magazine's text"}</button>` : null}</p>
    <${IssueGames} m=${{ title: i.mag }} issue=${i} off=${off} onChange=${again} />
    ${isDesktop() ? html`<p class="muted">Drop a picture on the page to make it the cover.</p>` : null}
  </div>`;
}

export function Magazines() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("mags", { sort: "number", group: "magazine" });
  const arg = useRouteArg("magazines");
  const [stamp, setStamp] = useState(0);
  const [reading, setReading] = useState(false);
  const [focus, setFocus] = useState(null);
  useEffect(() => {
    if (!s.library) return;
    api("magazines").then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  // `#/magazines/<title>`: that magazine's issues
  useEffect(() => { if (arg) set({ f: { ...(st.f || {}), magazine: [arg] } }); }, [arg]);
  const mags = data?.magazines || [];
  const items = useMemo(() => {
    const out = [];
    for (const m of mags) {
      for (const i of m.issues) out.push({ ...i, id: `${m.title}/${i.file}`, mag: m.title, publisher: m.publisher });
      if (st.missing) for (const x of m.missing || []) out.push({ ...x, id: `missing/${m.title}/${x.title}`, mag: m.title, publisher: m.publisher, missing: true, tags: [], games: [] });
    }
    return out;
  }, [data, st.missing]);
  const names = useMemo(() => {
    const m = new Map();
    for (const i of items) for (const g of i.games || []) m.set(g.platform, g.platform_name);
    return m;
  }, [items]);
  const tags = useMemo(() => [...new Set(items.flatMap((i) => i.tags || []))].sort(byName), [items]);
  const cover = async (i, picture) => {
    try {
      await api("magazine_cover", { title: i.mag, file: i.file, picture });
      setStamp((n) => n + 1);
      toast(`Set the cover of ${i.title}.`, 5000);
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  // a picture dropped while an issue is picked is its cover; anything else is imported, as magazines (D78)
  useDroppedFiles((paths) => {
    const pic = paths.find((p) => PICTURE.test(p));
    if (!(pic && focus && !focus.missing && !off)) return "mags";
    cover(focus, pic);
    return true;
  });
  const chooseCover = async (i) => {
    const p = await ctx.platform.library.pickFile(`A cover for ${i.title}`, ["jpg", "jpeg", "png", "webp"]);
    if (p) cover(i, p);
  };
  const read = async (title, again = false) => {
    setReading(true);
    await readMagazines(title, again);
    setReading(false);
  };
  const editMag = (title, patch) => api("magazine_edit", { title, file: null, patch }).then(again);
  const fields = [
    { id: "magazine", name: "Magazine", of: (i) => [i.mag], long: true, groupOrder: "name" },
    { id: "publisher", name: "Publisher", of: (i) => [i.publisher], long: true, none: "No publisher given" },
    { id: "tag", name: "Tag", of: (i) => i.tags, long: true, none: "No tags" },
    { id: "year", name: "Year", of: (i) => [i.date?.slice(0, 4)], order: "newest", none: "No date" },
    { id: "decade", name: "Decade", hidden: true, of: (i) => [i.date ? `${i.date.slice(0, 3)}0` : null], label: (d) => `${d}s`, none: "No date" },
    { id: "system", name: "System", of: (i) => (i.games || []).filter((g) => g.ticked).map((g) => g.platform), label: (p) => names.get(p) || p },
    { id: "text", name: "Text", of: (i) => [i.missing ? null : textOf(i)], label: (x) => TEXT[x] || x },
    { id: "has", name: "Has", of: (i) => [(i.games || []).some((g) => g.ticked) ? "games" : null, i.cover ? "cover" : null], label: (x) => HAS[x] || x },
    ...(st.missing ? [{ id: "is", name: "Is", of: (i) => [i.missing ? "missing" : "have"], label: (x) => (x === "missing" ? "Missing" : "In the library") }] : []),
  ];
  const actions = (list) => {
    const real = list.filter((i) => !i.missing);
    if (!real.length) return [];
    const one = list.length === 1 ? real[0] : null;
    const refs = real.map(ref);
    const theirTags = [...new Set(real.flatMap((i) => i.tags || []))].sort(byName);
    const titles = mags.filter((m) => !m.none).map((m) => m.title);
    return [
      { id: "read", label: "Read", icon: "book", one: true, disabled: !one?.doc, run: () => { if (one?.doc) viewDoc(one.doc); } },
      { id: "move", label: "Move to another magazine…", icon: "move", disabled: off, run: () => askValue({
        title: real.length === 1 ? `Move ${real[0].title}` : `Move ${plural(real.length, "issue", "issues")}`, label: "The magazine they go to", list: titles, button: "Preview",
        note: "Their files are moved into its folder and named for it; a magazine that isn't there yet is made. Undo puts them back.",
        run: async (to) => ui.set({ dialog: { type: "organise", plan: await api("magazine_move", { items: refs, to }) } }),
      }) },
      { id: "tag-add", label: "Add a tag…", icon: "pin", disabled: off, run: () => askValue({ title: "Add a tag", label: "The tag", list: tags, button: "Add it", note: "A tag is a label of your own, such as Favourites or Reviews.", run: (x) => api("magazine_issues_edit", { items: refs, patch: { tag_add: x } }).then(again) }) },
      { id: "tag-remove", label: "Remove a tag…", icon: "close", disabled: off || !theirTags.length, run: () => askValue({ title: "Remove a tag", label: "The tag", value: theirTags[0], options: theirTags.map((t) => [t, t]), button: "Remove it", run: (x) => api("magazine_issues_edit", { items: refs, patch: { tag_remove: x } }).then(again) }) },
      { id: "cover", label: "Choose a cover…", icon: "image", one: true, disabled: off || !isDesktop(), run: () => one && chooseCover(one) },
      { id: "show", label: "Show in folder", icon: "folder", one: true, disabled: !isDesktop() || one?.gone, run: () => one && reveal(`${s.library.path}/${one.path}`) },
    ];
  };
  const groupMenu = (g) => {
    if (st.group !== "magazine" || !g.value) return null;
    const m = mags.find((x) => x.title === g.value);
    if (!m) return null;
    return [
      { id: "mag-rename", label: "Rename…", icon: "edit", disabled: off || !!m.none, run: () => askValue({ title: `Rename ${m.title}`, label: "Its name", value: m.title, button: "Rename it", run: async (x) => {
        await editMag(m.title, { title: x });
        if ((st.f?.magazine || []).includes(m.title)) set({ f: { ...st.f, magazine: st.f.magazine.map((t) => (t === m.title ? x : t)) } });
      } }) },
      { id: "mag-publisher", label: "Publisher…", icon: "edit", disabled: off || !!m.none, run: () => askValue({ title: `${m.title}'s publisher`, label: "Its publisher", value: m.publisher || "", empty: true, button: "Save", run: (x) => editMag(m.title, { publisher: x }) }) },
      { sep: true },
      { id: "mag-read", label: m.read && !m.unread ? "Read its text again" : "Read its text", icon: "search", disabled: off || reading || !m.issues.length, run: () => read(m.title, !m.unread) },
      { id: "mag-save-missing", label: "Save a list of what's missing", icon: "download", disabled: off || !m.missing?.length, run: () => saveMissing(m.title) },
      { id: "mag-show", label: "Show its folder", icon: "folder", disabled: !isDesktop() || !m.issues[0], run: () => reveal(`${s.library.path}/${m.issues[0].path}`) },
    ];
  };
  const missingAll = mags.reduce((n, x) => n + (x.missing?.length || 0), 0);
  // with issues to show, Import magazines… is under ⋯, as the Library's Import… is
  const more = () => [
    ...(items.length ? [{ id: "mag-import-more", label: "Import magazines…", icon: "inbox", disabled: off || !isDesktop(), run: () => importFrom("import-mags", () => importMagazines()) }] : []),
    { id: "mag-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-mags", () => importMagazines(true)) },
    { id: "mag-lists", label: "Lists…", icon: "audit", run: () => ui.set({ dialog: { type: "mag-lists" } }) },
    { id: "mag-save-missing", label: "Save a list of what's missing", icon: "download", disabled: off || !missingAll, run: () => saveMissing() },
    { sep: true },
    { id: "mag-read-all", label: "Read every magazine's text", icon: "search", disabled: off || reading || !mags.some((x) => x.unread), run: () => read(null) },
  ];
  const primary = html`<button type="button" class="primary compact" id="mag-import" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-mags", () => importMagazines())}>${Icon.inbox(15)} Import magazines…</button>`;
  if (!s.library) return html`<${CommandBar} id="mags-head" title="Magazines" /><div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const switches = html`<label class="check-row" title="The issues the lists name that you don't have, as faded tiles"><input type="checkbox" id="mag-missing-too" checked=${!!st.missing} onChange=${(e) => set({ missing: e.target.checked })} /> Missing issues too${missingAll ? html` <span class="lib-count">${missingAll.toLocaleString()}</span>` : null}</label>
    ${mags.some((x) => x.unread) ? html`<p class="muted shelf-note" id="mag-unread">${plural(mags.reduce((n, x) => n + x.unread, 0), "issue's", "issues'")} text isn't read yet.${" "}
      <button type="button" class="linkish" id="mag-read-all-now" disabled=${off || reading} onClick=${() => read(null)}>${reading ? "Reading…" : "Read it"}</button>${!data?.reader ? html`${" "}<span id="mag-no-reader">Scanned pages need Tesseract (Settings › Tools).</span>` : null}</p>` : null}`;
  const empty = html`<div class="library-settings" id="mag-empty">
    <p class="page-lead">Your magazines, each with its issues as PDF, CBZ or CBR files, their covers, and the issues you're missing.</p>
    <p class="muted">Import magazines… reads a folder of them, takes each one's magazine, number and date from its name, and imports them into the library's Magazines folder. It's on the <a href=${routeHash("import")}>Import</a> page, where files dropped on the window go too. Lists… in ⋯ imports a list of a magazine's issues, to see which you don't have.</p>
  </div>`;
  const label = (i) => (i.number ? `No. ${i.number}` : i.date || i.title);
  return html`<${Shelf} page="mags" noun=${["issue", "issues"]} title="Magazines" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} ratio="3 / 4" columns=${COLUMNS} cols=${COLS}
    text=${(i) => [i.title, i.mag, i.publisher, i.number, i.date, ...(i.tags || []), ...(i.games || []).filter((g) => g.ticked).map((g) => g.title)].join(" ")}
    tile=${(i) => ({ pics: [i.missing || i.gone ? null : `${libraryUrl(`~cover/${i.path}`)}?v=${stamp}`], none: { text: i.number || "?", colour: systemColour(i.mag) }, badge: st.group === "magazine" ? null : { text: i.mag, colour: systemColour(i.mag) },
      title: st.group === "magazine" ? label(i) : `${i.mag} ${label(i)}`, meta: i.missing ? `Missing${i.date ? ` · ${i.date}` : ""}` : [i.number ? i.date : null, size(i.size)].filter(Boolean).join(" · "), missing: !!i.missing || i.gone })}
    line=${(i) => ({ pics: [i.missing || i.gone ? null : `${libraryUrl(`~cover/${i.path}`)}?v=${stamp}`], none: { text: i.number || "?", colour: systemColour(i.mag) }, title: i.title,
      meta: i.missing ? "Missing" : [i.publisher, size(i.size), (i.games || []).some((g) => g.ticked) ? plural(i.games.filter((g) => g.ticked).length, "game", "games") : null, ...(i.tags || [])].filter(Boolean).join(" · "), missing: !!i.missing || i.gone })}
    actions=${actions} onOpen=${(i) => { if (i.doc) viewDoc(i.doc); }} groupMenu=${groupMenu} switches=${switches} primary=${mags.length ? null : primary} more=${more} moreTitle="Import files, lists, what's missing, tile size" empty=${empty}
    onPicked=${setFocus}
    panel=${(i) => html`<${IssuePanel} i=${i} off=${off} tags=${tags} stamp=${stamp} reading=${reading} read=${read} actions=${actions([i]).filter((a) => ["read", "cover", "show"].includes(a.id))} />`} />`;
}
