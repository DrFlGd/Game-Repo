// Texture packs (docs/PLAN.md, "Cheats and patches", D204): the Patches page's second tab.
// A pack is kept with its game in the library (patches/textures/), as it came (a ZIP or 7z,
// or a folder of its files), brought in hardlinked where it can so it's never kept twice.
// The emulator installs it: each emulator Game Repo knows says where in its folder it loads
// a game's pack (Dolphin Load/Textures/<game ID>/, PCSX2 textures/<serial>/replacements/…),
// and Install into an emulator… puts the pack's files there as a change History can undo,
// remembering the emulator's folder on this computer.
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, isDesktop, libraryUrl } from "./library.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, byName, askGame } from "./shelf.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
const size = (n) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round((n || 0) / 1e3))} KB`);
const installed = (p) => (p.installed?.emulator ? "Installed" : "Not installed");

/** Add a texture pack…: its ZIP or 7z (or its folder, `folder`) for `key`'s game, then the preview. */
export async function importTextures({ key, folder = false, onDone = null }) {
  const lib = ctx.platform.library;
  const paths = folder ? [await lib.pickFolder("The texture pack's folder")].filter(Boolean) : (await lib.pickFiles("Texture packs (.zip, .7z)")) || [];
  if (!paths.length) return;
  ui.set({ dialog: { type: "textures-import", k: key, paths, onDone } });
}

/** Bring them in: hardlinked or copied (the download stays), or moved in; then the preview. */
function ImportDialog({ k, paths, onDone }) {
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("textures_import", { key: k, paths, how }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="textures-import-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Bring it in"
      cancelLabel="Back" onCancel=${() => setPlan(null)} onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
      <${Preview} plan=${plan} />
      <p class="muted">The pack goes into the game's folder in the library (<code>patches/textures/</code>). History can undo it.</p>
    <//>`;
  }
  return html`<${Dialog} title=${paths.length === 1 ? "Add a texture pack" : `Add ${paths.length} texture packs`} id="textures-import-dialog" onSubmit=${preview} busy=${busy} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <ul class="ls-list" id="textures-import-paths">${paths.map((p) => html`<li key=${p}><code>${p}</code></li>`)}</ul>
    <fieldset class="field-block" id="textures-import-how"><legend class="field-label">How</legend>
      <label class="check-row"><input type="radio" name="textures-how" value="copy" checked=${how === "copy"} onChange=${() => setHow("copy")} /> Hardlinked on the same drive, else copied: the download stays where it is</label>
      <label class="check-row"><input type="radio" name="textures-how" value="move" checked=${how === "move"} onChange=${() => setHow("move")} /> Moved into the library</label>
    </fieldset>
  <//>`;
}

/** Install into an emulator…: which emulator, its folder on this computer, then the preview. */
function InstallDialog({ k, pack, emulators, onDone }) {
  const [emu, setEmu] = useState(emulators[0]?.id || "");
  const e = emulators.find((x) => x.id === emu) || {};
  const [folder, setFolder] = useState(e.folder || "");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const choose = async () => {
    const f = await ctx.platform.library.pickFolder(`${e.name}'s folder`);
    if (f) setFolder(f);
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("textures_install", { key: k, pack, emulator: emu, folder }));
    } catch (x) {
      setError(err(x));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="textures-install-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Install it"
      cancelLabel="Back" onCancel=${() => setPlan(null)} onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
      <${Preview} plan=${plan} />
      <p class="muted" id="textures-install-to">Into <code>${plan.to}</code>, hardlinked where it can. History can undo it.</p>
      ${e.setting ? html`<p class="muted">To see it, in ${e.name}: ${e.setting}.</p>` : null}
    <//>`;
  }
  return html`<${Dialog} title=${`Install ${pack}`} id="textures-install-dialog" onSubmit=${preview} busy=${busy || !emu || !folder} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <label class="field-block"><span class="field-label">The emulator</span>
      <select id="textures-install-emulator" class="compact" value=${emu} onChange=${(x) => { setEmu(x.target.value); setFolder(emulators.find((y) => y.id === x.target.value)?.folder || ""); }}>
        ${emulators.map((x) => html`<option key=${x.id} value=${x.id}>${x.name}</option>`)}</select></label>
    <label class="field-block"><span class="field-label">${e.name || "Its"} folder on this computer</span>
      <span class="field-row"><input type="text" id="textures-install-folder" value=${folder} placeholder="The folder with its settings (its user folder)" onInput=${(x) => setFolder(x.target.value)} />
      <button type="button" class="ghost" id="textures-install-choose" onClick=${choose}>Choose…</button></span></label>
    ${e.path ? html`<p class="muted">The pack goes in <code>${e.path}</code> there.${e.setting ? ` To see it, in ${e.name}: ${e.setting}.` : ""}</p>` : null}
  <//>`;
}

export function TextureDialogs({ dialog }) {
  if (dialog.type === "textures-import") return html`<${ImportDialog} ...${dialog} />`;
  if (dialog.type === "textures-install") return html`<${InstallDialog} ...${dialog} />`;
  return null;
}

/** A game's texture packs, each with Install into an emulator… and Remove. */
export function TexturesEditor({ k, off, focus = null }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const load = () => api("textures_packs", { key: k }).then((x) => { setG(x); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { setG(null); load(); }, [k]);
  if (error) return html`<p class="warn-note">${error}</p>`;
  if (!g) return html`<p class="muted">Reading…</p>`;
  const emus = g.emulators || [];
  const name = (id) => emus.find((x) => x.id === id)?.name || id;
  return html`<div class="patches-editor" id="textures-editor" data-key=${k}>
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" id="textures-add" disabled=${off || !isDesktop()} onClick=${() => importTextures({ key: k, onDone: load })}>${Icon.plus(14)} Add a pack…</button>
      <button type="button" class="ghost compact" id="textures-add-folder" disabled=${off || !isDesktop()} onClick=${() => importTextures({ key: k, folder: true, onDone: load })}>Add a pack's folder…</button>
    </div>
    <p class="muted" id="textures-emulators">${emus.length ? `Loaded by ${emus.map((x) => x.name).join(", ")}.` : `No emulator Game Repo knows loads texture packs for ${g.platform}: they can be kept here all the same.`}
      ${emus.some((x) => x.path.includes("{id")) && !g.game_id ? " Game Repo couldn't read this game's ID, which the emulator finds its pack by." : ""}</p>
    ${!g.packs?.length ? html`<p class="muted" id="textures-none">${g.title} has no texture packs yet.</p>` : null}
    ${(g.packs || []).map((p) => html`<section class=${`patch-file${focus === p.name ? " on" : ""}`} key=${p.name} data-file=${p.name}>
      <h3>${p.title || p.name}</h3>
      <p class="muted">${[p.name, p.archive ? "an archive" : "a folder", size(p.size), p.version ? `v${p.version}` : null, p.author ? `by ${p.author}` : null].filter(Boolean).join(" · ")}</p>
      <p class="muted" data-installed=${p.installed?.emulator ? "yes" : "no"}>${p.installed?.emulator ? html`Installed into ${name(p.installed.emulator)}: <code>${p.installed.to}</code>` : "Not installed here yet."}</p>
      <div class="lib-panel-actions">
        <button type="button" class="primary compact" data-action="install" disabled=${off || !emus.length || !isDesktop()} onClick=${() => ui.set({ dialog: { type: "textures-install", k, pack: p.name, emulators: emus, onDone: load } })}>Install into an emulator…</button>
        <button type="button" class="ghost compact" data-action="remove" disabled=${off} onClick=${() => confirmDialog({ title: `Remove ${p.name}?`, text: "It's kept aside in the game's patches/textures folder (.removed). What's installed in an emulator stays: History undoes that.", button: "Remove it", run: () => api("textures_remove", { key: k, pack: p.name }).then((x) => { setG({ ...g, ...x }); again(); }) })}>${Icon.trash(14)} Remove</button>
      </div>
    </section>`)}
  </div>`;
}

const pics = (p, px) => [thumb(p.picture, px)];
const noPic = (p) => ({ text: p.platform_name || p.platform, colour: systemColour(p.platform) });
const SORTS = [
  ["title", "Pack", (a, b) => byName(a.title, b.title)],
  ["game", "Game", (a, b) => byName(a.game_title, b.game_title)],
  ["system", "System", (a, b) => byName(a.platform_name, b.platform_name)],
  ["size", "Size", (a, b) => (b.size || 0) - (a.size || 0)],
];
const GROUPS = [["", "None"], ["system", "System"], ["game", "Game"]];
const COLUMNS = [
  ["title", "Pack", (p) => p.title, "title"],
  ["game", "Game", (p) => p.game_title, "game"],
  ["size", "Size", (p) => size(p.size), "size"],
  ["installed", "Installed", installed, null],
];
const COLS = "minmax(140px, 2.4fr) minmax(140px, 2.4fr) 90px 110px";

/** The Texture packs tab: every game's packs, a game's opened above them (`opened`). */
export function TexturePacks({ switches, opened, off }) {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("textures", { sort: "title", view: "list" });
  useEffect(() => {
    if (!s.library) return;
    setData(null);
    api("textures_list", {}).then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  const items = data?.items || [];
  const systems = useMemo(() => new Map(items.map((p) => [p.platform, p.platform_name])), [data]);
  const games = useMemo(() => new Map(items.map((p) => [p.game_key, p.game_title])), [data]);
  const fields = [
    { id: "system", name: "System", of: (p) => [p.platform], label: (v) => systems.get(v) || v },
    { id: "game", name: "Game", of: (p) => [p.game_key], label: (v) => games.get(v) || v },
    { id: "installed", name: "Installed", of: (p) => [installed(p)] },
  ];
  const add = (folder) => askGame({
    title: "Add a texture pack", button: folder ? "Choose its folder…" : "Choose the pack…", note: "The game the pack is for.",
    run: async (g) => { if (g) { close(); await importTextures({ key: g.key, folder }); } },
  });
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    return [
      { id: "open", label: "Install or remove…", icon: "check", one: true, run: () => one && (location.hash = argHash("patches", one.game_key)) },
      { id: "game", label: "Its game", icon: "gamepad", one: true, run: () => one && (location.hash = gameHash(one.game_key)) },
    ];
  };
  const more = () => [
    { id: "textures-add-folder-page", label: "Add a pack's folder…", icon: "inbox", disabled: off || !isDesktop(), run: () => add(true) },
  ];
  const primary = html`<button type="button" class="primary compact" id="textures-import" disabled=${off || !isDesktop()} onClick=${() => add(false)}>${Icon.inbox(15)} Add a texture pack…</button>`;
  const top = opened ? html`<section class="cheats-top" id="patches-top" data-key=${opened}>
    <div class="video-player-head"><h2><a href=${gameHash(opened)}>${items.find((p) => p.game_key === opened)?.game_title || opened.split("/").slice(1).join("/")}</a></h2>
      <button type="button" class="ghost compact" id="patches-top-close" onClick=${() => { location.hash = routeHash("patches"); }}>${Icon.close(14)} Close</button></div>
    <${TexturesEditor} k=${opened} off=${off} />
  </section>` : null;
  const empty = html`<div class="library-settings" id="textures-empty">
    <p class="page-lead">Texture packs for your games, kept with each game in the library and installed where its emulator loads them.</p>
    <p class="muted">Add a texture pack… brings in a pack (its ZIP or 7z, or its folder) for a game; Install into an emulator… puts its files in the emulator's own folder for them (Dolphin, PCSX2, PPSSPP, DuckStation, Azahar, Cemu, Eden, RetroArch for NES and N64), hardlinked where it can.</p>
    <div class="home-actions">${primary}</div>
    <div class="lib-switches">${switches}</div>
  </div>`;
  return html`<${Shelf} page="textures" noun=${["texture pack", "texture packs"]} title="Patches" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} columns=${COLUMNS} cols=${COLS} ratio="1"
    text=${(p) => [p.title, p.file, p.game_title, p.platform_name, p.author].join(" ")}
    tile=${(p) => ({ pics: pics(p, 512), none: noPic(p), badge: { text: p.platform_name, colour: systemColour(p.platform) }, title: p.title, meta: [p.game_title, installed(p)].join(" · ") })}
    line=${(p) => ({ pics: pics(p, 256), none: noPic(p), badge: { text: p.platform_name, colour: systemColour(p.platform) }, title: p.title, meta: [p.game_title, size(p.size), installed(p)].join(" · ") })}
    actions=${actions} onOpen=${(p) => { location.hash = argHash("patches", p.game_key); }} switches=${switches} top=${top}
    primary=${items.length ? null : primary} more=${more} moreTitle="Add a pack's folder, tile size" empty=${top ? null : empty}
    panel=${(p) => html`<div class="shelf-panel" id="textures-panel" data-key=${p.game_key} data-file=${p.file}>
      <${Pic} srcs=${pics(p, 512)} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
      <h2 id="textures-panel-title"><a href=${gameHash(p.game_key)}>${p.game_title}</a></h2>
      <p class="muted">${p.platform_name}</p>
      <${TexturesEditor} k=${p.game_key} off=${off} focus=${p.file} />
    </div>`} />`;
}

