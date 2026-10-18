// Metadata (docs/PLAN.md, "Phase 5 design", D30, D32–D35): a system's games as its DAT
// lists them, with what the library has for each (details, pictures, documents), kept in
// the game's folder under _library/games/. Scrape… asks the sources for a system's games
// in the library, every game in its DAT, or chosen ones, with its choices (sources and
// their order, details, pictures, regions, what to do with games that have something)
// kept with the library. Each source's answer is kept apart, and a kind of picture can
// be kept from every source that has one ("Phase 6b design", D46, D47). A game's panel
// shows what it has, scrapes it alone, picks from the candidates a scrape wasn't sure
// about and attaches documents. Scrape again… and Delete metadata… (D48) work on a
// system or a game. What an export carries (CarryChoices) is chosen in the Export dialog.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, SearchNote, MoreButton } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { identifyDialog } from "./pcgames.js";
import { attachDocs, viewDoc, sourceName, useDroppedFiles } from "./documents.js";
import { Icon } from "./icons.js";

const PAGE = 200;
const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const prefs = () => ctx.platform?.store?.prefs;

/** The lists: which of the DAT's games are shown. */
const SHOWS = [
  ["have", "In the library", "have"],
  ["all", "Every game", "all"],
  ["look", "Needs a look", "look"],
  ["none", "Nothing yet", null],
];
const SHOW_HELP = {
  have: "The games you have, with what the library holds for each.",
  all: "Every game in the system's DAT, yours or not: a scrape of these fills the library ahead of the games.",
  look: "Games a scrape found more than one possible match for, or none it was sure of: pick the right one.",
  none: "Games you have with no details or pictures yet.",
};
const PICTURES = { cover: "Box", back: "Back", box3d: "3D box", screenshot: "Screenshot", titlescreen: "Title screen", marquee: "Logo", fanart: "Fan art", physical: "Cartridge or disc", video: "Video" };
const EXISTING = [["fill", "Fill in what's missing"], ["replace", "Replace what's there"], ["skip", "Skip games that have anything"]];
const REGIONS = [["us", "USA"], ["eu", "Europe"], ["wor", "World"], ["jp", "Japan"], ["ss", "ScreenScraper's own"], ["fr", "France"], ["de", "Germany"], ["es", "Spain"], ["it", "Italy"], ["uk", "United Kingdom"], ["au", "Australia"], ["kr", "Korea"], ["br", "Brazil"]];

/** A picture of a game (its box first), or its initial. */
function Picture({ media, title, cls = "meta-cover", kind }) {
  const rel = kind ? media?.[kind] : media?.cover || media?.box3d || media?.screenshot || media?.titlescreen;
  const [broken, setBroken] = useState(false);
  if (rel && /\.(mp4|webm|mkv|avi)$/i.test(rel)) return html`<video class=${cls} src=${libraryUrl(rel)} controls preload="metadata"></video>`;
  return rel && !broken ? html`<img class=${cls} src=${libraryUrl(rel)} alt="" loading="lazy" onError=${() => setBroken(true)} />` : html`<span class=${`${cls} pc-cover-none`} aria-hidden="true">${(title || "?").slice(0, 1)}</span>`;
}

/** A list row's second line: its DAT name, then what the library has for it. */
function rowMeta(g) {
  const has = [g.details ? "details" : null, g.media.length ? plural(g.media.length, "picture", "pictures") : null, g.docs ? plural(g.docs, "document", "documents") : null, g.review ? "needs a look" : null].filter(Boolean);
  return [g.name !== g.title ? g.name : null, ...(has.length ? has : ["nothing yet"])].filter(Boolean).join(" · ");
}

// ------------------------------------------------------------ Scrape…

/** A list of choices that can be ticked and put in order (the sources). */
function Ordered({ id, items, value, onChange, note }) {
  const order = [...value.filter((v) => items.some((x) => x.id === v)), ...items.map((x) => x.id).filter((v) => !value.includes(v))];
  const on = new Set(value);
  const set = (ids, ticked) => onChange(ids.filter((v) => ticked.has(v)));
  const move = (i, by) => {
    const next = [...order];
    const [x] = next.splice(i, 1);
    next.splice(i + by, 0, x);
    set(next, on);
  };
  return html`<ul class="ls-list change-list" id=${id}>${order.map((v, i) => {
    const x = items.find((o) => o.id === v);
    return html`<li key=${v} data-source=${v} data-on=${on.has(v) ? "yes" : "no"}>
      <span class="change-label" title=${x.needs || null}><label class="check-row source-name"><input type="checkbox" aria-label=${`Ask ${x.name}`} checked=${on.has(v) && !x.needs} disabled=${!!x.needs} onChange=${(e) => { const t = new Set(on); e.target.checked ? t.add(v) : t.delete(v); set(order, t); }} />
        <b>${x.name}</b>${note ? html` <span class="muted">${note(x)}</span>` : null}${x.needs ? html` <span class="muted scrape-needs">· not set up</span>` : null}</label></span>
      <button type="button" class="ghost compact" aria-label=${`Ask ${x.name} sooner`} disabled=${i === 0} onClick=${() => move(i, -1)}>↑</button>
      <button type="button" class="ghost compact" aria-label=${`Ask ${x.name} later`} disabled=${i === order.length - 1} onClick=${() => move(i, 1)}>↓</button>
    </li>`;
  })}</ul>`;
}

/** Ticks for a set of choices: `items` [{ id, name }], `value` the ids ticked. */
function Ticks({ id, items, value, onChange }) {
  const on = new Set(value);
  return html`<div class="tick-grid" id=${id}>${items.map((x) => html`<label class="check-row" key=${x.id} data-id=${x.id}>
    <input type="checkbox" checked=${on.has(x.id)} onChange=${(e) => onChange(items.map((o) => o.id).filter((k) => (k === x.id ? e.target.checked : on.has(k))))} /> ${x.name}</label>`)}</div>`;
}

/** Scrape…: for `platform`'s games (or `games`, chosen ones), with the choices the last
 *  scrape kept. */
function ScrapeDialog({ platform = "", platformName, games = null, collection = "", gameName = "", have = 0, all = 0, again: anew = false }) {
  const [c, setC] = useState(null);
  const [o, setO] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    api("scrape_choices").then((x) => {
      setC(x);
      setO({ ...x.options, platform, scope: games ? "games" : collection ? "collection" : "have", games: games || [], collection, ...(anew ? { existing: "replace", fresh: true } : {}) });
    }).catch((e) => setError(err(e)));
  }, []);
  const set = (patch) => setO((v) => ({ ...v, ...patch }));
  const ready = (c?.sources || []).filter((x) => !x.needs).map((x) => x.id);
  const run = async () => {
    setBusy(true);
    setError("");
    try {
      // a source that isn't ready yet keeps its place for next time, but isn't asked
      const { job } = await api("scrape_run", { options: { ...o, sources: o.sources.filter((s) => ready.includes(s)) } });
      close();
      const done = await followJob(job, games || collection ? `Scraping ${gameName}` : `Scraping ${platformName}`);
      again();
      if (done.error) throw new Error(done.error);
      const r = done.result;
      const errors = r.errors.map((e) => `${e.name}: ${e.error}`).join(" ");
      const text = games && r.games === 1
        ? r.found ? `Scraped ${gameName}: ${plural(r.pictures, "picture", "pictures")}.` : r.look ? `${gameName} needs a look: the sources found more than one game it could be.` : `No source knew ${gameName}.`
        : `Scraped ${plural(r.games, "game", "games")}: ${r.found} found, ${r.look} need a look, ${r.none} not found${r.skipped ? `, ${r.skipped} skipped` : ""}; ${plural(r.pictures, "picture", "pictures")}.${r.stopped ? " Stopped." : ""}`;
      toast(`${text}${errors ? ` ${errors}` : ""}${r.sources_stopped ? " The sources stopped answering, so it stopped." : ""}`, 12000);
    } catch (e) {
      toast(`Couldn't scrape: ${err(e)}`, 10000);
    } finally {
      setBusy(false);
    }
  };
  const title = `Scrape ${games || collection ? gameName : platformName}${anew ? " again" : ""}`;
  if (!c || !o) return html`<${Dialog} title=${title} id="scrape-dialog" onSubmit=${close} submitLabel="OK" error=${error}><p class="muted">Reading the choices…</p><//>`;
  const asked = o.sources.filter((s) => ready.includes(s));
  const pictureKinds = c.media.filter((m) => m.id !== "manual");
  const regions = [...o.regions, ...REGIONS.map(([k]) => k).filter((k) => !o.regions.includes(k))];
  return html`<${Dialog} title=${title} id="scrape-dialog" wide=${true} onSubmit=${run} busy=${busy || !asked.length || (!o.fields.length && !o.media.length)} error=${error} submitLabel=${busy ? "Starting…" : "Scrape"}>
    ${games || collection ? null : html`<div class="field-block" role="radiogroup" aria-label="Which games">
      <span class="field-label">Which games</span>
      <label class="check-row"><input type="radio" name="scrape-scope" value="have" checked=${o.scope === "have"} onChange=${() => set({ scope: "have" })} /> The ${plural(have, "game", "games")} in the library</label>
      <label class="check-row"><input type="radio" name="scrape-scope" value="all" checked=${o.scope === "all"} onChange=${() => set({ scope: "all" })} /> Every game in its DAT (${all.toLocaleString()}), so the library has them all ahead of the games</label>
    </div>`}
    <div class="field-block"><span class="field-label">Sources, in the order they're asked</span>
      <${Ordered} id="scrape-sources" items=${c.sources} value=${o.sources} onChange=${(sources) => set({ sources })} note=${(x) => (x.hashes ? "by the ROMs' hashes" : "by title")} />
      ${ready.length && ready.length < c.sources.length ? html`<small class="muted" id="scrape-not-ready">${c.sources.filter((x) => x.needs).map((x) => x.name).join(", ")} ${c.sources.filter((x) => x.needs).length === 1 ? "needs" : "need"} setting up in <a href=${routeHash("settings")} onClick=${() => { close(); setTimeout(() => document.getElementById("settings-sources")?.scrollIntoView(), 200); }}>Settings › Sources</a> first.</small>` : null}
      ${!ready.length ? html`<p class="warn-note" id="scrape-no-source">No source is ready: set their accounts or keys in <a href=${routeHash("settings")} onClick=${() => { close(); setTimeout(() => document.getElementById("settings-sources")?.scrollIntoView(), 200); }}>Settings › Sources</a>, or get LaunchBox's database there.</p>` : null}
      <small class="muted">Each game is asked of the first source that knows it; a source asked by title is only believed when the title is the same.</small></div>
    <div class="field-block"><span class="field-label">Details</span>
      <${Ticks} id="scrape-fields" items=${c.fields} value=${o.fields} onChange=${(fields) => set({ fields })} /></div>
    <div class="field-block"><span class="field-label">Pictures, video and manual</span>
      <table class="kinds-table" id="scrape-media">
        <thead><tr><th>Kind</th><th>Scrape it</th><th id="scrape-variants" title="One of each kind is chosen and shown; the other sources' are kept beside it, to choose from on the game's page. Exports take only the chosen one.">Keep every source's</th></tr></thead>
        <tbody>${c.media.map((m) => {
          const on = o.media.includes(m.id);
          const every = (o.variants || []).includes(m.id);
          return html`<tr key=${m.id} data-id=${m.id}><th scope="row">${m.name}</th>
            <td><input type="checkbox" aria-label=${`Scrape ${m.name}`} checked=${on} onChange=${(e) => set({ media: e.target.checked ? [...o.media, m.id] : o.media.filter((k) => k !== m.id) })} /></td>
            <td>${m.id === "manual" ? null : html`<input type="checkbox" data-variant=${m.id} aria-label=${`Keep every source's ${m.name}`} disabled=${!on} checked=${on && every}
              onChange=${(e) => set({ variants: e.target.checked ? [...(o.variants || []), m.id] : (o.variants || []).filter((k) => k !== m.id) })} />`}</td></tr>`;
        })}</tbody>
      </table>
      <small class="muted">One of each kind is shown and exported; with <i>Keep every source's</i>, the other sources' are kept beside it, to choose from on the game's page. Manuals go with the game's documents, to search and read on the Documents page.</small></div>
    <details class="field-block scrape-more" id="scrape-more"><summary>Regions, language, and a source for a kind of picture</summary>
    <div class="field-block"><span class="field-label">Regions</span>
      <div class="tick-grid" id="scrape-regions">${regions.map((k) => html`<label class="check-row" key=${k} data-region=${k}><input type="checkbox" checked=${o.regions.includes(k)}
        onChange=${(e) => set({ regions: e.target.checked ? [...o.regions, k] : o.regions.filter((r) => r !== k) })} /> ${REGIONS.find(([v]) => v === k)?.[1] || k}</label>`)}</div>
      <small class="muted">A game's own region (from its DAT name) comes first, then these, in the order ticked: for its name, box and the rest.</small></div>
    <label class="field-inline"><span>Language</span>
      <select id="scrape-language" class="compact" value=${o.language} onChange=${(e) => set({ language: e.target.value })}>
        ${[["en", "English"], ["fr", "French"], ["de", "German"], ["es", "Spanish"], ["it", "Italian"], ["pt", "Portuguese"], ["ja", "Japanese"]].map(([k, l]) => html`<option key=${k} value=${k}>${l}</option>`)}
      </select></label>
    ${pictureKinds.some((m) => o.media.includes(m.id)) ? html`<div class="field-block" id="scrape-prefer"><span class="field-label">A kind of picture from a source of its own</span>
      <div class="prefer-grid">${pictureKinds.filter((m) => o.media.includes(m.id)).map((m) => html`<label class="field-inline" key=${m.id}><span>${m.name}</span>
        <select class="compact" data-kind=${m.id} value=${o.prefer[m.id] || ""} onChange=${(e) => set({ prefer: { ...o.prefer, [m.id]: e.target.value || undefined } })}>
          <option value="">The first that has one</option>
          ${c.sources.map((x) => html`<option key=${x.id} value=${x.id}>${x.name}</option>`)}
        </select></label>`)}</div></div>` : null}
    </details>
    <div class="field-block" role="radiogroup" aria-label="Games that have something already">
      <span class="field-label">Games that have something already</span>
      ${EXISTING.map(([k, l]) => html`<label class="check-row" key=${k}><input type="radio" name="scrape-existing" value=${k} checked=${o.existing === k} onChange=${() => set({ existing: k })} /> ${l}</label>`)}
      <small class="muted">What you've written yourself, what you chose on a game's page and your own documents are never replaced.</small></div>
    <label class="check-row"><input type="checkbox" id="scrape-fresh" checked=${!!o.fresh} onChange=${(e) => set({ fresh: e.target.checked })} /> Ask the sources again <span class="muted">(their answers are otherwise kept for a week)</span></label>
  <//>`;
}

export const scrapeDialog = (opts) => ui.set({ dialog: { type: "scrape", ...opts } });

// ------------------------------------------------------------ Delete metadata…

/** Delete metadata…: what a system's games (or one game's folder) have from the sources,
 *  set aside as a change with a preview and Undo (D48). A PC game's (`pc`) offers its
 *  kinds of artwork (D55). */
function DeleteDialog({ platform = "", platformName = "", folder = "", gameName = "", pc = false }) {
  const [c, setC] = useState(null);
  const [what, setWhat] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { api("scrape_choices").then((x) => { const d = pc ? x.delete_pc : x.delete; setC(d); setWhat(d.map((w) => w.id)); }).catch((e) => setError(err(e))); }, []);
  const whose = folder ? gameName : platformName;
  const run = async () => {
    setBusy(true);
    setError("");
    try {
      const plan = await api("meta_delete", { platform: folder ? "" : platform, folder, what });
      ui.set({ dialog: { type: "organise", plan, platformName } });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`Delete ${whose}'s metadata`} id="meta-delete-dialog" onSubmit=${run} busy=${busy || !c || !what.length} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <p class="muted">${folder ? "What this game has from the sources" : `What ${platformName}'s games have from the sources`} is set aside, with a preview first and Undo after. Your own documents and what you've written yourself stay.</p>
    ${c ? html`<${Ticks} id="meta-delete-what" items=${c} value=${what} onChange=${setWhat} />` : html`<p class="muted">Reading…</p>`}
  <//>`;
}

export const deleteMetadata = (opts) => ui.set({ dialog: { type: "meta-delete", ...opts } });

export function MetaDialogs({ dialog }) {
  if (dialog.type === "scrape") return html`<${ScrapeDialog} ...${dialog} />`;
  if (dialog.type === "meta-delete") return html`<${DeleteDialog} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ what an export carries

/** The Export dialog's choices of what goes with the games: `value` { fields, media,
 *  manuals, docs, image_size }; `target` esde | folder. */
export function CarryChoices({ value, onChange, carries, layout }) {
  const [c, setC] = useState(null);
  useEffect(() => { api("scrape_choices").then(setC).catch(() => setC({ fields: [], media: [] })); }, []);
  if (!c) return null;
  const can = carries || { fields: c.fields.map((f) => f.id), media: c.media.map((m) => m.id), manuals: true, docs: true, bios: false };
  const set = (patch) => onChange({ ...value, ...patch });
  const fields = c.fields.filter((f) => can.fields.includes(f.id));
  const pictures = c.media.filter((m) => m.id !== "manual" && can.media.includes(m.id));
  const where = layout?.gamelist ? "in each system's gamelist" : "as text files beside the pictures";
  return html`<details class="field-block carry" id="export-carry" open=${!!(value.fields.length || value.media.length || value.manuals || value.docs || value.bios || value.magazines)}>
    <summary>What it carries <span class="muted">(details, pictures, manuals, BIOS files, magazines)</span></summary>
    <p class="muted">The library keeps everything it scraped; an export takes only what you tick here, where ${layout ? `the layout ${layout.name}` : "its layout"} puts it.</p>
    ${fields.length ? html`<div class="field-block"><span class="field-label">Details, ${where}</span>
      <${Ticks} id="carry-fields" items=${fields} value=${value.fields} onChange=${(f) => set({ fields: f })} /></div>` : null}
    ${pictures.length ? html`<div class="field-block"><span class="field-label">Pictures and video</span>
      <${Ticks} id="carry-media" items=${pictures} value=${value.media} onChange=${(media) => set({ media })} />
      ${layout?.id === "esde" ? html`<small class="muted">Into ES-DE's downloaded_media folder (or the one its settings name), named after each game: give ES-DE's folder above.</small>` : null}</div>` : null}
    ${can.manuals ? html`<label class="check-row"><input type="checkbox" id="carry-manuals" checked=${value.manuals} onChange=${(e) => set({ manuals: e.target.checked })} /> Each game's manual (yours first, else a scraped one)</label>` : null}
    ${can.docs ? html`<label class="check-row"><input type="checkbox" id="carry-docs" checked=${value.docs} onChange=${(e) => set({ docs: e.target.checked })} /> Every document of each game (guides, maps, posters)</label>` : null}
    ${can.bios ? html`<label class="check-row"><input type="checkbox" id="carry-bios" checked=${!!value.bios} onChange=${(e) => set({ bios: e.target.checked })} /> The systems' BIOS files the library has (<a href=${routeHash("bios")} onClick=${close}>BIOS files</a>)</label>` : null}
    <label class="check-row"><input type="checkbox" id="carry-magazines" checked=${!!value.magazines} onChange=${(e) => set({ magazines: e.target.checked })} /> The library's magazines, in a Magazines folder (<a href=${routeHash("magazines")} onClick=${close}>Magazines</a>)</label>
    ${value.media.length && pictures.length ? html`<label class="field-inline"><span>Picture size</span>
      <select id="carry-size" class="compact" value=${String(value.image_size || 0)} onChange=${(e) => set({ image_size: Number(e.target.value) })}>
        <option value="0">As they are</option>
        ${[320, 480, 640, 800, 1080].map((n) => html`<option key=${n} value=${String(n)}>At most ${n} pixels (for a handheld's screen)</option>`)}
      </select></label>` : null}
  </details>`;
}

export const NO_CARRY = { fields: [], media: [], manuals: false, docs: false, bios: false, magazines: false, image_size: 0 };

// ------------------------------------------------------------ a game's panel

function MetaPanel({ id, off, version, onChange }) {
  const [g, setG] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const load = () => api("meta_game", { id }).then(setG).catch((e) => setError(err(e)));
  useEffect(() => { setError(""); load(); }, [id, version]);
  // files dropped on the window while the panel shows are attached to its game
  useDroppedFiles((paths) => { if (g && !off) attachDocs({ game: g.id, gameName: g.title, platform: g.platform, paths }); });
  const pick = async (source, sid) => {
    setBusy(true);
    setError("");
    try {
      setG(await api("meta_pick", { game: id, source, id: sid }));
      onChange();
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (error && !g) return html`<aside class="pc-panel" id="meta-panel"><p class="warn-note" role="alert">${error}</p></aside>`;
  if (!g) return html`<aside class="pc-panel" id="meta-panel"><p class="muted">Reading…</p></aside>`;
  const d = g.details || {};
  const facts = [["Released", d.released], ["Developer", d.developer], ["Publisher", d.publisher], ["Genres", Array.isArray(d.genres) ? d.genres.join(", ") : d.genres], ["Players", d.players], ["Rating", d.rating ? `${Math.round(d.rating * 100)}%` : null]].filter(([, v]) => v);
  const kinds = Object.keys(g.media || {});
  const from = [...new Set(Object.keys(d).map((k) => g.scraped?.[k]).filter((x) => typeof x === "string"))];
  const identify = () => identifyDialog({
    title: `Identify ${g.name}`, query: g.title, platform: g.platform,
    onPick: async (x) => { await pick(x.source, x.id); },
  });
  return html`<aside class="pc-panel meta-panel" id="meta-panel" data-id=${g.id}>
    <div class="pc-panel-head">
      <${Picture} media=${g.media} title=${g.title} cls="meta-cover big" />
      <div><h2 id="meta-title">${g.title}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</h2>
        <p class="muted">${[g.regions, g.platform_name, g.have ? "in the library" : "not in the library", Object.entries(g.ids || {}).map(([k, v]) => `${sourceName(k)} ${v}`).join(", ") || null].filter(Boolean).join(" · ")}</p></div>
    </div>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    <div class="home-actions meta-actions">
      <a class="primary compact button-link" id="meta-open" href=${`#/game/${encodeURIComponent(g.platform)}/${encodeURIComponent(g.name)}`}>Open its page</a>
      <button type="button" class="ghost compact" id="meta-scrape-one" disabled=${off || busy} onClick=${() => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title })}>${Icon.scrape(14)} Scrape…</button>
      <${MoreButton} id="meta-game-menu" title="Identify, attach documents, show its folder, scrape again, delete metadata" items=${() => [
        { id: "meta-identify", label: "Identify…", icon: "search", disabled: off || busy, run: identify },
        { id: "meta-attach", label: "Attach documents…", icon: "file", disabled: off || busy || !isDesktop(), run: () => attachDocs({ game: g.id, gameName: g.title, platform: g.platform }) },
        ...(g.folder_path && isDesktop() ? [{ id: "meta-reveal", label: "Show in folder", icon: "folder", run: () => reveal(g.folder_path) }] : []),
        ...(g.folder && !off ? [{ sep: true },
          { id: "meta-again", label: "Scrape again…", icon: "scrape", run: () => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title, again: true }) },
          { id: "meta-delete", label: "Delete metadata…", icon: "trash", run: () => deleteMetadata({ folder: g.folder, gameName: g.title, platformName: g.platform_name }) }] : []),
      ]} />
    </div>
    ${g.review?.length ? html`<section class="pc-section" id="meta-review"><h3>Which game is it?</h3>
      <p class="muted">The scrape wasn't sure. Pick the right one and its details and pictures are taken.</p>
      <ul class="ls-list identify-list">${g.review.map((x) => html`<li key=${`${x.source}-${x.id}`} data-source=${x.source} data-id=${x.id}>
        ${x.cover ? html`<img class="identify-cover" src=${x.cover} alt="" loading="lazy" />` : html`<span class="identify-cover pc-cover-none" aria-hidden="true">${x.title.slice(0, 1)}</span>`}
        <span class="identify-label"><span><b>${x.title}</b>${x.year ? ` (${x.year})` : ""}</span><span class="muted">${x.source_name || x.source} · ${x.id}${x.url ? html` · <a href=${x.url} target="_blank" rel="noreferrer">page</a>` : null}</span></span>
        <button type="button" class="ghost" data-action="pick" disabled=${off || busy} onClick=${() => pick(x.source, x.id)}>This one</button>
      </li>`)}</ul></section>` : null}
    ${d.description ? html`<p class="pc-description" id="meta-description">${d.description}</p>` : null}
    ${facts.length ? html`<dl class="pc-facts" id="meta-facts">${facts.map(([k, v]) => html`<dt key=${`k${k}`}>${k}</dt><dd key=${`v${k}`}>${v}</dd>`)}</dl>` : null}
    ${from.length ? html`<p class="muted">Details from ${from.map(sourceName).join(", ")}.</p>` : null}
    ${kinds.length ? html`<section class="pc-section"><h3>Pictures</h3>
      <div class="meta-media" id="meta-media">${kinds.map((k) => html`<figure key=${k} data-kind=${k}><${Picture} media=${g.media} kind=${k} title=${g.title} cls="meta-thumb" /><figcaption class="muted">${PICTURES[k] || k}${g.scraped?.media?.[k] ? ` · ${sourceName(g.scraped.media[k])}` : ""}</figcaption></figure>`)}</div></section>` : null}
    <section class="pc-section"><h3>Documents</h3>
      ${g.docs.length ? html`<ul class="ls-list" id="meta-docs">${g.docs.map((x) => html`<li key=${x.id} data-kind=${x.kind}>
        <button type="button" class="linkish" onClick=${() => viewDoc(x.id)}>${x.title}</button><span class="muted">${x.kind_name}${x.source ? ` · ${sourceName(x.source)}` : ""}</span></li>`)}</ul>`
        : html`<p class="muted">None yet. Attach your own, or scrape its manual.</p>`}</section>
    ${!kinds.length && !Object.keys(d).length && !g.docs.length ? html`<p class="muted" id="meta-nothing">Nothing in the library for this game yet: Scrape… or Identify… fills it in.</p>` : null}
  </aside>`;
}

// ------------------------------------------------------------ the page

export function Metadata() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes, home: st.home }));
  const saved = prefs()?.get("gr-meta", {}) || {};
  const [platform, setPlatform] = useState(saved.platform || null);
  const [show, setShow] = useState(saved.show || "have");
  const [summary, setSummary] = useState(null);
  const [q, setQ] = useState("");
  const [list, setList] = useState({ total: 0, items: [], loading: true });
  const [sel, setSel] = useState(null);
  const [tick, setTick] = useState(0);
  const off = !!s.library?.read_only;
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("audit_summary", { platform }).then((d) => { if (!gone) setSummary(d); }).catch((e) => { if (!gone) setSummary({ error: err(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, platform, s.home]);
  const p = summary?.summary?.dat?.platform;
  useEffect(() => {
    if (!p) return;
    let gone = false;
    setList((l) => ({ ...l, loading: true }));
    const t = setTimeout(() => {
      api("meta_list", { platform: p, show, search: q, offset: 0, limit: PAGE })
        .then((r) => { if (!gone) setList({ ...r, loading: false }); })
        .catch((e) => { if (!gone) setList({ total: 0, items: [], loading: false, error: err(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [p, show, q, s.changes, tick]);
  const more = async () => {
    const r = await api("meta_list", { platform: p, show, search: q, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  const choose = (patch) => {
    const next = { platform: patch.platform ?? p, show: patch.show ?? show };
    if (patch.platform) { setPlatform(patch.platform); setSel(null); }
    if (patch.show) setShow(patch.show);
    prefs()?.set("gr-meta", next);
  };
  const platforms = summary?.platforms || [];
  const pName = platforms.find((x) => x.id === p)?.name || p;
  const counts = list.counts || {};
  const head = html`<${CommandBar} id="meta-head" title="Metadata" count=${p ? pName : null}
    search=${p ? html`<input type="search" id="meta-search" class="compact" placeholder="Search this system's games" aria-label="Search this system's games" value=${q} onInput=${(e) => setQ(e.target.value)} />` : null}
    primary=${p ? html`<button type="button" class="primary compact" id="meta-scrape" disabled=${off} title="Fill in the games' details, pictures and manuals from the sources"
      onClick=${() => scrapeDialog({ platform: p, platformName: pName, have: counts.have || 0, all: counts.all || 0 })}>${Icon.scrape(15)} Scrape…</button>` : null}
    more=${{ id: "meta-menu", title: "Documents, sources", items: () => [
      { id: "meta-docs-page", label: "Documents", icon: "book", run: () => { location.hash = routeHash("documents"); } },
      { id: "meta-import-docs", label: "Import documents…", icon: "file", disabled: off || !isDesktop(), run: () => attachDocs({ platform: p }) },
      { sep: true },
      { id: "meta-scrape-again", label: "Scrape again…", icon: "scrape", disabled: off || !p, run: () => scrapeDialog({ platform: p, platformName: pName, have: counts.have || 0, all: counts.all || 0, again: true }) },
      { id: "meta-delete-all", label: "Delete metadata…", icon: "trash", disabled: off || !p, run: () => deleteMetadata({ platform: p, platformName: pName }) },
      { sep: true },
      { id: "meta-sources", label: "Sources settings", icon: "cog", run: () => { location.hash = "#/settings"; setTimeout(() => document.getElementById("settings-sources")?.scrollIntoView(), 200); } },
    ] }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!summary) return html`${head}<div class="pages"><p class="muted">Reading the systems…</p></div>`;
  if (summary.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${summary.error}</p></div>`;
  if (!p) {
    return html`${head}<div class="pages"><div class="library-settings" id="meta-empty">
      <p class="page-lead">${Icon.scrape(18)} Each game's details, pictures, video and manual, from ScreenScraper, IGDB, TheGamesDB and LaunchBox, kept with the game in the library.</p>
      <section class="ls-section"><h2>To start</h2>
        <p>Games are known by their DAT, so import one for a system first (the <a href=${routeHash("audit")}>Audit</a> page), then set the sources' accounts or keys in <a href=${routeHash("settings")}>Settings › Sources</a>.</p></section>
    </div></div>`;
  }
  const current = sel && list.items.some((g) => g.id === sel) ? sel : null;
  return html`${head}<div class="pages pc-page meta-page" id="meta-page">
    <div class="audit-pick">
      <label class="field-inline"><span>System</span>
        <select id="meta-platform" class="compact" value=${p} onChange=${(e) => choose({ platform: e.target.value })}>
          ${platforms.map((x) => html`<option key=${x.id} value=${x.id}>${x.name}</option>`)}
        </select></label>
      <span class="muted" id="meta-counts">${counts.all != null ? `${(counts.have || 0).toLocaleString()} of ${(counts.all || 0).toLocaleString()} games in the library · ${(counts.details || 0).toLocaleString()} with details · ${(counts.media || 0).toLocaleString()} with pictures` : ""}</span>
    </div>
    <div class="seg audit-tabs" role="tablist" id="meta-tabs">${SHOWS.map(([k, label, key]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${show === k ? "true" : "false"} onClick=${() => choose({ show: k })}>
      ${label}${key ? html` <span class="tab-count">${(counts[key] || 0).toLocaleString()}</span>` : null}</button>`)}</div>
    <p class="muted meta-help">${SHOW_HELP[show]}</p>
    <div class="pc-body">
      <div class="pc-list-wrap">
        <${SearchNote} q=${q} found=${list.loading ? null : plural(list.total, "game", "games")} onClear=${() => setQ("")} />
        ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
        ${!list.loading && !list.items.length ? html`<p class="muted" id="meta-none">${q ? "No game here matches the search." : show === "look" ? "None: every game a scrape found was a sure match." : "No games here."}</p>` : null}
        <ul class="pc-list" id="meta-list" data-total=${list.total} aria-busy=${list.loading ? "true" : "false"}>${list.items.map((g) => html`<li key=${g.id}>
          <button type="button" class=${`pc-row${current === g.id ? " active" : ""}`} data-id=${g.id} data-name=${g.name} aria-pressed=${current === g.id ? "true" : "false"} onClick=${() => setSel(g.id)}>
            <span class=${`meta-dot${g.status === "missing" ? " off" : ""}`} title=${g.status === "missing" ? "Not in the library" : "In the library"}></span>
            <span class="pc-row-main"><span class="pc-row-title">${g.title || g.name}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</span>
              <span class="muted pc-row-meta">${rowMeta(g)}</span></span>
          </button></li>`)}</ul>
        ${list.items.length < list.total ? html`<div class="home-actions"><button type="button" class="ghost" id="meta-more" onClick=${more}>Show ${Math.min(PAGE, list.total - list.items.length)} more (${list.total - list.items.length} left)</button></div>` : null}
      </div>
      ${current ? html`<${MetaPanel} key=${current} id=${current} off=${off} version=${`${tick}-${s.changes}`} onChange=${() => setTick((t) => t + 1)} />` : html`<aside class="pc-panel pc-panel-empty"><p class="muted">Pick a game to see what the library has for it, scrape it, or attach your documents.</p></aside>`}
    </div>
  </div>`;
}
