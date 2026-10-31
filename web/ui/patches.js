// Patches (docs/PLAN.md, "Cheats and patches", D201–D203): one page under the Library with a
// tab for each kind, since each is put to use differently. ROM patches (translations, hacks,
// fixes, restorations) are kept with their game in its folder's patches/ (patches.json
// for each one's kind, title, author, version, language), brought in from files, folders
// or a download's ZIP (each paired with its game: a BPS's or UPS's own CRC32 against your
// files is sure, the name likely), and applied to a copy of the game that's a variant of it
// in the system's custom DAT, never changing the game's file. IPS, UPS, BPS, PPF and xdelta
// are read here; Test applies one in memory and says whether it fits. Texture packs (D204,
// web/ui/textures.js) are the second tab; PC mods (D205, web/ui/mods.js) the third.
// `#/patches/<key>` opens a game's patches above the list (a game's page links there).
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, libraryUrl } from "./library.js";
import { CommandBar } from "./layout.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { useRouteArg, argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, byName } from "./shelf.js";
import { TexturePacks } from "./textures.js";
import { PcMods } from "./mods.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
const KINDS = [["translation", "Translation"], ["hack", "Hack"], ["fix", "Fix"], ["restoration", "Restoration"], ["other", "Other"]];
const kindName = (k) => KINDS.find((x) => x[0] === k)?.[1] || "Other";
const TABS = [["rom", "ROM patches"], ["textures", "Texture packs"], ["mods", "PC mods"]];
const PAIRING = {
  sure: ["Sure", "The patch says it's for this very file (its CRC32)"], likely: ["Likely", "Paired by its name: check it"],
  you: ["You chose", "Added to the game you chose"], none: ["No game", "No game you have is named like it"],
};
const FORMATS = { ips: "IPS", ips32: "IPS32", ups: "UPS", bps: "BPS", ppf: "PPF", xdelta: "xdelta" };

/** Import patches… (files, ZIPs or a folder), each paired with a game, or all for `key`'s game. */
export async function importPatches({ folder = false, key = null } = {}) {
  const lib = ctx.platform.library;
  const paths = folder ? [await lib.pickFolder("A folder of patches")].filter(Boolean) : (await lib.pickFiles("Patches (.ips, .ups, .bps, .ppf, .xdelta) or ZIPs of them")) || [];
  if (!paths.length) return;
  try {
    const { job } = await api("patches_files", { paths, key });
    const done = await followJob(job, "Reading the patches");
    if (done.error) throw new Error(done.error);
    ui.set({ dialog: { type: "patches-found", found: done.result.found } });
  } catch (e) {
    toast(`Couldn't read them: ${err(e)}`, 10000);
  }
}

/** Apply…: the game's patches to apply, in order, and the copy's name; then the preview. */
export const applyDialog = (opts) => ui.set({ dialog: { type: "patches-apply", ...opts } });

/** What an import found: each patch with its game, how sure, its kind; Test, and Apply now. */
function FoundDialog({ found: given }) {
  const [found, setFound] = useState(() => given || []);
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const set = (i, patch) => setFound((all) => all.map((f, j) => (j === i ? { ...f, ...patch } : f)));
  const test = async (i) => {
    const f = found[i];
    set(i, { test: { testing: true } });
    try {
      set(i, { test: await api("patches_test", { key: f.key, path: f.path }) });
    } catch (e) {
      set(i, { test: { fits: false, why: err(e) } });
    }
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("patches_plan", { found }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const bring = async () => {
    close();
    const r = await runPlan(plan);
    if (!r) return again();
    // Apply now: each ticked one made into its patched copy
    for (const f of found.filter((x) => x.include && x.apply && x.key)) {
      try {
        const p = await api("patches_apply", { key: f.key, names: [f.name] });
        await runPlan(p);
      } catch (e) {
        toast(`Couldn't apply ${f.name}: ${err(e)}`, 10000);
      }
    }
    again();
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="patches-found-dialog" wide=${true} sticky=${true} onSubmit=${bring} busy=${plan.short || !plan.ops} submitLabel="Bring them in"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each game's patches go into its folder in the library (<code>patches/</code>), with their readmes.${found.some((f) => f.include && f.apply) ? " Those ticked Apply now are then made into patched copies, each a change of its own." : ""} History can undo it.</p>
    <//>`;
  }
  const n = found.filter((f) => f.include).length;
  const likely = found.filter((f) => f.pairing === "likely").length;
  return html`<${Dialog} title="Import patches" id="patches-found-dialog" wide=${true} sticky=${found.length > 0} onSubmit=${preview} busy=${busy || !n} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!found.length ? html`<p class="muted" id="patches-found-none">No patches there (IPS, UPS, BPS, PPF or xdelta).</p>` : html`
      ${likely ? html`<p class="warn-note" id="patches-found-likely">${plural(likely, "patch was", "patches were")} paired by name only: Test says whether it fits; tick the ones that are right.</p>` : null}
      <ul class="ls-list pc-items" id="patches-found">${found.map((f, i) => html`<li key=${`${f.path}|${i}`} class="pc-item" data-key=${f.key} data-pairing=${f.pairing} data-include=${f.include ? "yes" : "no"} data-format=${f.format}>
        <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Bring in ${f.name}`} checked=${f.include} disabled=${!f.key} onChange=${(e) => set(i, { include: e.target.checked })} /></label>
        <div class="pc-item-main">
          <span class="pc-item-name">${f.title || html`<span class="muted">No game</span>`} <span class="muted">${f.platform}</span>
            <span class="pc-badge" title=${PAIRING[f.pairing]?.[1]}>${PAIRING[f.pairing]?.[0] || f.pairing}</span></span>
          <span class="muted">${f.name} · ${FORMATS[f.format] || f.format}${f.details?.version ? ` · v${f.details.version}` : ""}${f.details?.author ? ` · ${f.details.author}` : ""}${f.from ? ` · from ${f.from}` : ""}${f.readme ? " · with its readme" : ""}</span>
          <span class="patch-found-row">
            <select class="compact" aria-label="Kind" data-action="kind" value=${f.details?.kind || "hack"} onChange=${(e) => set(i, { details: { ...f.details, kind: e.target.value } })}>${KINDS.map(([v, l]) => html`<option value=${v} key=${v}>${l}</option>`)}</select>
            <input type="text" class="compact" aria-label="Language" placeholder="Language" value=${f.details?.language || ""} onInput=${(e) => set(i, { details: { ...f.details, language: e.target.value } })} />
            <button type="button" class="ghost compact" data-action="test" disabled=${!f.key || f.needs} onClick=${() => test(i)}>Test</button>
            <label class="check-row"><input type="checkbox" data-action="apply" checked=${f.apply} disabled=${!f.key || !!f.needs} onChange=${(e) => set(i, { apply: e.target.checked })} /> Apply now</label>
          </span>
          ${f.needs ? html`<span class="warn-note">Made with xdelta3's own compression, which Game Repo can't read yet: it's kept, to apply with xdelta3.</span>` : null}
          ${f.test ? html`<span class=${f.test.fits ? "muted patch-test" : "warn-note patch-test"} data-fits=${f.test.testing ? "" : f.test.fits ? "yes" : "no"}>${f.test.testing ? "Testing…" : f.test.fits ? (f.test.how === "checked" ? "It fits: the patched file is the one the patch says it makes." : "It applies (IPS can't say more: try the copy).") : `It doesn't fit: ${f.test.why}`}</span>` : null}
        </div></li>`)}</ul>
      <p class="muted"><b>${plural(n, "patch", "patches")}</b> to bring in.</p>`}
  <//>`;
}

/** Apply…: which of the game's patches, in what order, and the copy's name. */
function ApplyDialog({ k, names: first = [], onDone }) {
  const [g, setG] = useState(null);
  const [picked, setPicked] = useState(first);
  const [name, setName] = useState("");
  const [named, setNamed] = useState(false);
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { api("patches_game", { key: k }).then(setG).catch((e) => setError(err(e))); }, [k]);
  useEffect(() => {
    if (named || !picked.length) return;
    api("patches_name", { key: k, names: picked }).then((r) => setName(r.name)).catch(() => {});
  }, [picked.join("|")]);
  const toggle = (n, on) => setPicked((p) => (on ? [...p.filter((x) => x !== n), n] : p.filter((x) => x !== n)));
  const move = (i, by) => setPicked((p) => { const l = [...p]; const [x] = l.splice(i, 1); l.splice(Math.max(0, Math.min(l.length, i + by)), 0, x); return l; });
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("patches_apply", { key: k, names: picked, name }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="patches-apply-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(() => { onDone?.(); again(); }); }} busy=${!plan.ops} submitLabel="Make it"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">The patched copy is written beside the game and added to the system's custom DAT as a variant of it, so it's in the Library under the game. The game's own file isn't changed. History can undo it.</p>
    <//>`;
  }
  const patches = g?.patches || [];
  return html`<${Dialog} title=${`Apply patches to ${g?.title || "the game"}`} id="patches-apply-dialog" wide=${true} onSubmit=${preview} busy=${busy || !picked.length || !name.trim()} error=${error} submitLabel=${busy ? "Applying in memory…" : "Preview"}>
    ${!g ? html`<p class="muted">Reading…</p>` : html`
      <p class="muted">Tick the patches to apply. Several are applied in the order below (a translation, then a fix for it).</p>
      <ul class="ls-list" id="patches-apply-list">${patches.map((p) => html`<li key=${p.name} data-name=${p.name}><label class="check-row"><input type="checkbox" data-action="pick" checked=${picked.includes(p.name)} onChange=${(e) => toggle(p.name, e.target.checked)} />
        ${p.details.title || p.name} <span class="muted">${kindName(p.details.kind)}${p.details.version ? ` · v${p.details.version}` : ""} · ${p.name}</span></label></li>`)}</ul>
      ${picked.length > 1 ? html`<ol class="ls-list" id="patches-apply-order">${picked.map((n, i) => html`<li key=${n}><span>${i + 1}. ${n}</span><span>
        <button type="button" class="ghost compact" aria-label="Earlier" disabled=${i === 0} onClick=${() => move(i, -1)}>↑</button>
        <button type="button" class="ghost compact" aria-label="Later" disabled=${i === picked.length - 1} onClick=${() => move(i, 1)}>↓</button></span></li>`)}</ol>` : null}
      <label class="field-block"><span class="field-label">The copy's name</span>
        <input type="text" id="patches-apply-name" value=${name} onInput=${(e) => { setName(e.target.value); setNamed(true); }} />
        <span class="muted">The game's name with the patches' tags, as the custom DAT keeps it.</span></label>`}
  <//>`;
}

/** Edit details…: a patch's kind, title, version, author, language, web address and notes. */
function EditDialog({ k, file, details, onDone }) {
  const [d, setD] = useState({ ...details });
  const [error, setError] = useState("");
  const f = (key, label, ph = "") => html`<label class="field-block"><span class="field-label">${label}</span><input type="text" data-field=${key} value=${d[key] || ""} placeholder=${ph} onInput=${(e) => setD({ ...d, [key]: e.target.value })} /></label>`;
  const save = () => api("patches_edit", { key: k, name: file, details: d }).then((g) => { close(); onDone?.(g); again(); }).catch((e) => setError(err(e)));
  return html`<${Dialog} title=${`Details of ${file}`} id="patches-edit-dialog" onSubmit=${save} error=${error} submitLabel="Save">
    <label class="field-block"><span class="field-label">Kind</span><select data-field="kind" value=${d.kind || "hack"} onChange=${(e) => setD({ ...d, kind: e.target.value })}>${KINDS.map(([v, l]) => html`<option value=${v} key=${v}>${l}</option>`)}</select></label>
    ${f("title", "Title", "Its name, as its authors give it")}
    ${f("version", "Version", "1.1")}
    ${f("author", "Author")}
    ${d.kind === "translation" ? f("language", "Language", "English") : null}
    ${f("url", "Web address", "Where it came from, to find it again")}
    <label class="field-block"><span class="field-label">Notes</span><textarea data-field="notes" rows="3" onInput=${(e) => setD({ ...d, notes: e.target.value })}>${d.notes || ""}</textarea></label>
  <//>`;
}

export function PatchDialogs({ dialog }) {
  if (dialog.type === "patches-found") return html`<${FoundDialog} ...${dialog} />`;
  if (dialog.type === "patches-apply") return html`<${ApplyDialog} ...${dialog} />`;
  if (dialog.type === "patches-edit") return html`<${EditDialog} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ one game's patches

const infoLine = (i) => {
  if (!i) return "Not a patch Game Repo can read";
  const crc = (c) => (c == null ? "" : c.toString(16).padStart(8, "0"));
  return [FORMATS[i.format] || i.format, i.source_size != null && i.source_crc != null ? `for a file of ${i.source_size} bytes, CRC32 ${crc(i.source_crc)}` : i.format === "ips" || i.format === "ips32" ? "says nothing of the file it's for" : null,
    i.needs ? "needs xdelta3" : null].filter(Boolean).join(" · ");
};

/** A game's patches, each with Apply…, Edit details… and Remove; a variant names its own. */
export function PatchesEditor({ k, off, focus = null }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const load = () => api("patches_game", { key: k }).then((x) => { setG(x); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { setG(null); load(); }, [k]);
  if (error) return html`<p class="warn-note">${error}</p>`;
  if (!g) return html`<p class="muted">Reading…</p>`;
  const list = g.patches || [];
  const done = (x) => { if (x) setG(x); else load(); };
  return html`<div class="patches-editor" id="patches-editor" data-key=${k}>
    ${g.patched ? html`<p class="muted" id="patches-made-with">Made from <a href=${gameHash(`${g.platform}/${g.patched.original}`)}>${g.patched.original}</a> with ${(g.patched.patches || []).map((p) => p.title || p.file).join(", then ")}.</p>` : null}
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" id="patches-add" disabled=${off || !isDesktop()} onClick=${() => importPatches({ key: k }).then(load)}>${Icon.plus(14)} Add a patch…</button>
      ${list.length > 1 ? html`<button type="button" class="ghost compact" id="patches-apply-several" disabled=${off} onClick=${() => applyDialog({ k, names: [], onDone: load })}>Apply several…</button>` : null}
    </div>
    ${!list.length && !g.patched ? html`<p class="muted" id="patches-none">${g.title} has no patches yet. Add a patch… brings one in: a translation, a hack or a fix (IPS, UPS, BPS, PPF or xdelta).</p>` : null}
    ${list.map((p) => html`<section class=${`patch-file${focus === p.name ? " on" : ""}`} key=${p.name} data-file=${p.name} data-kind=${p.details.kind}>
      <h3>${p.details.title || p.name}</h3>
      <p class="muted">${[kindName(p.details.kind), p.details.language, p.details.version ? `v${p.details.version}` : null, p.details.author ? `by ${p.details.author}` : null].filter(Boolean).join(" · ")}</p>
      <p class="muted">${p.name} · ${infoLine(p.info)}</p>
      ${p.applied?.length ? html`<p class="muted">Applied: ${p.applied.map((n) => html`<a key=${n} href=${gameHash(`${g.platform}/${n}`)}>${n}</a> `)}</p>` : html`<p class="muted">Not applied yet.</p>`}
      ${p.details.url ? html`<p class="muted"><a href=${p.details.url} target="_blank" rel="noopener">${p.details.url}</a></p>` : null}
      ${p.details.notes ? html`<p>${p.details.notes}</p>` : null}
      ${p.readme ? html`<details><summary>Its readme</summary><pre class="patch-readme">${p.readme}</pre></details>` : null}
      <div class="lib-panel-actions">
        <button type="button" class="primary compact" data-action="apply" disabled=${off || !!p.info?.needs || !p.info || !!g.patched} title=${g.patched ? "Apply patches to the original game" : ""} onClick=${() => applyDialog({ k, names: [p.name], onDone: load })}>Apply…</button>
        <button type="button" class="ghost compact" data-action="edit" disabled=${off} onClick=${() => ui.set({ dialog: { type: "patches-edit", k, file: p.name, details: p.details, onDone: done } })}>Edit details…</button>
        <button type="button" class="ghost compact" data-action="remove" disabled=${off} onClick=${() => confirmDialog({ title: `Remove ${p.name}?`, text: "It's kept aside in the game's patches folder (.removed). A patched copy made with it stays.", button: "Remove it", run: () => api("patches_remove", { key: k, name: p.name }).then((x) => { done(x); again(); }) })}>${Icon.trash(14)} Remove</button>
      </div>
    </section>`)}
  </div>`;
}

// ------------------------------------------------------------ the page

const pics = (p, px) => [thumb(p.picture, px)];
const noPic = (p) => ({ text: p.platform_name || p.platform, colour: systemColour(p.platform) });
const applied = (p) => (p.applied?.length ? "Applied" : "Not applied");
const SORTS = [
  ["title", "Patch", (a, b) => byName(a.title, b.title)],
  ["game", "Game", (a, b) => byName(a.game_title, b.game_title)],
  ["system", "System", (a, b) => byName(a.platform_name, b.platform_name)],
];
const GROUPS = [["", "None"], ["system", "System"], ["game", "Game"], ["kind", "Kind"]];
const COLUMNS = [
  ["title", "Patch", (p) => p.title, "title"],
  ["game", "Game", (p) => p.game_title, "game"],
  ["kind", "Kind", (p) => p.kind_name, null],
  ["language", "Language", (p) => p.language, null],
  ["version", "Version", (p) => p.version, null],
  ["applied", "Applied", applied, null],
];
const COLS = "minmax(140px, 2.4fr) minmax(140px, 2.4fr) 100px 100px 80px 100px";

export function Patches() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("patches", { sort: "title", view: "list", tab: "rom" });
  const opened = useRouteArg("patches");
  const tab = st.tab || "rom";
  useEffect(() => {
    if (!s.library || tab !== "rom") return;
    setData(null);
    api("patches_list", {}).then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path, tab]);
  const items = data?.items || [];
  const systems = useMemo(() => new Map(items.map((p) => [p.platform, p.platform_name])), [data]);
  const games = useMemo(() => new Map(items.map((p) => [p.game_key, p.game_title])), [data]);
  const fields = [
    { id: "system", name: "System", of: (p) => [p.platform], label: (v) => systems.get(v) || v },
    { id: "game", name: "Game", of: (p) => [p.game_key], label: (v) => games.get(v) || v },
    { id: "kind", name: "Kind", of: (p) => [p.kind], label: kindName },
    { id: "language", name: "Language", of: (p) => (p.language ? [p.language] : []), none: "No language" },
    { id: "applied", name: "Applied", of: (p) => [applied(p)] },
  ];
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    return [
      { id: "apply", label: "Apply…", icon: "check", one: true, disabled: off, run: () => one && applyDialog({ k: one.game_key, names: [one.file] }) },
      { id: "edit", label: "Edit details…", icon: "edit", one: true, disabled: off, run: () => one && api("patches_game", { key: one.game_key }).then((g) => ui.set({ dialog: { type: "patches-edit", k: one.game_key, file: one.file, details: g.patches.find((x) => x.name === one.file)?.details || {} } })) },
      { id: "game", label: "Its game", icon: "gamepad", one: true, run: () => one && (location.hash = gameHash(one.game_key)) },
    ];
  };
  const more = () => [
    { id: "patches-import-folder", label: "Import a folder of patches…", icon: "inbox", disabled: off || !isDesktop(), run: () => importPatches({ folder: true }) },
  ];
  const primary = html`<button type="button" class="primary compact" id="patches-import" disabled=${off || !isDesktop()} onClick=${() => importPatches()}>${Icon.inbox(15)} Import patches…</button>`;
  if (!s.library) return html`<${CommandBar} id="patches-head" title="Patches" /><div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const switches = html`<span class="seg" role="tablist" aria-label="Kind of patch" id="patches-tabs">${TABS.map(([v, label]) => html`<button type="button" role="tab" key=${v} class=${`ghost compact${tab === v ? " on" : ""}`} data-tab=${v} aria-selected=${tab === v ? "true" : "false"} onClick=${() => set({ tab: v })}>${label}</button>`)}</span>`;
  if (tab === "textures") return html`<${TexturePacks} switches=${switches} opened=${opened} off=${off} />`;
  if (tab === "mods") return html`<${PcMods} switches=${switches} opened=${opened} off=${off} />`;
  const top = opened ? html`<section class="cheats-top" id="patches-top" data-key=${opened}>
    <div class="video-player-head"><h2><a href=${gameHash(opened)}>${items.find((p) => p.game_key === opened)?.game_title || opened.split("/").slice(1).join("/")}</a></h2>
      <button type="button" class="ghost compact" id="patches-top-close" onClick=${() => { location.hash = routeHash("patches"); }}>${Icon.close(14)} Close</button></div>
    <${PatchesEditor} k=${opened} off=${off} />
  </section>` : null;
  const empty = html`<div class="library-settings" id="patches-empty">
    <p class="page-lead">Translations, hacks and fixes for your games, kept with each game in the library.</p>
    <p class="muted">Import patches… brings in patch files (IPS, UPS, BPS, PPF, xdelta) or the ZIPs they come in, each paired with its game; Apply… then makes a patched copy beside the game, a variant of it in the Library. The game's own file is never changed.</p>
    <div class="home-actions">${primary}</div>
    <div class="lib-switches">${switches}</div>
  </div>`;
  return html`<${Shelf} page="patches" noun=${["patch", "patches"]} title="Patches" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} columns=${COLUMNS} cols=${COLS} ratio="1"
    text=${(p) => [p.title, p.file, p.game_title, p.platform_name, p.author, p.language].join(" ")}
    tile=${(p) => ({ pics: pics(p, 512), none: noPic(p), badge: { text: p.kind_name, colour: systemColour(p.platform) }, title: p.title, meta: [p.game_title, applied(p)].join(" · ") })}
    line=${(p) => ({ pics: pics(p, 256), none: noPic(p), badge: { text: p.platform_name, colour: systemColour(p.platform) }, title: p.title, meta: [p.game_title, p.kind_name, p.language, p.version ? `v${p.version}` : null, applied(p)].filter(Boolean).join(" · ") })}
    actions=${actions} onOpen=${(p) => { location.hash = argHash("patches", p.game_key); }} switches=${switches} top=${top}
    primary=${items.length ? null : primary} more=${more} moreTitle="Import a folder of patches, tile size" empty=${top ? null : empty}
    panel=${(p) => html`<div class="shelf-panel" id="patches-panel" data-key=${p.game_key} data-file=${p.file}>
      <${Pic} srcs=${pics(p, 512)} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
      <h2 id="patches-panel-title"><a href=${gameHash(p.game_key)}>${p.game_title}</a></h2>
      <p class="muted">${p.platform_name}</p>
      <${PatchesEditor} k=${p.game_key} off=${off} focus=${p.file} />
    </div>`} />`;
}
