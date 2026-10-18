// Import (docs/PLAN.md, "UI pass 3 design", D71 and D73): everything that brings files into
// the library, in one place. Games in folders you keep (the folders with games, scanned
// where they are), games copied into the library from a folder (matched by the audit DATs,
// named as they say, D53's bring-in) or one at a time (Add a game…), PC games, DATs, BIOS
// files, documents and magazines. Other pages link here instead of repeating these.
import { html, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, followJob, isDesktop, plural } from "./library.js";
import { CommandBar } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { RootsSection, DatsSection } from "./settings.js";
import { importPc, addGameDialog } from "./pcgames.js";
import { importBios } from "./bios.js";
import { attachDocs } from "./documents.js";
import { importMagazines } from "./magazines.js";
import { importDats, starterDats } from "./games.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
const baseName = (p) => String(p).replace(/[\\/]+$/, "").split(/[\\/]/).pop();

/** Games to copy into the library: folders read and matched against the audit DATs, then
 *  the games the library lacks copied into its folders with games under their DAT names. */
function ImportGamesDialog() {
  const [folders, setFolders] = useState([]);
  const [typed, setTyped] = useState("");
  const [review, setReview] = useState(null);
  const [off, setOff] = useState(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const add = (p) => { if (p && !folders.includes(p)) setFolders((f) => [...f, p]); };
  const read = async () => {
    setBusy(true);
    setError("");
    try {
      const paths = typed.trim() && !folders.includes(typed.trim()) ? [...folders, typed.trim()] : folders;
      if (typed.trim()) { setFolders(paths); setTyped(""); }
      const { job } = await api("collection_read_folders", { paths });
      const done = await followJob(job, "Reading folders");
      if (done.error) throw new Error(done.error);
      setOff(new Set());
      setReview(done.result);
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  // what can come in: games the library doesn't have, with the files that are them
  const wanted = () => (review?.systems || []).flatMap((x) => x.games).filter((g) => g.status === "missing" && g.files?.length);
  const bring = async () => {
    setBusy(true);
    setError("");
    try {
      const games = wanted().filter((g) => !off.has(g.id));
      const plan = await api("collection_bring_plan", { games: games.map((g) => ({ id: g.id, files: g.files })) });
      ui.set({ dialog: { type: "import-games-preview", plan } });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  if (review) {
    const can = wanted();
    const picked = can.filter((g) => !off.has(g.id)).length;
    const tick = (id) => setOff((s0) => { const s1 = new Set(s0); s1.has(id) ? s1.delete(id) : s1.add(id); return s1; });
    const unknown = review.unknown || [];
    return html`<${Dialog} title="The folders' games" id="import-games-review" wide=${true} onSubmit=${bring} busy=${busy || !picked} error=${error}
      submitLabel=${`Preview (${plural(picked, "game", "games")})`} cancelLabel="Back" onCancel=${() => setReview(null)}>
      <p id="import-games-totals">Read ${plural(review.read, "file", "files")}: <b>${plural(can.length, "game", "games")}</b> the library doesn't have${review.games - can.length > 0 ? `, ${plural(review.games - can.length, "game", "games")} it has already` : ""}${unknown.length ? `, ${plural(unknown.length, "file", "files")} no DAT knows` : ""}.</p>
      <div class="subset-systems" id="import-games-systems">${review.systems.map((x) => html`<section key=${x.platform} class="subset-system" data-platform=${x.platform}>
        <h3>${x.name} <span class="muted">${plural(x.games.length, "game", "games")}, ${x.have} in the library</span></h3>
        <ul class="ls-list organise-list">${x.games.map((g) => html`<li key=${g.id} data-name=${g.name} data-status=${g.status}>
          ${g.status === "missing" && g.files?.length ? html`<label class="check-row"><input type="checkbox" checked=${!off.has(g.id)} onChange=${() => tick(g.id)} /> ${g.name}</label>`
            : html`<span class="muted">${g.name}</span>`}
          <span class="muted" title=${g.from.join("\n")}>${g.status === "have" ? "in the library · " : g.status === "partial" ? "partly in the library · " : ""}${g.from.length > 1 ? plural(g.from.length, "file", "files") : g.from[0]}</span></li>`)}</ul>
      </section>`)}</div>
      ${unknown.length ? html`<details class="field-block subset-unknown" id="import-games-unknown"><summary>${plural(unknown.length, "file", "files")} no DAT knows</summary>
        <ul class="ls-list">${unknown.slice(0, 500).map((u, i) => html`<li key=${i} data-path=${u.path}><span class="op-to">${u.path}</span> <span class="muted">${u.why}</span></li>`)}</ul>
        <div class="home-actions"><button type="button" class="ghost" id="import-games-add" onClick=${() => addGameDialog({ files: unknown.map((u) => u.path) })}>Add them to a system…</button></div></details>` : null}
      <p class="muted">Each game goes into a folder with games for its system, named as its DAT says. Nothing changes until you've seen the preview.</p>
    <//>`;
  }
  return html`<${Dialog} title="Copy games into the library" id="import-games-dialog" onSubmit=${read} busy=${busy || !(folders.length || typed.trim())} error=${error} submitLabel=${busy ? "Reading…" : "Read"}>
    <div class="field-block"><span class="field-label">Folders</span>
      ${folders.length ? html`<ul class="ls-list" id="import-folders">${folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${baseName(f)} off`} onClick=${() => setFolders((v) => v.filter((x) => x !== f))}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="import-folder-path" placeholder="A folder's path" value=${typed} onInput=${(e) => setTyped(e.target.value)} onKeyDown=${(e) => { if (e.key === "Enter" && typed.trim()) { e.preventDefault(); add(typed.trim()); setTyped(""); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="import-add-folder" onClick=${async () => add(await ctx.platform.library.pickFolder("A folder of games to copy into the library"))}>Add a folder…</button>` : null}
      </div>
      <small class="muted">Every file in them is read and matched against your DATs; the folders aren't changed.</small></div>
  <//>`;
}

/** The preview of copying the games in; Run starts it. */
function ImportGamesPreview({ plan }) {
  const nothing = !plan.ops;
  return html`<${Dialog} title=${plan.title || "Import the games"} id="bring-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} submitLabel=${nothing ? "OK" : "Run"} cancelLabel=${nothing ? null : "Not now"}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to import: ${plan.left?.length ? "see what's left alone." : "the library has them all."}</p>`
      : html`<p class="muted">Hardlinked on the same drive (they take no space), else copied, then checked and scanned. History can undo it.</p>`}
  <//>`;
}

export function ImportDialogs({ dialog }) {
  if (dialog.type === "import-games") return html`<${ImportGamesDialog} />`;
  if (dialog.type === "import-games-preview" || dialog.type === "bring") return html`<${ImportGamesPreview} ...${dialog} />`;
  return null;
}

export const importGamesDialog = () => ui.set({ dialog: { type: "import-games" } });

/** One kind of thing to import: what it is, where it goes, its buttons. */
function Card({ id, icon, title, where, children }) {
  return html`<section class="import-card" id=${id}>
    <h2>${Icon[icon](16)} ${title}</h2>
    <p class="muted">${where}</p>
    <div class="home-actions">${children}</div>
  </section>`;
}

export function ImportPage() {
  const lib = useStore(ui, (s) => s.library);
  const head = html`<${CommandBar} id="import-head" title="Import" />`;
  if (!lib) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const off = !!lib.read_only;
  const desk = isDesktop();
  return html`${head}<div class="pages import-page" id="import-page"><div class="library-settings">
    <p class="page-lead">${Icon.inbox(18)} Everything that comes into the library comes in here. Checking it is the <a href=${routeHash("audit")}>Audit</a>'s; choosing games to export is <a href=${routeHash("collections")}>Collections</a>'.</p>
    <div class="import-cards">
      <${Card} id="import-copy" icon="move" title="Games to copy into the library" where="Matched against your DATs and copied (hardlinked on the same drive) into a folder with games, named as the DAT says.">
        <button type="button" class="primary" id="import-games" disabled=${off} onClick=${importGamesDialog}>From a folder…</button>
        <button type="button" class="ghost" id="import-one" disabled=${off} onClick=${() => addGameDialog()}>One game…</button>
      <//>
      <${Card} id="import-pc" icon="monitor" title="PC games" where="Installers, archives and game folders, each into its own folder under PC/.">
        <button type="button" class="ghost" id="import-pc-files" disabled=${off || !desk} onClick=${() => importPc("files")}>Files…</button>
        <button type="button" class="ghost" id="import-pc-folder" disabled=${off || !desk} onClick=${() => importPc("folder")}>A game's folder…</button>
      <//>
      <${Card} id="import-bios" icon="chip" title="BIOS files" where="Copied into the library's BIOS folder, by system, from your folders with games.">
        <button type="button" class="ghost" id="import-bios-files" disabled=${off} onClick=${importBios}>Import BIOS files…</button>
        <button type="button" class="ghost" id="import-bios-list" disabled=${off} onClick=${() => starterDats(["bios"])}>Get libretro's list…</button>
        ${desk ? html`<button type="button" class="ghost" id="import-bios-dat" disabled=${off} onClick=${() => importDats(null, { "*": "bios" })}>A BIOS DAT…</button>` : null}
      <//>
      <${Card} id="import-docs" icon="book" title="Documents" where="Manuals, guides and scans, each paired with its game, into the game's folder.">
        <button type="button" class="ghost" id="import-docs-folder" disabled=${off || !desk} onClick=${() => attachDocs({ how: "folder" })}>A folder…</button>
        <button type="button" class="ghost" id="import-docs-files" disabled=${off || !desk} onClick=${() => attachDocs({ how: "files" })}>Files…</button>
      <//>
      <${Card} id="import-mags" icon="stack" title="Magazines" where="Issues (PDF, CBZ or CBR) into Magazines/, by magazine.">
        <button type="button" class="ghost" id="import-mags-folder" disabled=${off || !desk} onClick=${() => importMagazines()}>A folder…</button>
        <button type="button" class="ghost" id="import-mags-files" disabled=${off || !desk} onClick=${() => importMagazines(true)}>Files…</button>
      <//>
    </div>
    <${RootsSection} lib=${lib} key=${`f-${lib.path}`} />
    <${DatsSection} lib=${lib} />
    <p class="muted" id="import-history">What's been imported can be undone from <a href=${routeHash("history")}>History</a>.</p>
  </div></div>`;
}
