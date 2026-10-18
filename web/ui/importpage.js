// Import (docs/PLAN.md, "UI pass 3 design", D71 and D73): everything that brings files into
// the library, in one place. Games in folders you keep (the folders with games, scanned
// where they are), games copied into the library from a folder (matched by the audit DATs,
// named as they say, D53's bring-in) or one at a time (Add a game…), PC games, DATs, BIOS
// files, documents and magazines. Other pages link here instead of repeating these.
import { html, useState, useEffect } from "../lib/html.js";
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

const dirOf = (p) => { const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")); return i > 0 ? p.slice(0, i) : ""; };

/** Where a system's games go: its folders as `addgame::places` ranks them, or another. */
function PlaceRow({ platform, name, count, places, value, onChange }) {
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder(`The folder ${name}'s games go to`);
    if (p) onChange(p);
  };
  const listed = places || [];
  return html`<li class="import-place" data-platform=${platform} data-dir=${value || ""}>
    <span class="import-place-name"><b>${name}</b> <span class="muted">${plural(count, "game", "games")}</span></span>
    <span class="pick-row">
      <select class="compact" aria-label=${`Where ${name}'s games go`} value=${value || ""} onChange=${(e) => (e.target.value === "\u0000" ? pick() : onChange(e.target.value))}>
        ${value ? null : html`<option value="">Choose a folder…</option>`}
        ${listed.map((p) => html`<option key=${p.dir} value=${p.dir} title=${p.why}>${p.dir}</option>`)}
        ${value && !listed.some((p) => p.dir === value) ? html`<option value=${value}>${value}</option>` : null}
        ${isDesktop() ? html`<option value="\u0000">Another folder…</option>` : null}
      </select>
    </span>
    ${value ? html`<small class="muted">${listed.find((p) => p.dir === value)?.why || "A new folder with games for it"}</small>` : html`<small class="warn">There's no folder with games for ${name} yet: choose where they go.</small>`}
  </li>`;
}

/** Games to copy into the library: folders read and matched against the audit DATs (every
 *  system's, or one system's), then each system's games copied or moved into its own
 *  folder with games under their DAT names; files no DAT knows go in if given a system. */
function ImportGamesDialog() {
  const platforms = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.id !== "pc").sort((a, b) => a.name.localeCompare(b.name));
  const [folders, setFolders] = useState([]);
  const [typed, setTyped] = useState("");
  const [only, setOnly] = useState("");
  const [mode, setMode] = useState("copy");
  const [review, setReview] = useState(null);
  const [off, setOff] = useState(new Set());
  const [where, setWhere] = useState({});
  const [lists, setLists] = useState({});
  const [given, setGiven] = useState({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const add = (p) => { if (p && !folders.includes(p)) setFolders((f) => [...f, p]); };
  const nameOf = (id) => platforms.find((p) => p.id === id)?.name || id;
  const read = async () => {
    setBusy(true);
    setError("");
    try {
      const paths = typed.trim() && !folders.includes(typed.trim()) ? [...folders, typed.trim()] : folders;
      if (typed.trim()) { setFolders(paths); setTyped(""); }
      const { job } = await api("collection_read_folders", { paths });
      const done = await followJob(job, "Reading folders");
      if (done.error) throw new Error(done.error);
      const r = done.result;
      // the folders no DAT knows the files of: each the system it seems to be (or the one chosen)
      const groups = {};
      for (const u of r.unknown || []) if (u.file) (groups[dirOf(u.path)] ||= []).push(u);
      const g0 = {};
      for (const [dir, us] of Object.entries(groups)) {
        const seems = [...new Set(us.map((u) => u.system || ""))];
        g0[dir] = only || (seems.length === 1 ? seems[0] : "");
      }
      setLists(Object.fromEntries(r.systems.map((x) => [x.platform, x.places || []])));
      setWhere(Object.fromEntries(r.systems.map((x) => [x.platform, x.places?.[0]?.dir || ""])));
      setGiven(g0);
      setOff(new Set());
      setReview({ ...r, groups });
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  // the places of a system the read found no games of, once its files are given it
  useEffect(() => {
    for (const p of new Set(Object.values(given).filter(Boolean))) {
      if (lists[p]) continue;
      setLists((l) => ({ ...l, [p]: [] }));
      api("add_places", { platform: p }).then((ps) => {
        setLists((l) => ({ ...l, [p]: ps }));
        setWhere((w) => (w[p] ? w : { ...w, [p]: ps[0]?.dir || "" }));
      }).catch(() => {});
    }
  }, [given]);
  const systems = () => (review?.systems || []).filter((x) => !only || x.platform === only);
  // what can come in: games the library doesn't have, with the files that are them
  const wanted = () => systems().flatMap((x) => x.games.map((g) => ({ ...g, platform: x.platform }))).filter((g) => g.status === "missing" && g.files?.length);
  const files = () => Object.entries(review?.groups || {}).filter(([d]) => given[d]).map(([d, us]) => ({ platform: given[d], files: us.map((u) => u.file) }));
  const bring = async () => {
    setBusy(true);
    setError("");
    try {
      const games = wanted().filter((g) => !off.has(g.id));
      const plan = await api("collection_bring_plan", {
        games: games.map((g) => ({ id: g.id, files: g.files })), files: files(), places: where,
        mode: mode === "move" ? "move" : "copy", always_copy: mode === "always",
      });
      ui.set({ dialog: { type: "import-games-preview", plan } });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  if (review) {
    const can = wanted();
    const picked = can.filter((g) => !off.has(g.id));
    const tick = (id) => setOff((s0) => { const s1 = new Set(s0); s1.has(id) ? s1.delete(id) : s1.add(id); return s1; });
    const groups = Object.entries(review.groups);
    const loose = groups.filter(([d]) => given[d]).reduce((n, [, us]) => n + us.length, 0);
    // a row for each system something goes to
    const count = {};
    for (const g of picked) count[g.platform] = (count[g.platform] || 0) + 1;
    for (const f of files()) count[f.platform] = (count[f.platform] || 0) + f.files.length;
    const going = Object.keys(count).sort((a, b) => nameOf(a).localeCompare(nameOf(b)));
    const unplaced = going.filter((p) => !where[p]);
    const shown = systems();
    const unknownCount = (review.unknown || []).length;
    return html`<${Dialog} title="The folders' games" id="import-games-review" wide=${true} onSubmit=${bring} busy=${busy || !(picked.length + loose) || unplaced.length > 0} error=${error}
      submitLabel=${`Preview (${plural(picked.length + loose, "game", "games")})`} cancelLabel="Back" onCancel=${() => setReview(null)}>
      <p id="import-games-totals">Read ${plural(review.read, "file", "files")}: <b>${plural(can.length, "game", "games")}</b> the library doesn't have${only ? ` for ${nameOf(only)}` : ""}${unknownCount ? `, ${plural(unknownCount, "file", "files")} no DAT knows` : ""}.
        ${mode === "move" ? " They'll be moved." : mode === "always" ? " They'll be copied in full." : " They'll be copied (hardlinked on the same drive)."}</p>
      ${going.length ? html`<div class="field-block"><span class="field-label">Where each system's games go</span>
        <ul class="ls-list import-places" id="import-places">${going.map((p) => html`<${PlaceRow} key=${p} platform=${p} name=${nameOf(p)} count=${count[p]} places=${lists[p]} value=${where[p]}
          onChange=${(d) => setWhere((w) => ({ ...w, [p]: d }))} />`)}</ul></div>` : null}
      <div class="subset-systems" id="import-games-systems">${shown.map((x) => html`<section key=${x.platform} class="subset-system" data-platform=${x.platform}>
        <h3>${x.name} <span class="muted">${plural(x.games.length, "game", "games")}, ${x.have} in the library</span></h3>
        <ul class="ls-list organise-list">${x.games.map((g) => html`<li key=${g.id} data-name=${g.name} data-status=${g.status}>
          ${g.status === "missing" && g.files?.length ? html`<label class="check-row"><input type="checkbox" checked=${!off.has(g.id)} onChange=${() => tick(g.id)} /> ${g.name}</label>`
            : html`<span class="muted">${g.name}</span>`}
          <span class="muted" title=${g.from.join("\n")}>${g.status === "have" ? "in the library · " : g.status === "partial" ? "partly in the library · " : ""}${g.from.length > 1 ? plural(g.from.length, "file", "files") : g.from[0]}</span></li>`)}</ul>
      </section>`)}</div>
      ${groups.length ? html`<div class="field-block subset-unknown" id="import-games-unknown"><span class="field-label">Files no DAT knows</span>
        <small class="muted">Give a folder's files a system to import them as they're named, or leave them out.</small>
        <ul class="ls-list">${groups.map(([d, us]) => html`<li key=${d} class="import-unknown" data-folder=${d}>
          <span title=${us.map((u) => u.path).join("\n")}><span class="op-to">${d || "(the folder read)"}</span> <span class="muted">${plural(us.length, "file", "files")}</span></span>
          <select class="compact" aria-label=${`The system of the files in ${d}`} value=${given[d] || ""} onChange=${(e) => setGiven((g) => ({ ...g, [d]: e.target.value }))}>
            <option value="">Leave them out</option>
            ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
          </select></li>`)}</ul></div>` : null}
      ${(review.unknown || []).some((u) => !u.file) ? html`<details class="field-block"><summary>${plural(review.unknown.filter((u) => !u.file).length, "file", "files")} that couldn't be read</summary>
        <ul class="ls-list">${review.unknown.filter((u) => !u.file).slice(0, 200).map((u, i) => html`<li key=${i}><span class="op-to">${u.path}</span> <span class="muted">${u.why}</span></li>`)}</ul></details>` : null}
      <p class="muted">Each game is named as its DAT says. Nothing changes until you've seen the preview.</p>
    <//>`;
  }
  return html`<${Dialog} title="Import games from folders" id="import-games-dialog" onSubmit=${read} busy=${busy || !(folders.length || typed.trim())} error=${error} submitLabel=${busy ? "Reading…" : "Read"}>
    <div class="field-block"><span class="field-label">Folders</span>
      ${folders.length ? html`<ul class="ls-list" id="import-folders">${folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${baseName(f)} off`} onClick=${() => setFolders((v) => v.filter((x) => x !== f))}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="import-folder-path" placeholder="A folder's path" value=${typed} onInput=${(e) => setTyped(e.target.value)} onKeyDown=${(e) => { if (e.key === "Enter" && typed.trim()) { e.preventDefault(); add(typed.trim()); setTyped(""); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="import-add-folder" onClick=${async () => add(await ctx.platform.library.pickFolder("A folder of games to import"))}>Add a folder…</button>` : null}
      </div>
      <small class="muted">Every file in them is read and matched against your DATs.</small></div>
    <label class="field-block"><span class="field-label">System</span>
      <select id="import-system" class="compact" value=${only} onChange=${(e) => setOnly(e.target.value)}>
        <option value="">Every system (each game's own, found by its DAT)</option>
        ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <small class="muted">${only ? `Only ${nameOf(only)}'s games, and files no DAT knows are taken as ${nameOf(only)}'s.` : "Each system's games go to its own folder; you'll be asked about files no DAT knows."}</small></label>
    <div class="field-block" role="radiogroup" aria-label="How" id="import-mode">
      <label class="check-row"><input type="radio" name="import-mode" value="copy" checked=${mode === "copy"} onChange=${() => setMode("copy")} /> Copy them, hardlinked on the same drive (they take no space; the originals stay)</label>
      <label class="check-row"><input type="radio" name="import-mode" value="always" checked=${mode === "always"} onChange=${() => setMode("always")} /> Copy them, always (a full copy, even on the same drive)</label>
      <label class="check-row"><input type="radio" name="import-mode" value="move" checked=${mode === "move"} onChange=${() => setMode("move")} /> Move them into the library</label>
    </div>
  <//>`;
}

/** The preview of copying the games in; Run starts it. */
function ImportGamesPreview({ plan }) {
  const nothing = !plan.ops;
  return html`<${Dialog} title=${plan.title || "Import the games"} id="bring-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} submitLabel=${nothing ? "OK" : "Run"} cancelLabel=${nothing ? null : "Not now"}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to import: ${plan.left?.length ? "see what's left alone." : "the library has them all."}</p>`
      : html`<p class="muted">${plan.mode === "move" ? "Moved, then checked and scanned." : "Copied (or hardlinked, as chosen), then checked and scanned."}${plan.new_roots?.length ? ` ${plan.new_roots.length === 1 ? "A new folder becomes a folder with games" : "The new folders become folders with games"} for ${plan.new_roots.length === 1 ? "its system" : "their systems"}.` : ""} History can undo it.</p>`}
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
      <${Card} id="import-copy" icon="move" title="Games into the library" where="Matched against your DATs and copied or moved into each system's folder with games, named as the DAT says.">
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
