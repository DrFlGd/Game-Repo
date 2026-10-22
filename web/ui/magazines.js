// Magazines (docs/PLAN.md, "Phase 8 design", D68): each magazine in the library's
// Magazines/<Title>/ folder with its issues, their covers (the first page, or a picture
// chosen for it), and how they stand against the lists kept to audit them (a CSV of
// magazine, number and date, or a DAT of issues). Import magazines… reads a folder into a
// review list (each file's magazine, number and date worked out from its name, put right
// here) and brings them in as a change with a preview and Undo. An issue opens in the
// document viewer: PDFs as documents are, CBZ and CBR a page at a time. The magazines are
// in the ribbon beside the menu (`#/magazines/<title>`, D69). Their text is read page by
// page (D145: a PDF's own, else text recognition), searched on the Documents page, and the
// games in it are listed with each issue, ticked when sure, and on the games' pages.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, EditBox } from "./layout.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { importFrom } from "./importpage.js";
import { viewDoc, useDroppedFiles } from "./documents.js";
import { Icon } from "./icons.js";
import { useSidePanel, useRouteArg, argHash } from "./panels.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const PICTURE = /\.(jpe?g|png|webp|gif|bmp)$/i;

/** An issue's cover, or its number on the magazine's colour. */
function Cover({ issue, stamp }) {
  const [broken, setBroken] = useState(false);
  useEffect(() => setBroken(false), [stamp]);
  if (broken || issue.gone) return html`<span class="mag-cover mag-cover-none" aria-hidden="true">${issue.number || "?"}</span>`;
  return html`<img class="mag-cover" src=${libraryUrl(`~cover/${issue.path}`) + `?v=${stamp}`} alt="" loading="lazy" onError=${() => setBroken(true)} />`;
}

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
      <p class="muted">Each goes into its magazine's folder in the library, <code>Magazines/${"<magazine>"}/</code>, named by its number and date, and listed in its magazine.json. ${how !== "move" ? "Your files stay where they are." : "The files are moved; Undo puts them back."}</p>
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

export function MagazineDialogs({ dialog }) {
  if (dialog.type === "mag-import") return html`<${ImportDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  return null;
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

export function Magazines() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const pick = useRouteArg("magazines");
  const [stamp, setStamp] = useState(0);
  const [focus, setFocus] = useState(null);
  const [open, setOpen] = useState(null);
  const [reading, setReading] = useState(false);
  useEffect(() => {
    if (!s.library) return;
    api("magazines").then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  const mags = data?.magazines || [];
  const m = mags.find((x) => x.title === pick) || mags[0];
  useEffect(() => { setFocus(null); setOpen(null); }, [m?.title]);
  useSidePanel(mags.length ? {
    id: "mag-panel", route: "magazines", title: "Magazines", icon: "stack", count: mags.filter((x) => !x.none).length, current: m?.title,
    items: mags.map((x) => ({ id: x.title, label: x.title, sub: [x.none ? "none yet" : plural(x.issues.length, "issue", "issues"), x.missing?.length ? `${x.missing.length} missing` : null].filter(Boolean).join(" · "), href: argHash("magazines", x.title) })),
  } : null);
  const reload = async () => setData(await api("magazines"));
  const cover = async (issue, picture) => {
    try {
      await api("magazine_cover", { title: m.title, file: issue.file, picture });
      setStamp((n) => n + 1);
      toast(`Set the cover of ${issue.title}.`, 5000);
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  // a picture dropped on an issue is its cover; anything else is imported, as magazines (D78)
  useDroppedFiles((paths) => {
    const pic = paths.find((p) => PICTURE.test(p));
    if (!(pic && focus && !off)) return "mags";
    cover(focus, pic);
    return true;
  });
  const chooseCover = async (issue) => {
    const p = await ctx.platform.library.pickFile(`A cover for ${issue.title}`, ["jpg", "jpeg", "png", "webp"]);
    if (p) cover(issue, p);
  };
  const edit = async (issue, patch) => {
    try {
      await api("magazine_edit", { title: m.title, file: issue?.file ?? null, patch });
      await reload();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const addList = async () => {
    const p = await ctx.platform.library.pickFile("A list of issues (CSV of magazine, number, date; or a DAT)", ["csv", "txt", "dat", "xml", "zip"]);
    if (!p) return;
    try {
      const r = await api("magazine_list_import", { path: p });
      toast(`Imported the list ${r.name}: ${plural(r.issues, "issue", "issues")}.`, 6000);
      await reload();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const saveMissing = async (title = null) => {
    try {
      const r = await api("magazine_missing", { title });
      toast(`Saved ${plural(r.issues, "missing issue", "missing issues")} as ${r.rel || r.path}.`, 8000);
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const read = async (title, again = false) => {
    setReading(true);
    await readMagazines(title, again);
    setReading(false);
  };
  const removeList = (name) => confirmDialog({ title: `Remove the list ${name}?`, text: "Its issues are no longer looked for. The magazines you have stay.", button: "Remove it", run: async () => { await api("magazine_list_remove", { name }); await reload(); } });
  const missingAll = mags.reduce((n, x) => n + (x.missing?.length || 0), 0);
  const more = () => [
    { id: "mag-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-mags", () => importMagazines(true)) },
    { id: "mag-add-list", label: "Import a list…", icon: "audit", disabled: off || !isDesktop(), run: addList },
    { id: "mag-save-missing", label: "Save a list of what's missing", icon: "download", disabled: off || !missingAll, run: () => saveMissing() },
    { sep: true },
    { id: "mag-read-all", label: "Read every magazine's text", icon: "search", disabled: off || reading || !mags.some((x) => x.unread), run: () => read(null) },
    { id: "mag-read-again", label: "Read this magazine's text again", icon: "search", disabled: off || reading || !m?.read, run: () => read(m.title, true) },
  ];
  const head = html`<${CommandBar} id="mag-head" title="Magazines" count=${data ? plural(mags.filter((x) => !x.none).length, "magazine", "magazines") : null}
    primary=${html`<button type="button" class="primary compact" id="mag-import" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-mags", () => importMagazines())}>${Icon.inbox(15)} Import magazines…</button>`}
    more=${{ id: "mag-menu", title: "Import files or a list, save what's missing", items: more }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${error}</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  const lists = data.lists || [];
  return html`${head}<div class="pages magazines-page" id="magazines-page">
    ${!mags.length ? html`<div class="library-settings" id="mag-empty">
      <p class="page-lead">Your magazines, each with its issues as PDF, CBZ or CBR files, their covers, and the issues you're missing.</p>
      <p class="muted">Import magazines… reads a folder of them, takes each one's magazine, number and date from its name, and imports them into the library's Magazines folder. It's on the <a href=${routeHash("import")}>Import</a> page, where files dropped on the window go too. Import a list… of a magazine's issues to see which you don't have.</p>
    </div>` : html`<div class="mag-layout">
      <div class="collection-main" id="mag-main" data-title=${m.title}>
        <div class="collection-top">
          <h2 id="mag-title">${m.title}</h2>
          <p class="muted" id="mag-totals">${[m.publisher, plural(m.issues.length, "issue", "issues"), m.listed ? `${m.listed - (m.missing?.length || 0)} of the ${m.listed} listed` : null].filter(Boolean).join(" · ")}</p>
          ${m.read + m.unread ? html`<p class="muted" id="mag-text" data-read=${m.read} data-unread=${m.unread}>${m.unread ? (m.read ? `The text of ${m.read} of its ${m.read + m.unread} issues is read.` : "Its issues' text isn't read yet.") : "Every issue's text is read: search the Documents page for it."}${" "}
            ${m.unread ? html`<button type="button" class="linkish" id="mag-read" disabled=${off || reading} onClick=${() => read(m.title)}>${reading ? "Reading…" : m.read ? "Read the rest" : "Read their text"}</button>` : null}
            ${m.unread && !data.reader ? html`${" "}<span id="mag-no-reader">Scanned pages need Tesseract (Settings › Tools).</span>` : null}</p>` : null}
          ${!m.none ? html`<div class="pick-row">
            <${EditBox} id="mag-publisher" label="Publisher" title="Its publisher: click to change it" placeholder="Publisher" value=${m.publisher || ""} off=${off} onSave=${(v) => edit(null, { publisher: v })} />
            ${isDesktop() && m.folder ? html`<button type="button" class="ghost compact" onClick=${() => reveal(m.issues[0] ? `${s.library.path}/${m.issues[0].path}` : s.library.path)}>${Icon.folder(14)} Show its folder</button>` : null}
          </div>` : null}
        </div>
        ${m.issues.length ? html`<ul class="mag-issues" id="mag-issues">${m.issues.map((i) => html`<li key=${i.file} class=${focus?.file === i.file ? "mag-issue focused" : "mag-issue"} data-file=${i.file} data-number=${i.number || ""}>
          <button type="button" class="mag-open" title=${i.doc ? `Read ${i.title}` : i.title} disabled=${!i.doc} onClick=${() => { setFocus(i); viewDoc(i.doc); }} onFocus=${() => setFocus(i)}>
            <${Cover} issue=${i} stamp=${stamp} /><span class="mag-issue-title">${i.number ? `No. ${i.number}` : i.title}</span><small class="muted">${[i.date, size(i.size)].filter(Boolean).join(" · ")}</small></button>
          <span class="seg">
            <${EditBox} cls="mag-issue-date" label="Date (YYYY-MM)" title="Its date: click to change it" placeholder="YYYY-MM" value=${i.date || ""} off=${off} onSave=${(v) => edit(i, { date: v })} />
            <button type="button" class="ghost compact" data-action="cover" disabled=${off || !isDesktop()} title="Choose a picture for its cover (or drop one on the page)" onClick=${() => { setFocus(i); chooseCover(i); }}>${Icon.image(14)}</button>
            <button type="button" class="ghost compact" data-action="games" aria-pressed=${open === i.file ? "true" : "false"} title="The games in it" onClick=${() => { setFocus(i); setOpen(open === i.file ? null : i.file); }}>${(i.games || []).some((g) => g.ticked) ? plural(i.games.filter((g) => g.ticked).length, "game", "games") : "Games"}</button>
          </span></li>`)}</ul>` : html`<p class="muted">You have none of its issues.</p>`}
        ${open && m.issues.some((i) => i.file === open) ? html`<${IssueGames} m=${m} issue=${m.issues.find((i) => i.file === open)} off=${off} onChange=${reload} />` : null}
        ${m.missing?.length ? html`<section class="ls-section" id="mag-missing"><h2>Missing</h2>
          <p class="muted">The issues the lists name that you don't have.</p>
          <ul class="ls-list">${m.missing.map((x) => html`<li key=${x.title} data-number=${x.number || ""}>${x.title}</li>`)}</ul>
          <button type="button" class="ghost compact" id="mag-save-missing-one" disabled=${off} onClick=${() => saveMissing(m.title)}>${Icon.download(14)} Save a list of what's missing</button>
        </section>` : null}
      </div>
    </div>`}
    <section class="ls-section" id="mag-lists"><h2>Lists</h2>
      <p class="muted">What the magazines are checked against: a CSV of magazine, number and date, or a DAT whose games are issues (matched by their files, else by magazine and number). They're kept in _library/dats/magazines.</p>
      ${lists.length ? html`<ul class="ls-list">${lists.map((l) => html`<li key=${l.name} data-name=${l.name}><span>${l.name} <span class="muted">${plural(l.issues, "issue", "issues")}</span></span>
        <button type="button" class="ghost compact" disabled=${off} onClick=${() => removeList(l.name)}>Remove…</button></li>`)}</ul>` : html`<p class="muted">None yet.</p>`}
      <button type="button" class="ghost compact" id="mag-add-list-button" disabled=${off || !isDesktop()} onClick=${addList}>${Icon.audit(14)} Import a list…</button>
    </section>
  </div>`;
}
