// PC games (docs/PLAN.md, "Phase 4 design", D25–D28): the games in the PC games folder,
// each in its own folder with its game.json; Import… reads the inbox (or files picked)
// into a review list, where each item can be identified, retitled or left out, then
// imports them as a change with a preview and Undo. A game's details, pictures and
// documents are on its page, as every game's are ("PC game page design", D55); the
// panel here, and that page's PC section (`PcParts`), set the program that starts it,
// unpack an installer or archive, and show its folder. Add to Steam… writes the games
// as non-Steam shortcuts with their artwork. Identify… and Add a game… (any other
// system) are here too, as dialogs the other pages open.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl, shortPath, baseName } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, SearchNote } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { Icon } from "./icons.js";
import { gameHash } from "./browse.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));

const STATUS = {
  ready: ["New", "A new game, identified"],
  look: ["Needs a look", "Nothing inside named the game: check its title, or Identify it"],
  have: ["Have it", "Every file is in the library already"],
  update: ["Goes into", "The same game is in the library: these files join it"],
};
const FOUND = { goggame: "its goggame file", innoextract: "innoextract (the installer's GOG id)", steam: "steam_appid.txt", name: "its name", search: "a search of the sources", you: "you" };
const PARTS = [["install", "Installer or archive"], ["extracted", "The game's files (ready to play)"], ["patch", "Patch"], ["dlc", "DLC"], ["extra", "Extra (manual, soundtrack…)"]];

/** A thumbnail of a game's artwork, or its initial. */
function Cover({ media, title, cls = "pc-cover" }) {
  const rel = media?.cover || media?.banner || media?.icon;
  const url = rel ? libraryUrl(rel) : null;
  const [broken, setBroken] = useState(false);
  return url && !broken ? html`<img class=${cls} src=${url} alt="" loading="lazy" onError=${() => setBroken(true)} />` : html`<span class=${`${cls} pc-cover-none`} aria-hidden="true">${(title || "?").slice(0, 1)}</span>`;
}

const yearOf = (v) => {
  const n = parseInt(String(v || "").trim(), 10);
  return n >= 1950 && n <= 2100 ? n : null;
};

// ------------------------------------------------------------ Identify

/** Identify…: `{ title, query, year, platform, onPick(result), back }`. A search across the
 *  sources (Settings › Sources) by title; a result picked is handed to onPick, which may
 *  return the dialog to show next. `back` is the dialog it goes back to (else it closes). */
export function IdentifyDialog({ title, query = "", year = null, platform = "pc", onPick, back = null }) {
  const leave = () => (back ? ui.set({ dialog: back }) : close());
  const [q, setQ] = useState(query);
  const [y, setY] = useState(year ? String(year) : "");
  const [r, setR] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const search = async () => {
    if (!q.trim()) return;
    setBusy(true);
    setError("");
    try {
      setR(await api("identify_search", { query: q.trim(), platform, year: yearOf(y) }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => { if (query) search(); }, []);
  const pick = async (x) => {
    setBusy(true);
    setError("");
    try {
      const next = await onPick(x);
      if (next) ui.set({ dialog: next });
      else leave();
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${title || "Identify"} id="identify-dialog" wide=${true} onSubmit=${search} busy=${busy || !q.trim()} error=${error} submitLabel=${busy ? "Searching…" : "Search"} cancelLabel=${back ? "Back" : "Close"} onCancel=${leave} onDismiss=${leave}>
    <div class="pick-row identify-query">
      <input type="search" id="identify-query" placeholder="The game's title" aria-label="The game's title" value=${q} onInput=${(e) => setQ(e.target.value)} />
      <input type="text" id="identify-year" class="year-box" inputmode="numeric" maxlength="4" placeholder="Year" aria-label="Year (optional)" value=${y} onInput=${(e) => setY(e.target.value)} />
    </div>
    ${r ? html`
      ${!r.results.length ? html`<p class="muted" id="identify-none">Nothing found. Try fewer words, or the title as the store writes it.</p>` : null}
      <ul class="ls-list identify-list" id="identify-results">${r.results.map((x) => html`<li key=${`${x.source}-${x.id}`} data-source=${x.source} data-id=${x.id}>
        ${x.cover ? html`<img class="identify-cover" src=${x.cover} alt="" loading="lazy" />` : html`<span class="identify-cover pc-cover-none" aria-hidden="true">${x.title.slice(0, 1)}</span>`}
        <span class="identify-label"><span><b>${x.title}</b>${x.year ? ` (${x.year})` : ""}</span>
          <span class="muted">${x.source_name}${x.platform && x.platform !== "pc" ? ` · ${x.platform}` : ""} · ${x.id}${x.url ? html` · <a href=${x.url} target="_blank" rel="noreferrer">page</a>` : null}</span></span>
        <button type="button" class="ghost" data-action="pick" disabled=${busy} onClick=${() => pick(x)}>This one</button>
      </li>`)}</ul>
      ${r.errors?.map((e) => html`<p class="warn-note identify-error" key=${e.source} data-source=${e.source}>${e.name || e.source}: ${e.error}</p>`)}
      ${r.needs?.length ? html`<p class="muted identify-needs" id="identify-needs" title=${r.needs.map((e) => `${e.name}: ${e.text}`).join("\n")}>${r.needs.length === 1 ? `${r.needs[0].name} isn't set up` : `${r.needs.length} sources aren't set up (${r.needs.map((e) => e.name).join(", ")})`}, so ${r.needs.length === 1 ? "it wasn't" : "they weren't"} asked. <a href=${routeHash("settings")} onClick=${() => { close(); setTimeout(() => document.getElementById("settings-sources")?.scrollIntoView(), 200); }}>Settings › Sources</a></p>` : null}` : html`<p class="muted">The sources are asked in the order Settings › Sources gives. Picking one keeps its id and details with the game and gets its artwork.</p>`}
  <//>`;
}

export const identifyDialog = (opts) => ui.set({ dialog: { type: "identify", ...opts } });

// ------------------------------------------------------------ Import…

/** Import…: reads `paths` (the inbox when empty) into items, which are reviewed here,
 *  then planned and run as a change. */
function ImportDialog({ paths, restore }) {
  const [items, setItemsHere] = useState(restore ? review.items : null);
  const setItems = (f) => setItemsHere((all) => (review.items = typeof f === "function" ? f(all) : f));
  const [progress, setProgress] = useState("");
  const [mode, setMode] = useState("copy");
  const [alwaysCopy, setAlwaysCopy] = useState(false);
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (restore && review.items) return;
    let gone = false;
    (async () => {
      try {
        const { job } = await api("pc_read", { paths: paths || [] });
        const done = await followJob(job, "Reading the inbox", (j) => {
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
  const set = (key, patch) => setItems((all) => all.map((it) => (it.key === key ? { ...it, ...patch } : it)));
  // Identify takes this dialog's place while it's open, then comes back to the list
  const identify = (it) => identifyDialog({
    title: `Identify ${it.name}`, query: it.title, year: it.year, platform: "pc", back: { type: "pc-import", paths, restore: true },
    onPick: async (x) => {
      const next = await api("pc_item_match", { item: it, source: x.source, id: x.id });
      review.items = review.items.map((o) => (o.key === it.key ? next : o));
    },
  });
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("pc_import_plan", { items, mode, always_copy: alwaysCopy }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const nothing = !plan.ops;
    return html`<${Dialog} title=${plan.title || plan.label} id="pc-import-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan).then(again); }} busy=${plan.short} submitLabel=${nothing ? "OK" : "Import"}
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each game gets its own folder in the PC games folder, with a game.json that says what it is. ${mode === "copy" ? "Your files stay where they are." : "The files are moved; Undo puts them back."} Artwork from the sources is fetched afterwards.</p>
    <//>`;
  }
  const included = (items || []).filter((i) => i.include).length;
  const games = (items || []).filter((i) => i.status !== "have");
  return html`<${Dialog} title="Import PC games" id="pc-import-dialog" wide=${true} onSubmit=${preview} busy=${busy || !items || !included} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!items ? html`<p class="muted" id="pc-import-reading">Reading what's there: each file is hashed and looked inside${progress ? html` (${progress})` : null}…</p>` : html`
      ${!items.length ? html`<p id="pc-import-empty">There's nothing to import${paths?.length ? "" : " in the inbox"}.</p>` : null}
      <ul class="ls-list pc-items" id="pc-items">${items.map((it) => {
        const [badge, why] = STATUS[it.status] || [it.status, ""];
        const where = it.status === "update" || it.status === "have" ? ` ${it.game_title || ""}` : "";
        return html`<li key=${it.key} class="pc-item" data-key=${it.key} data-status=${it.status} data-include=${it.include ? "yes" : "no"}>
          <label class="check-row pc-item-include"><input type="checkbox" data-action="include" checked=${it.include} disabled=${it.status === "have"} onChange=${(e) => set(it.key, { include: e.target.checked })} /></label>
          <div class="pc-item-main">
            <span class="pc-item-name" title=${it.files.map((f) => f.path).join("\n")}>${it.name}${it.files.length > 1 ? html` <span class="muted">(${plural(it.files.length, "file", "files")})</span>` : null}${it.patches.length ? html` <span class="muted">+ ${plural(it.patches.length, "patch", "patches")}</span>` : null}</span>
            <span class="pc-item-edit">
              <input type="text" class="compact pc-item-title" aria-label="Title" value=${it.title} disabled=${!it.include || it.status === "update"} onInput=${(e) => set(it.key, { title: e.target.value })} />
              <input type="text" class="compact year-box pc-item-year" inputmode="numeric" maxlength="4" aria-label="Year" placeholder="Year" value=${it.year || ""} disabled=${!it.include || it.status === "update"} onInput=${(e) => set(it.key, { year: yearOf(e.target.value) })} />
              <select class="compact pc-item-part" aria-label="What it is" value=${it.part} disabled=${!it.include} onChange=${(e) => set(it.key, { part: e.target.value })}>
                ${PARTS.map(([v, label]) => html`<option key=${v} value=${v}>${label}</option>`)}</select>
            </span>
            <span class="muted pc-item-meta"><span class=${`pc-badge pc-badge-${it.status}`} title=${why}>${badge}${where}</span>
              ${[size(it.files.reduce((n, f) => n + (f.size || 0), 0)), it.version ? `version ${it.version}` : null, Object.entries(it.ids || {}).map(([k, v]) => `${k} ${v}`).join(", ") || null, it.found ? `found by ${FOUND[it.found] || it.found}` : null].filter(Boolean).join(" · ")}</span>
            ${it.note ? html`<span class="muted">${it.note}</span>` : null}
          </div>
          ${it.status !== "have" ? html`<button type="button" class="ghost compact" data-action="identify" onClick=${() => identify(it)}>Identify…</button>` : null}
        </li>`;
      })}</ul>
      ${games.length ? html`<div class="field-block" role="radiogroup" aria-label="How">
        <label class="check-row"><input type="radio" name="pc-import-mode" value="copy" checked=${mode === "copy"} onChange=${() => setMode("copy")} /> Copy (hardlinked when it's on the same drive, so it takes no space)</label>
        <label class="check-row"><input type="radio" name="pc-import-mode" value="move" checked=${mode === "move"} onChange=${() => setMode("move")} /> Move them into the library</label>
        ${mode === "copy" ? html`<label class="check-row"><input type="checkbox" id="pc-import-copy" checked=${alwaysCopy} onChange=${(e) => setAlwaysCopy(e.target.checked)} /> <span title="Hardlinks are a second name for the same file on the same drive: they take no space">Always copy (no hardlinks)</span></label>` : null}
      </div>
      <p class="muted"><b>${plural(included, "item", "items")}</b> to import. Nothing changes until you've seen the preview.</p>` : null}`}
  <//>`;
}

/** The review list, kept while Identify has the dialog's place. */
const review = { items: null };

/** Start Import…: the inbox, or files or a folder picked. */
export async function importPc(how = "inbox") {
  let paths = [];
  if (how === "files") {
    paths = (await ctx.platform.library.pickFiles("The installers, archives or discs to import")) || [];
    if (!paths.length) return;
  } else if (how === "folder") {
    const p = await ctx.platform.library.pickFolder("A game's folder to import");
    if (!p) return;
    paths = [p];
  }
  review.items = null;
  ui.set({ dialog: { type: "pc-import", paths } });
}

// ------------------------------------------------------------ Steam

function SteamDialog() {
  const [info, setInfo] = useState(null);
  const [user, setUser] = useState("");
  const [pv, setPv] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const load = () => api("steam_info").then((i) => { setInfo(i); setUser((u) => u || i.users[0]?.id || ""); }).catch((e) => setError(err(e)));
  useEffect(() => { load(); }, []);
  useEffect(() => {
    if (!user) return;
    setPv(null);
    api("steam_preview", { user }).then(setPv).catch((e) => setError(err(e)));
  }, [user]);
  const write = async (cmd) => {
    setBusy(true);
    setError("");
    try {
      const r = await api(cmd, { user });
      close();
      toast(cmd === "steam_write" ? `Wrote ${plural(r.shortcuts, "shortcut", "shortcuts")} to Steam, with artwork for ${plural(r.art, "game", "games")}. Start Steam to see them.` : `Removed ${plural(r.removed, "shortcut", "shortcuts")} from Steam; yours are kept.`, 8000);
    } catch (e) {
      setError(err(e));
      load();
    } finally {
      setBusy(false);
    }
  };
  if (!info) return html`<${Dialog} title="Add to Steam" id="steam-dialog" onSubmit=${close} submitLabel="OK" error=${error}><p class="muted">Looking for Steam…</p><//>`;
  if (!info.steam) {
    return html`<${Dialog} title="Add to Steam" id="steam-dialog" onSubmit=${close} submitLabel="OK" error=${error}>
      <p id="steam-none">Steam isn't on this computer, or isn't where it usually is. Choose its folder in <a href=${routeHash("settings")} onClick=${close}>Settings › Steam</a>.</p><//>`;
  }
  const off = info.running || busy || !pv;
  const last = info.users.find((u) => u.id === user)?.last_written;
  return html`<${Dialog} title="Add to Steam" id="steam-dialog" wide=${true} onSubmit=${() => write("steam_write")} busy=${off || !(pv?.add.length || pv?.update.length || pv?.remove.length)} error=${error} submitLabel="Write the shortcuts">
    <p class="muted">Each PC game with a program becomes a non-Steam game in your Steam library, with its artwork in Steam's grid. Your own shortcuts are kept, and shortcuts.vdf is backed up first.</p>
    ${info.running ? html`<p class="warn-note" role="alert" id="steam-running">Steam is running. Close it first (Steam › Exit): it rewrites its shortcuts when it closes, so changes made now would be lost.</p>` : null}
    <label class="field-block"><span class="field-label">Steam user</span>
      <select id="steam-user" class="compact" value=${user} onChange=${(e) => setUser(e.target.value)}>
        ${info.users.map((u) => html`<option key=${u.id} value=${u.id}>${u.name || u.account || u.id}${u.recent ? " (signed in last)" : ""}</option>`)}
      </select></label>
    ${!info.users.length ? html`<p class="warn-note">No Steam user has signed in on this computer yet.</p>` : null}
    ${pv ? html`<div id="steam-preview">
      <p>${pv.add.length ? html`<b>${plural(pv.add.length, "shortcut", "shortcuts")}</b> to add` : "Nothing new to add"}${pv.update.length ? `, ${pv.update.length} there already (their artwork is refreshed)` : ""}${pv.remove.length ? `, ${pv.remove.length} to remove (games gone from the library)` : ""}. ${pv.others ? `${plural(pv.others, "shortcut of your own is", "shortcuts of your own are")} kept.` : ""}</p>
      <ul class="ls-list" id="steam-list">${[...pv.add.map((x) => ["add", x]), ...pv.update.map((x) => ["same", x]), ...pv.remove.map((x) => ["remove", x])].map(([k, x]) => html`<li key=${`${k}-${x.appid}`} data-op=${k}>
        <span class="identify-label"><span>${x.name}</span><span class="muted" title=${x.exe || ""}>${k === "remove" ? "removed" : `${baseName(x.exe || "")}${x.art?.length ? ` · ${x.art.join(", ")}` : " · no artwork yet"}`}</span></span></li>`)}</ul>
      ${pv.without_program.length ? html`<p class="muted" id="steam-without">Without a program to start, so left out: ${pv.without_program.join(", ")}. Set one in the game's panel.</p>` : null}
      ${last?.written ? html`<p class="muted">Last written ${last.written.slice(0, 16).replace("T", " ")}. <button type="button" class="linkish" id="steam-remove" disabled=${off} onClick=${() => write("steam_remove")}>Remove the app's shortcuts</button></p>` : null}
    </div>` : null}
  <//>`;
}

// ------------------------------------------------------------ Add a game…

/** Add a game… for any system but PC: files picked, the folder with games they go to,
 *  then the preview; once run and scanned, what each file turned out to be. */
function AddGameDialog({ platform: given, files: givenFiles }) {
  const platforms = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.id !== "pc").sort((a, b) => a.name.localeCompare(b.name));
  const [platform, setPlatform] = useState(given || "");
  const [files, setFiles] = useState(givenFiles || []);
  const [typed, setTyped] = useState("");
  const [places, setPlaces] = useState([]);
  const [into, setInto] = useState("");
  const [mode, setMode] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!platform) return;
    api("add_places", { platform }).then((p) => { setPlaces(p); setInto((i) => (p.some((x) => x.dir === i) ? i : p[0]?.dir || "")); }).catch(() => setPlaces([]));
  }, [platform]);
  const pickFiles = async () => {
    const f = await ctx.platform.library.pickFiles("The game's files (a ROM, an archive, or a disc's sheet or image)");
    if (f?.length) setFiles(f);
  };
  const pickInto = async () => {
    const p = await ctx.platform.library.pickFolder("The folder the game goes to");
    if (p) setInto(p);
  };
  const chosen = files.length ? files : typed.split("\n").map((x) => x.trim()).filter(Boolean);
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("add_plan", { files: chosen, platform, into, mode: mode === "move" ? "move" : "copy", always_copy: mode === "always" }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    const run = async () => {
      close();
      const r = await runPlan(plan);
      again();
      if (r?.added) ui.set({ dialog: { type: "add-results", added: r.added } });
    };
    return html`<${Dialog} title=${plan.title || plan.label} id="add-game-dialog" wide=${true} onSubmit=${run} busy=${plan.short || !plan.ops} submitLabel="Add" cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Once it's there the folder is scanned, and you'll see what its DAT says about it.</p>
    <//>`;
  }
  return html`<${Dialog} title="Import a game file" id="add-game-dialog" onSubmit=${preview} busy=${busy || !platform || !chosen.length || !into} error=${error} submitLabel=${busy ? "Reading…" : "Preview"}>
    <label class="field-block"><span class="field-label">System</span>
      <select id="add-platform" class="compact" value=${platform} onChange=${(e) => setPlatform(e.target.value)}>
        <option value="">Choose one</option>
        ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <small class="muted">PC games are imported on the <a href=${routeHash("pc")} onClick=${close}>PC games</a> page.</small></label>
    <div class="field-block"><span class="field-label">The game</span>
      ${isDesktop() ? html`<div class="pick-row"><span class="pick-path" id="add-files">${files.length ? files.join(", ") : "Nothing chosen"}</span>
        <button type="button" class="ghost" id="add-pick-files" onClick=${pickFiles}>Choose…</button></div>`
        : html`<textarea id="add-files-typed" rows="2" placeholder="Its file's path (one per line)" onInput=${(e) => setTyped(e.target.value)}>${typed}</textarea>`}
      <small class="muted">A ROM, a ZIP or 7z of one, or a disc (its cue sheet brings its tracks).</small></div>
    ${platform ? html`<div class="field-block"><span class="field-label">Into</span>
      <div class="pick-row">
        <select id="add-into" class="compact" value=${into} onChange=${(e) => setInto(e.target.value)}>
          ${places.map((p) => html`<option key=${p.dir} value=${p.dir} title=${p.why}>${p.dir}</option>`)}
          ${into && !places.some((p) => p.dir === into) ? html`<option value=${into}>${into}</option>` : null}
        </select>
        ${isDesktop() ? html`<button type="button" class="ghost" id="add-pick-into" onClick=${pickInto}>Another folder…</button>` : null}
      </div>
      ${!places.length ? html`<small class="muted">There's no folder with games for this system yet: the folder you choose becomes one.</small>` : null}</div>` : null}
    <div class="field-block" role="radiogroup" aria-label="How">
      <label class="check-row"><input type="radio" name="add-mode" value="copy" checked=${mode === "copy"} onChange=${() => setMode("copy")} /> Copy it, hardlinked on the same drive (the original stays)</label>
      <label class="check-row"><input type="radio" name="add-mode" value="always" checked=${mode === "always"} onChange=${() => setMode("always")} /> Copy it, always (a full copy, even on the same drive)</label>
      <label class="check-row"><input type="radio" name="add-mode" value="move" checked=${mode === "move"} onChange=${() => setMode("move")} /> Move it</label>
    </div>
  <//>`;
}

/** What the files Add a game… placed turned out to be. */
function AddResultsDialog({ added }) {
  const files = added.files;
  const identify = (f) => identifyDialog({
    title: `Identify ${f.file}`, query: f.file.replace(/\.[^.]+$/, "").replace(/\s*[([].*$/, ""), platform: added.platform, back: { type: "add-results", added: { ...added, files } },
    onPick: async (x) => {
      const identity = await api("identify_pick", { source: x.source, id: x.id, platform: added.platform, file: f });
      return { type: "add-results", added: { ...added, files: files.map((o) => (o.path === f.path ? { ...o, identity } : o)) } };
    },
  });
  return html`<${Dialog} title=${`Imported into ${added.platform_name}`} id="add-results-dialog" onSubmit=${close} submitLabel="Done" cancelLabel=${null}>
    <ul class="ls-list" id="add-results">${files.map((f) => html`<li key=${f.path} data-matched=${f.game ? "yes" : "no"} data-named=${f.named ? "yes" : "no"}>
      <span class="identify-label"><span><b>${f.file}</b></span>
        <span class="muted">${f.game ? (f.named ? `${f.game}, named as its DAT says.` : `${f.game} in its DAT. Organise… (Rename these, on the Audit page's Misnamed list) gives it the DAT's name.`)
          : f.identity ? `Identified: ${f.identity.title}${f.identity.year ? ` (${f.identity.year})` : ""}` : f.dat ? "No DAT knows this file." : "There's no DAT for this system to check it against."}</span></span>
      ${!f.game && f.sha1 ? html`<button type="button" class="ghost" data-action="identify" onClick=${() => identify(f)}>Identify…</button>` : null}
    </li>`)}</ul>
  <//>`;
}

export const addGameDialog = (opts = {}) => ui.set({ dialog: { type: "add-game", ...opts } });

export function PcDialogs({ dialog }) {
  if (dialog.type === "identify") return html`<${IdentifyDialog} key=${dialog.title} ...${dialog} />`;
  if (dialog.type === "pc-import") return html`<${ImportDialog} key=${dialog.restore ? "back" : "new"} ...${dialog} />`;
  if (dialog.type === "steam") return html`<${SteamDialog} />`;
  if (dialog.type === "add-game") return html`<${AddGameDialog} ...${dialog} />`;
  if (dialog.type === "add-results") return html`<${AddResultsDialog} key=${dialog.added.files.map((f) => f.identity?.title || "").join("|")} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ what only a PC game has

/** What only a PC game has (D55): the program that starts it, Unpack, its folder and its
 *  files. On its page's PC section and in this page's panel; `g` is the game as `pc_get`
 *  gives it, and `onChange` reads it again. */
export function PcParts({ g, off, onChange }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [program, setProgram] = useState(g.launch?.program || "");
  const [args, setArgs] = useState(g.launch?.args || "");
  useEffect(() => { setProgram(g.launch?.program || ""); setArgs(g.launch?.args || ""); }, [g.id, g.launch?.program, g.launch?.args]);
  const act = async (f, said) => {
    setBusy(true);
    setError("");
    try {
      const r = await f();
      await onChange();
      if (said) toast(typeof said === "function" ? said(r) : said, 6000);
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const save = (patch, said) => act(() => api("pc_update", { id: g.id, patch }), said);
  const unpack = (file) => act(async () => {
    const { job } = await api("pc_unpack", { id: g.id, file });
    const done = await followJob(job, `Unpacking ${g.title}`);
    if (done.error) throw new Error(done.error);
    return done.result;
  }, (r) => `Unpacked ${plural(r.files, "file", "files")}${r.launch ? `; it starts with ${r.launch.program}` : ""}.`);
  const removeUnpacked = () => act(() => api("pc_unpack_remove", { id: g.id }));
  const installers = g.files.filter((f) => f.kind === "install");
  return html`<div class="pc-parts" id="pc-parts" data-id=${g.id}>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    <section class="pc-section" id="pc-program-section"><h3>Program</h3>
      ${g.programs.length ? html`<form class="pc-program" id="pc-program-form" onSubmit=${(e) => { e.preventDefault(); save({ launch: program ? { program, args } : null }, "Saved the program."); }}>
        <select id="pc-program" class="compact" aria-label="The program that starts it" value=${program} disabled=${off} onChange=${(e) => setProgram(e.target.value)}>
          <option value="">None</option>
          ${[...new Set([...(program ? [program] : []), ...g.programs])].map((p) => html`<option key=${p} value=${p}>${p}</option>`)}
        </select>
        <input type="text" id="pc-args" class="compact" placeholder="Options (optional)" aria-label="Options" value=${args} disabled=${off || !program} onInput=${(e) => setArgs(e.target.value)} />
        <button type="submit" class="ghost compact" disabled=${off || busy}>Save</button>
      </form>` : html`<p class="muted" id="pc-no-program">${g.extracted ? "No program found in its files." : "It isn't unpacked, so there's no program to start yet."}</p>`}
      ${g.program ? html`<small class="muted" id="pc-program-path" title=${g.program}>${shortPath(g.program, 3)}</small>` : null}
    </section>
    ${installers.length && !g.extracted ? html`<section class="pc-section" id="pc-unpack-section"><h3>Unpack</h3>
      <p class="muted">Unpacks ${installers.length === 1 ? installers[0].path.split("/").pop() : "an installer or archive"} into the game's <code>extracted</code> folder, so it can be played without installing.${installers.some((f) => /\.exe$/i.test(f.path)) && !g.innoextract ? " GOG installers need innoextract (Settings › Tools)." : ""}</p>
      <div class="home-actions">${installers.slice(0, 4).map((f) => html`<button type="button" class="ghost compact" key=${f.path} data-action="unpack" disabled=${off || busy} onClick=${() => unpack(f.path)}>Unpack ${installers.length > 1 ? f.path.split("/").pop() : ""}</button>`)}</div>
    </section>` : null}
    ${g.unpacked ? html`<section class="pc-section" id="pc-unpacked-section"><h3>Unpacked</h3>
      <p class="muted">From ${g.unpacked.from.split("/").pop()}. <button type="button" class="linkish" id="pc-unpack-remove" disabled=${off || busy} onClick=${removeUnpacked}>Remove the unpacked copy</button> (the installer stays).</p></section>` : null}
    <section class="pc-section" id="pc-folder-section"><h3>Its folder</h3>
      <p class="pick-row"><code id="pc-folder-name" title=${g.dir}>${baseName(g.dir || g.folder)}</code>
        ${isDesktop() ? html`<button type="button" class="ghost compact" id="pc-reveal" onClick=${() => reveal(g.dir)}>${Icon.folder(14)} Show in folder</button>` : null}</p>
      <small class="muted">In the PC games folder, named by its title and year: a new title or year renames it.</small></section>
    <section class="pc-section pc-files-section"><h3>Files</h3>
      <ul class="ls-list pc-files" id="pc-files">${g.files.slice(0, 50).map((f) => html`<li key=${f.path}><span>${f.path}</span><span class="muted">${f.kind} · ${size(f.size)}</span></li>`)}</ul>
      ${g.files.length > 50 ? html`<p class="muted">… and ${g.files.length - 50} more.</p>` : null}
    </section>
  </div>`;
}

/** A PC game's one line under its title: where it's from, its version, size and ids. */
export const pcLine = (g) => [g.source && g.source !== "other" ? g.source.toUpperCase() : null, g.version ? `version ${g.version}` : null, size(g.size), Object.entries(g.ids || {}).map(([k, v]) => `${k} ${v}`).join(", ") || null].filter(Boolean).join(" · ");

// ------------------------------------------------------------ a game's panel

/** A game's panel on this page: its cover and title, the way to its page (its details,
 *  pictures and documents are there, D55), Identify…, and what only a PC game has. */
function GamePanel({ id, off, onChange, version }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const load = () => api("pc_get", { id }).then(setG).catch((e) => setError(err(e)));
  useEffect(() => { setError(""); load(); }, [id, version]);
  const identify = () => identifyDialog({
    title: `Identify ${g.title}`, query: g.title, year: g.year, platform: "pc",
    onPick: async (x) => { await api("identify_pick", { source: x.source, id: x.id, game: id, retitle: true }); onChange(); },
  });
  if (error && !g) return html`<aside class="pc-panel" id="pc-panel"><p class="warn-note" role="alert">${error}</p></aside>`;
  if (!g) return html`<aside class="pc-panel" id="pc-panel"><p class="muted">Reading…</p></aside>`;
  const d = g.details || {};
  return html`<aside class="pc-panel" id="pc-panel" data-id=${g.id}>
    ${g.media?.hero || g.media?.banner ? html`<img class="pc-hero" src=${libraryUrl(g.media.hero || g.media.banner)} alt="" />` : null}
    <div class="pc-panel-head">
      <${Cover} media=${g.media} title=${g.title} cls="pc-cover big" />
      <div><h2 id="pc-title">${g.title}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</h2>
        <p class="muted">${pcLine(g)}</p>
        ${d.developer || d.genres ? html`<p class="muted" id="pc-facts-line">${[d.developer, Array.isArray(d.genres) ? d.genres.join(", ") : d.genres].filter(Boolean).join(" · ")}</p>` : null}</div>
    </div>
    <div class="home-actions">
      <a class="primary compact lib-open" id="pc-open" href=${gameHash(`pc/${g.id}`)}>Open its page</a>
      <button type="button" class="ghost compact" id="pc-identify" disabled=${off} onClick=${identify}>${Icon.search(14)} Identify…</button>
    </div>
    <p class="muted">Its details, pictures and documents are on its page, where they're changed as any game's are.</p>
    <${PcParts} g=${g} off=${off} onChange=${async () => { await load(); onChange(); }} />
  </aside>`;
}

// ------------------------------------------------------------ the page

export function PcGames() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const [data, setData] = useState(null);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(null);
  const [tick, setTick] = useState(0);
  const off = !!s.library?.read_only;
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("pc_games").then((d) => { if (!gone) setData(d); }).catch((e) => { if (!gone) setData({ error: err(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, s.changes, tick]);
  const games = data?.games || [];
  const shown = q ? games.filter((g) => `${g.title} ${g.year || ""}`.toLowerCase().includes(q.toLowerCase())) : games;
  const menu = () => [
    { id: "pc-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importPc("files") },
    { id: "pc-import-folder", label: "Import a game's folder…", icon: "folder", disabled: off || !isDesktop(), run: () => importPc("folder") },
    { sep: true },
    { id: "pc-settings", label: "Settings › PC games", icon: "cog", run: () => { location.hash = "#/settings"; setTimeout(() => document.getElementById("settings-pc")?.scrollIntoView(), 200); } },
  ];
  const head = html`<${CommandBar} id="pc-head" title="PC games" count=${data?.games ? plural(games.length, "game", "games") : null}
    search=${games.length ? html`<input type="search" id="pc-search" class="compact" placeholder="Search the PC games" aria-label="Search the PC games" value=${q} onInput=${(e) => setQ(e.target.value)} />` : null}
    primary=${html`<span class="cmd-buttons">
      <button type="button" class="ghost compact" id="pc-steam" disabled=${!games.length} title="Your PC games as non-Steam games, with their artwork" onClick=${() => ui.set({ dialog: { type: "steam" } })}>${Icon.gamepad(15)} Add to Steam…</button>
      <button type="button" class="primary compact" id="pc-import" disabled=${off} title=${data?.inbox ? `Read the inbox (${data.inbox}${data.inbox_waiting ? `: ${plural(data.inbox_waiting, "thing", "things")} in it` : ""})` : "Choose what to import"} onClick=${() => importPc(data?.inbox ? "inbox" : "files")}>${Icon.inbox(15)} Import…</button></span>`}
    more=${{ id: "pc-menu", title: "Import files or a folder, settings", items: menu }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading the PC games…</p></div>`;
  if (data.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${data.error}</p></div>`;
  if (!games.length) {
    return html`${head}<div class="pages"><div class="library-settings" id="pc-empty">
      <p class="page-lead">${Icon.monitor(18)} Import installers, zipped games and game folders into one layout: each game in its own folder, with what it is and how it starts.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <p>Drop GOG installers, archives, disc images or game folders into an inbox folder and press Import…, or pick them.</p>
        <p class="muted">PC games go to <code id="pc-folder">${data.folder}</code>. ${data.inbox ? html`The inbox is <code id="pc-inbox">${data.inbox}</code>.` : html`Choose an inbox in <a href=${routeHash("settings")} onClick=${() => setTimeout(() => document.getElementById("settings-pc")?.scrollIntoView(), 200)}>Settings › PC games</a>.`}</p>
        <div class="home-actions"><button type="button" class="primary" id="pc-first" disabled=${off} onClick=${() => importPc(data.inbox ? "inbox" : "files")}>Import…</button></div>
      </section>
    </div></div>`;
  }
  const current = sel && games.some((g) => g.id === sel) ? sel : null;
  return html`${head}<div class="pages pc-page" id="pc-page">
    <div class="pc-body">
      <div class="pc-list-wrap">
        <${SearchNote} q=${q} found=${plural(shown.length, "game", "games")} onClear=${() => setQ("")} />
        <ul class="pc-list" id="pc-list" data-total=${games.length}>${shown.map((g) => html`<li key=${g.id}>
          <button type="button" class=${`pc-row${current === g.id ? " active" : ""}`} data-id=${g.id} aria-pressed=${current === g.id ? "true" : "false"} onClick=${() => setSel(g.id)}>
            <${Cover} media=${g.media} title=${g.title} />
            <span class="pc-row-main"><span class="pc-row-title">${g.title}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</span>
              <span class="muted pc-row-meta">${[g.source && g.source !== "other" ? g.source.toUpperCase() : null, size(g.size), g.launch ? null : "no program yet"].filter(Boolean).join(" · ")}</span></span>
          </button></li>`)}</ul>
      </div>
      ${current ? html`<${GamePanel} key=${current} id=${current} off=${off} version=${`${tick}-${s.changes}`} onChange=${() => setTick((t) => t + 1)} />` : html`<aside class="pc-panel pc-panel-empty"><p class="muted">Pick a game to see its details, set the program that starts it, or unpack it.</p></aside>`}
    </div>
  </div>`;
}

// ------------------------------------------------------------ Settings

/** Settings › PC games: where they go (kept in the library) and the inbox (this computer). */
export function PcSettings({ lib }) {
  const [d, setD] = useState(null);
  const [error, setError] = useState("");
  const [typed, setTyped] = useState({ folder: "", inbox: "" });
  const load = () => api("pc_games").then(setD).catch((e) => setError(err(e)));
  useEffect(() => { load(); }, [lib.path]);
  const set = async (what, path) => {
    setError("");
    try {
      await api(what === "folder" ? "pc_folder_set" : "pc_inbox_set", { path });
      await load();
    } catch (e) {
      setError(err(e));
    }
  };
  const pick = async (what) => {
    const p = await ctx.platform.library.pickFolder(what === "folder" ? "The folder PC games go to" : "The inbox: where installers and archives wait to be imported");
    if (p) set(what, p);
  };
  const off = !!lib.read_only;
  const row = (what, label, value, help) => html`<div class="field-block" id=${`settings-pc-${what}`}><span class="field-label">${label}</span>
    <div class="pick-row"><span class="pick-path" id=${`pc-${what}-path`}>${value || "Not set"}</span>
      ${isDesktop() ? html`<button type="button" class="ghost" data-action="choose" disabled=${off && what === "folder"} onClick=${() => pick(what)}>Choose…</button>` : html`<form class="pick-row" onSubmit=${(e) => { e.preventDefault(); set(what, typed[what]); }}>
        <input type="text" id=${`pc-${what}-typed`} class="compact" placeholder="A folder's path" aria-label=${label} value=${typed[what]} onInput=${(e) => setTyped({ ...typed, [what]: e.target.value })} />
        <button type="submit" class="ghost" id=${`pc-${what}-set`}>Set</button></form>`}
      ${what === "inbox" && value ? html`<button type="button" class="ghost" data-action="clear" onClick=${() => set(what, null)}>Clear</button>` : null}
    </div>
    <small class="muted">${help}</small></div>`;
  return html`<section class="ls-section" id="settings-pc">
    <h2>PC games</h2>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${d ? html`${row("folder", "PC games folder", d.folder, "Each game gets its own folder here: <Title> (<Year>) with install, extracted, patches, extras and dlc, and a game.json. Kept with the library.")}
      ${row("inbox", "Inbox", d.inbox, "Import… reads this folder: GOG installers (with their .bin parts and patches), ZIPs and 7z archives, disc images, game folders. Kept on this computer.")}` : null}
  </section>`;
}

/** A source's account or key: the fields it needs (secrets only said to be set), saved on
 *  this computer, and Check, which asks the source with them. */
function SourceKeys({ x, onSaved }) {
  const [typed, setTyped] = useState({});
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState(null);
  const save = async (clear = false) => {
    setBusy(true);
    setSaid(null);
    try {
      onSaved(await api("sources_keys_set", { source: x.id, keys: typed, clear }));
      setTyped({});
      if (!clear) {
        const r = await api("sources_check", { source: x.id }).then((t) => ({ ok: true, text: t }), (e) => ({ ok: false, text: err(e) }));
        setSaid(r);
      }
    } catch (e) {
      setSaid({ ok: false, text: err(e) });
    } finally {
      setBusy(false);
    }
  };
  const check = async () => {
    setBusy(true);
    setSaid(await api("sources_check", { source: x.id }).then((t) => ({ ok: true, text: t }), (e) => ({ ok: false, text: err(e) })));
    setBusy(false);
  };
  const anySet = x.keys.some((k) => k.set);
  return html`<form class="source-keys" data-source=${x.id} onSubmit=${(e) => { e.preventDefault(); save(); }}>
    ${x.keys.map((k) => html`<label class="field-inline" key=${k.field}><span>${k.label}</span>
      <input type=${k.secret ? "password" : "text"} class="compact" name=${k.field} autocomplete="off" aria-label=${`${x.name}: ${k.label}`}
        placeholder=${k.set ? (k.secret ? "Set (type to change)" : k.value) : "Not set"} value=${typed[k.field] ?? ""} onInput=${(e) => setTyped({ ...typed, [k.field]: e.target.value })} /></label>`)}
    <span class="pick-row">
      <button type="submit" class="ghost compact" data-action="save-keys" disabled=${busy || !Object.values(typed).some((v) => v.trim())}>Save</button>
      <button type="button" class="ghost compact" data-action="check" disabled=${busy || !anySet} onClick=${check}>Check</button>
      ${anySet ? html`<button type="button" class="linkish" data-action="clear-keys" disabled=${busy} onClick=${() => save(true)}>Forget them</button>` : null}
    </span>
    ${said ? html`<span class=${said.ok ? "muted source-said" : "warn-note source-said"} role="status">${said.text}</span>` : null}
  </form>`;
}

const SIGN_UP = {
  screenscraper: html`Your ScreenScraper account (free, at <a href="https://www.screenscraper.fr" target="_blank" rel="noreferrer">screenscraper.fr</a>), and a developer id and password for the app, which ScreenScraper gives on request (its forum); it's asked by the ROMs' hashes, so it knows games by their files.`,
  igdb: html`A Twitch developer application's client id and secret (free, at <a href="https://dev.twitch.tv/console/apps" target="_blank" rel="noreferrer">dev.twitch.tv</a>); IGDB is asked by title.`,
  thegamesdb: html`An API key (free, asked for on <a href="https://forums.thegamesdb.net" target="_blank" rel="noreferrer">TheGamesDB's forum</a>); it's asked by title.`,
};

/** Settings › Sources: the order Identify and Scrape ask them in, which are on, their
 *  accounts and keys, and LaunchBox's data. */
export function SourcesSettings() {
  const [list, setList] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => { api("sources").then(setList).catch((e) => setError(err(e))); }, []);
  const save = async (next) => {
    setList(next);
    try {
      setList(await api("sources_set", { sources: next.map((x) => ({ id: x.id, on: x.on })) }));
    } catch (e) {
      setError(err(e));
    }
  };
  const move = (i, by) => {
    const next = [...list];
    const [x] = next.splice(i, 1);
    next.splice(i + by, 0, x);
    save(next);
  };
  const getLaunchBox = async () => {
    setBusy(true);
    setError("");
    try {
      const { job } = await api("launchbox_fetch");
      const done = await followJob(job, "Getting LaunchBox's database");
      if (done.error) throw new Error(done.error);
      setList(await api("sources"));
      toast(`LaunchBox's database is on this computer: ${done.result.games.toLocaleString()} games.`, 6000);
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<section class="ls-section" id="settings-sources">
    <h2>Sources</h2>
    <p class="muted">Identify asks these, in this order, about a game nothing recognised; a scrape (the Metadata page) asks the ones that aren't for PC games alone. Accounts and keys are kept on this computer, never in the library, and none is built in.</p>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${list ? html`<ul class="ls-list change-list" id="sources">${list.map((x, i) => html`<li key=${x.id} data-source=${x.id} data-on=${x.on ? "yes" : "no"}>
      <span class="change-label"><label class="check-row source-name"><input type="checkbox" data-action="on" checked=${x.on} onChange=${(e) => save(list.map((o) => (o.id === x.id ? { ...o, on: e.target.checked } : o)))} />
        <b>${x.name}</b> <span class="muted">${x.pc_only ? "PC games" : "every system"}</span></label>
        ${x.keys ? html`<span class="muted source-help">${SIGN_UP[x.id] || null}</span><${SourceKeys} x=${x} onSaved=${setList} />` : null}
        ${x.id === "launchbox" ? html`<span class="muted" id="launchbox-status">${x.database?.have ? `Its database is on this computer: ${(x.database.games || 0).toLocaleString()} games, from ${(x.database.fetched || "").slice(0, 10)}.` : "Its database (about 100 MB) is searched on this computer, so get it first."}</span>` : null}</span>
      ${x.id === "launchbox" ? html`<button type="button" class="ghost compact" id="launchbox-get" disabled=${busy} onClick=${getLaunchBox}>${x.database?.have ? "Get it again" : "Get LaunchBox's database"}</button>` : null}
      <button type="button" class="ghost compact" data-action="up" aria-label=${`Ask ${x.name} sooner`} disabled=${i === 0} onClick=${() => move(i, -1)}>↑</button>
      <button type="button" class="ghost compact" data-action="down" aria-label=${`Ask ${x.name} later`} disabled=${i === list.length - 1} onClick=${() => move(i, 1)}>↓</button>
    </li>`)}</ul>` : null}
  </section>`;
}

/** Settings › Steam: its folder (found, or set by hand). */
export function SteamSettings() {
  const [info, setInfo] = useState(null);
  const [typed, setTyped] = useState("");
  const [error, setError] = useState("");
  const load = () => api("steam_info").then(setInfo).catch((e) => setError(err(e)));
  useEffect(() => { load(); }, []);
  const set = async (path) => {
    setError("");
    try {
      await api("steam_dir_set", { path });
      await load();
    } catch (e) {
      setError(err(e));
    }
  };
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder("Steam's folder (the one with steam.exe and userdata)");
    if (p) set(p);
  };
  return html`<section class="ls-section" id="settings-steam">
    <h2>Steam</h2>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${info ? html`<div class="field-block"><span class="field-label">Steam's folder</span>
      <div class="pick-row"><span class="pick-path" id="steam-path">${info.steam || "Not found"}</span>
        ${isDesktop() ? html`<button type="button" class="ghost" id="steam-choose" onClick=${pick}>Choose…</button>` : html`<form class="pick-row" onSubmit=${(e) => { e.preventDefault(); set(typed); }}>
          <input type="text" id="steam-typed" class="compact" placeholder="Steam's folder" aria-label="Steam's folder" value=${typed} onInput=${(e) => setTyped(e.target.value)} />
          <button type="submit" class="ghost" id="steam-set">Set</button></form>`}
        ${info.set ? html`<button type="button" class="ghost" id="steam-clear" onClick=${() => set(null)}>Clear</button>` : null}
      </div>
      <small class="muted">${info.steam ? `${plural(info.users.length, "user", "users")} signed in here. ` : ""}Add to Steam… (on the PC games page) writes your PC games as non-Steam shortcuts, with their artwork, while Steam is closed.</small></div>` : null}
  </section>`;
}
