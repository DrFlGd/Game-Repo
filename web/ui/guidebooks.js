// Guidebooks (docs/PLAN.md, "Maps and Guides notes, part 10", D232–D240): published guides you
// have as one file (PDF, CBZ or CBR), kept in Maps and Guides beside a game's packs, in
// `<Game title>/Guidebooks/`. They're shown on the Maps and Guides page among the packs, each a
// card of its own (its title, pages, size, file type and whether its text is read), and on a
// game's page. Import guidebooks… copies (or moves) files into the game's folder, as a change
// with Undo; Rename… changes a title; Remove… sets one aside, with Undo; Read their text reads
// each page as the magazines' reader does (a job). Game Repo Server sends them to Atlas in a
// pack of their own for each game, made when a device asks.
import { html } from "../lib/html.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size } from "./library.js";
import { reveal } from "./games.js";
import { runPlan } from "./organise.js";
import { Icon } from "./icons.js";
import { Pic, PanelActions, askGame, askValue } from "./shelf.js";
import { gameHash, systemColour } from "./browse.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const fail = (e) => toast(err(e), 8000);

/** A guidebook's file type, as it's said. */
export const GUIDEBOOK_KIND = { pdf: "PDF", cbz: "CBZ", cbr: "CBR" };

/** Whether its text is read, in plain words. */
export const textWords = (g) => (!g.text ? "Text not read" : g.text.pages ? `Text read, ${plural(g.text.pages, "page", "pages")}` : "Read: no text found");

/** What's said of a guidebook: its file type, pages, size and text. */
export const guidebookFacts = (g) => [GUIDEBOOK_KIND[g.kind] || g.kind, g.pages ? plural(g.pages, "page", "pages") : null, size(g.size), textWords(g)].filter(Boolean).join(" · ");

/** A guidebook as one of the Maps and Guides page's cards: of the sort Guidebooks, as a pack of them is (D231). */
export const guidebookItem = (g) => ({ ...g, kinds: ["guidebook"], sort: "guidebooks", games: g.games || [] });

export const guidebookNoPic = (g) => ({ icon: "book", colour: systemColour(g.games?.[0]?.platform || "maps") });

/** Its card on the shelf (`tile` and `line`). */
export const guidebookTile = (g) => ({
  pics: [], none: guidebookNoPic(g),
  badge: { text: `Guidebook · ${GUIDEBOOK_KIND[g.kind] || g.kind}`, colour: systemColour(g.games?.[0]?.platform || "maps") },
  title: g.title, meta: [g.games?.[0]?.title || g.folder, g.pages ? plural(g.pages, "page", "pages") : null, size(g.size)].filter(Boolean).join(" · "),
  missing: !!g.cannot_go,
});

async function planned(r) {
  if (r?.plan) await runPlan(r.plan);
  again();
}

/** Import guidebooks…: PDF, CBZ or CBR files picked, into a game's folder (`key`, else asked), then their text read (a job). */
export async function importGuidebooks(key = null, gameName = null) {
  const paths = (await ctx.platform.library.pickFiles(gameName ? `Guidebooks for ${gameName} (PDF, CBZ or CBR)` : "Guidebooks (PDF, CBZ or CBR)")) || [];
  if (!paths.length) return;
  const go = async (k) => {
    const r = await api("guidebooks_import", { paths, key: k, mode: "copy" });
    await planned(r);
    readText([], true);
  };
  if (key) return go(key).catch(fail);
  askGame({ title: "Import guidebooks for a game", button: "Import them", note: "They're copied into the game's folder in Maps and Guides (Guidebooks), and go to Atlas with the game.", run: async (g) => { if (g) await go(g.key); } });
}

/** Read their text: those picked (or every one not read yet), as a job. */
export async function readText(rels = [], auto = false) {
  try {
    const { job } = await api("guidebooks_read", { rels, auto });
    const done = await followJob(job, "Reading guidebooks' text");
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    if (!auto || r.guidebooks) toast(`Read the text of ${plural(r.guidebooks || 0, "guidebook", "guidebooks")}${r.notes?.length ? `. ${r.notes[0]}` : "."}`, r.notes?.length ? 10000 : 4000);
    again();
  } catch (e) {
    if (!auto) fail(e);
  }
}

const rename = (g) => askValue({
  title: "Rename the guidebook", label: "Its title", value: g.title, button: "Rename", empty: true,
  note: "Empty: its file's name. Atlas shows this title.",
  run: async (t) => { await api("guidebooks_title", { rel: g.rel, title: t }); again(); },
});

const linkFolder = (g) => askGame({
  title: "Link these guidebooks to a game", button: "Link them",
  note: `Every guidebook in ${g.folder} goes with this game to Atlas.`,
  run: async (x) => { if (x) { await api("guidebooks_link", { folder: g.folder, key: x.key }); again(); } },
});

async function remove(list) {
  try {
    const r = await api("guidebooks_remove", { rels: list.map((g) => g.rel) });
    if (r.plan) ui.set({ dialog: { type: "organise", plan: r.plan } });
  } catch (e) {
    toast(`Couldn't plan it: ${err(e)}`, 8000);
  }
}

const open = (g) => ctx.platform.library.openPath(`${ui.get().library.path}/${g.path}`);

/** The actions for guidebooks picked on the shelf. */
export function guidebookActions(list, off) {
  const one = list.length === 1 ? list[0] : null;
  const desk = isDesktop();
  return [
    { id: "gb-open", label: "Open", icon: "book", one: true, disabled: !one || !desk, run: () => one && open(one) },
    { id: "gb-read", label: "Read their text", icon: "refresh", disabled: off, run: () => readText(list.map((g) => g.rel)) },
    { id: "gb-rename", label: "Rename…", icon: "edit", one: true, disabled: off || !one, run: () => one && rename(one) },
    { id: "gb-link", label: one?.games?.length ? "Link to another game…" : "Link to a game…", icon: "gamepad", one: true, disabled: off || !one, run: () => one && linkFolder(one) },
    { id: "show", label: "Show in folder", icon: "folder", one: true, disabled: !desk || !one, run: () => one && reveal(`${ui.get().library.path}/${one.path}`) },
    { id: "gb-remove", label: "Remove…", icon: "trash", disabled: off, run: () => remove(list) },
  ];
}

/** One guidebook in the panel. */
export function GuidebookPanel({ g, off }) {
  const actions = guidebookActions([g], off);
  return html`<div class="shelf-panel" id="guidebook-edit" data-rel=${g.rel}>
    <${Pic} srcs=${[]} none=${guidebookNoPic(g)} cls="lib-panel-cover" px=${64} />
    <h2 id="guidebook-panel-title">${g.title}</h2>
    <p class="muted" id="guidebook-panel-facts">${guidebookFacts(g)}</p>
    ${g.cannot_go ? html`<p class="warn-note" id="guidebook-panel-cannot">${g.cannot_go}</p>` : html`<p class="muted" id="guidebook-panel-atlas">Goes to Atlas with ${g.games?.length ? "its game" : "its game, once it has one"}, in a pack of the game's guidebooks.</p>`}
    ${g.text?.note ? html`<p class="muted">${g.text.note}</p>` : null}
    <${PanelActions} page="guidebook" actions=${actions} />
    <h3>Games</h3>
    ${g.games?.length ? html`<ul class="ls-list shelf-games" id="guidebook-games">${g.games.map((x) => html`<li key=${x.key} data-key=${x.key}><a href=${gameHash(x.key)}>${x.title}</a> <span class="muted">${x.platform_name}</span></li>`)}</ul>
      <p class="muted">${g.how === "title" ? `Found by its folder's name, ${g.folder}.` : g.how === "packs" ? "The game of the packs in its folder." : ""}</p>`
      : html`<p class="muted">None: no game's title is its folder's, ${g.folder}. Link it to a game.</p>`}
    <p class="muted maps-file" title=${g.path}>${g.rel}</p>
  </div>`;
}

/** A game's guidebooks on its page: each with its facts and Open, and Import guidebooks…. */
export function GameGuidebooks({ g }) {
  const list = g.guidebooks || [];
  const off = !!ui.get().library?.read_only;
  const name = g.details?.title || g.title || "";
  return html`<section class="ls-section" id="game-guidebooks"><h2>Guidebooks</h2>
    ${list.length ? html`<ul class="ls-list">${list.map((x) => html`<li key=${x.rel} data-rel=${x.rel}><span class="op-paths">
      <span>${Icon.book(14)} ${x.title}</span>
      <span class="muted">${guidebookFacts(x)}${x.cannot_go ? ` · ${x.cannot_go}` : ""}</span></span>
      <button type="button" class="ghost compact" data-action="open-guidebook" disabled=${!isDesktop()} onClick=${() => open(x)}>Open</button></li>`)}</ul>`
      : html`<p class="muted">None yet. A published guide you have as a PDF, CBZ or CBR goes to Atlas with the game.</p>`}
    <div class="home-actions">
      <button type="button" class="ghost compact" id="game-guidebooks-import" disabled=${off || !isDesktop()} onClick=${() => importGuidebooks(g.key, name)}>${Icon.inbox(14)} Import guidebooks…</button>
      ${list.some((x) => !x.text) ? html`<button type="button" class="ghost compact" id="game-guidebooks-read" disabled=${off} onClick=${() => readText(list.map((x) => x.rel))}>Read their text</button>` : null}
    </div>
    <p class="muted"><a href=${routeHash("maps")}>Maps and Guides</a> has every guidebook.</p>
  </section>`;
}
