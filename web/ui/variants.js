// Variants (docs/PLAN.md, "Phase 10 design", D119–D123): the files no DAT knows, on Audit's
// Unknown tab with the game each is likely a variant of (a translation, a hack, a
// prototype); added to the system's custom DAT as variants of it, or as games of their own;
// the Variants tab, which lists what's in it; and the dialog that names one, gives it tags
// and chooses its original. Every change writes the custom DAT again and has Undo.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { api, toast, isDesktop, plural, shortPath } from "./library.js";
import { reveal } from "./games.js";
import { Dialog, close } from "./dialogs.js";
import { gameHash } from "./browse.js";
import { openMenu } from "./actions.js";
import { Icon } from "./icons.js";
import { renameMisnamed } from "./organise.js";
import { ctx } from "./context.js";
import { argHash } from "./panels.js";

const err = (e) => e.message || String(e);
const changed = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const HOW = { same: "same title", close: "close name" };
const STATUS = { have: "Have", partial: "Partly there", missing: "Missing" };

/** A file's name without its extension, as the core takes it. */
export const stem = (file) => file.replace(/(.)\.[A-Za-z0-9]{1,4}$/, "$1");

const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/** A variant's name from its title and tags: as it was written while they're the same,
 *  otherwise each tag in parentheses of its own (D121). `was` is { name, title, tags }. */
export function compose(title, tags, was) {
  const t = title.trim();
  if (was?.name && t === was.title && same(tags, was.tags || [])) return was.name;
  return [t, ...tags.map((x) => x.trim()).filter(Boolean).map((x) => `(${x})`)].join(" ");
}

/** Put the custom DAT back as it was before its last change. */
export async function undoVariants(platform) {
  try {
    await api("variants_undo", { platform });
    changed();
    toast("Put the custom DAT back as it was.");
  } catch (e) { toast(`Couldn't undo it: ${err(e)}`, 8000); }
}

const undoAction = (platform) => [{ label: "Undo", run: () => undoVariants(platform) }];

async function add(platform, adds) {
  const r = await api("variants_add", { platform, adds });
  changed();
  const what = r.added.length === 1 ? r.added[0] : plural(r.added.length, "game", "games");
  toast(`Added ${what} to ${r.name}.`, { ms: 10000, actions: undoAction(platform) });
  return r;
}

/** Open Audit's Unknown tab for a system (after an import brought in files no DAT knows). */
export function reviewUnknown(platform) {
  const p = ctx.platform?.store?.prefs;
  p?.set("gr-audit", { ...(p.get("gr-audit", {}) || {}), platform, tab: "unknown" });
  location.hash = argHash("audit", platform);
}

/** Open the variant dialog: for files on Unknown (`files`, their ids) or a variant (`editing`, its name). */
export function variantDialog(o) { ui.set({ dialog: { type: "variant", ...o } }); }

const Tags = ({ tags }) => (tags?.length ? html`<span class="variant-tags">${tags.map((t) => html`<span class="variant-tag" key=${t}>${t}</span>`)}</span>` : null);

/** Audit's Unknown tab for a system with a DAT (not arcade): each file with the game it's
 *  likely a variant of, its tags and its folder; ticked ones are added with their suggestions. */
export function UnknownTable({ items, platform, off }) {
  const [ticked, setTicked] = useState(() => new Set());
  const [chosen, setChosen] = useState({});
  const [busy, setBusy] = useState(false);
  useEffect(() => { setTicked(new Set()); setChosen({}); }, [items]);
  const original = (it) => chosen[it.id] ?? it.suggested?.[0]?.name ?? null;
  const flip = (id) => setTicked((t) => { const n = new Set(t); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const run = async (adds) => {
    setBusy(true);
    try { await add(platform, adds); } catch (e) { toast(`Couldn't add it: ${err(e)}`, 8000); } finally { setBusy(false); }
  };
  const one = (it, o) => run([{ files: [it.id], name: stem(it.name), original: o }]);
  const was = (it) => ({ name: stem(it.name), title: it.title || stem(it.name), tags: it.tags || [] });
  const edit = (it, find = false, files = [it.id]) => variantDialog({ platform, files, was: was(it), suggested: it.suggested || [], original: find ? null : original(it), find });
  const picked = items.filter((it) => ticked.has(it.id));
  const withOne = picked.filter((it) => original(it));
  const addTicked = () => {
    if (withOne.length) run(withOne.map((it) => ({ files: [it.id], name: stem(it.name), original: original(it) })));
    if (withOne.length < picked.length) toast(`${plural(picked.length - withOne.length, "ticked file has", "ticked files have")} no game suggested: Edit… ${picked.length - withOne.length === 1 ? "it" : "each"} to choose one.`, 8000);
  };
  // a game of several files (a cue sheet and its tracks): named for its sheet or first file
  const asOne = () => {
    const main = picked.find((it) => /\.(cue|gdi|m3u|ccd)$/i.test(it.name)) || picked[0];
    edit(main, false, [main.id, ...picked.filter((it) => it !== main).map((it) => it.id)]);
  };
  const menu = (e, it) => {
    e.preventDefault();
    openMenu(e, [
      ...(it.suggested || []).map((s) => ({ id: "unknown-add-as", label: `Add as a variant of ${s.name}`, icon: "plus", disabled: off, run: () => one(it, s.name) })),
      { id: "unknown-choose", label: "Choose the game…", icon: "search", disabled: off, run: () => edit(it, true) },
      { id: "unknown-own", label: "Add as a game of its own", icon: "plus", disabled: off, run: () => one(it, null) },
      { id: "unknown-edit", label: "Edit its name and tags…", icon: "edit", disabled: off, run: () => edit(it) },
      ...(it.path && isDesktop() ? [{ id: "audit-reveal", label: "Show in folder", icon: "folder", run: () => reveal(it.path) }] : []),
    ]);
  };
  const all = items.length > 0 && items.every((it) => ticked.has(it.id));
  return html`<div class="unknown-wrap">
    ${off ? null : html`<div class="unknown-bar" id="unknown-bar">
      <span class="muted">${picked.length ? `${plural(picked.length, "file", "files")} ticked` : "Tick files to add them together."}</span>
      <button type="button" class="ghost compact" id="unknown-add-ticked" disabled=${busy || !withOne.length} title="Each as a variant of the game suggested for it, under its own name"
        onClick=${addTicked}>${Icon.plus(14)} Add the ticked as variants${withOne.length ? ` (${withOne.length})` : ""}</button>
      ${picked.length > 1 ? html`<button type="button" class="ghost compact" id="unknown-add-one" disabled=${busy} title="One game of several files, such as a cue sheet and its tracks" onClick=${asOne}>Add as one game…</button>` : null}
    </div>`}
    <table class="audit-table" id="audit-list-table"><thead><tr>
      ${off ? null : html`<th class="tick"><input type="checkbox" aria-label="Tick every file" checked=${all} onChange=${() => setTicked(all ? new Set() : new Set(items.map((it) => it.id)))} /></th>`}
      <th>File</th><th>Likely a variant of</th><th>Tags</th><th>Folder</th><th></th></tr></thead>
    <tbody>${items.map((it) => {
      const s = it.suggested || [];
      const o = original(it);
      return html`<tr class="audit-row" key=${it.id} data-name=${it.name} tabindex="0" onContextMenu=${(e) => menu(e, it)}>
        ${off ? null : html`<td class="tick"><input type="checkbox" aria-label=${`Tick ${it.name}`} checked=${ticked.has(it.id)} onChange=${() => flip(it.id)} /></td>`}
        <td class="audit-name">${it.name}</td>
        <td class="unknown-original">${s.length > 1 ? html`<select class="compact" aria-label=${`The game ${it.name} is a variant of`} value=${o} onChange=${(e) => setChosen({ ...chosen, [it.id]: e.target.value })}>
            ${s.map((x) => html`<option key=${x.name} value=${x.name}>${x.name} (${HOW[x.how] || x.how})</option>`)}</select>`
          : s.length ? html`<span>${s[0].name}</span> <span class="muted">${HOW[s[0].how] || s[0].how}</span>` : html`<span class="muted">No game found</span>`}</td>
        <td><${Tags} tags=${it.tags} /></td>
        <td class="audit-path muted" title=${it.shown || null}>${shortPath(it.shown, 3)}</td>
        <td class="audit-acts">
          ${off ? null : s.length ? html`<button type="button" class="ghost compact unknown-add" disabled=${busy} title=${`Add it to the custom DAT as a variant of ${o}`} onClick=${() => one(it, o)}>Add</button>`
            : html`<button type="button" class="ghost compact unknown-choose" title="Choose the game it's a variant of, or add it as a game of its own" onClick=${() => edit(it, true)}>Choose…</button>`}
          ${off ? null : html`<button type="button" class="ghost compact unknown-edit" title="Its name, tags and the game it's a variant of" onClick=${() => edit(it)}>Edit…</button>`}
          ${it.path && isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label=${`Show ${it.name} in its folder`} onClick=${() => reveal(it.path)}>${Icon.folder(14)}</button>` : null}
        </td></tr>`;
    })}</tbody></table>
  </div>`;
}

/** Audit's Variants tab: the system's custom DAT, each game with its tags, its original and
 *  whether its file is there; Edit…, Remove and Undo. */
export function VariantsList({ platform, platformName, q, off }) {
  const changes = useStore(ui, (st) => st.changes);
  const [list, setList] = useState(null);
  useEffect(() => {
    let gone = false;
    api("variants_list", { platform }).then((r) => { if (!gone) setList(r); }).catch((e) => { if (!gone) setList({ items: [], error: err(e) }); });
    return () => { gone = true; };
  }, [platform, changes]);
  if (!list) return html`<p class="muted">Reading the variants…</p>`;
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const items = list.items.filter((it) => words.every((w) => it.name.toLowerCase().includes(w) || (it.original || "").toLowerCase().includes(w)));
  const misnamed = list.items.filter((it) => it.misnamed && it.path).length;
  const remove = async (names) => {
    try {
      await api("variants_remove", { platform, names });
      changed();
      toast(`Took ${names.length === 1 ? names[0] : plural(names.length, "variant", "variants")} out of ${list.name}: ${names.length === 1 ? "its file is" : "their files are"} on Unknown again.`, { ms: 10000, actions: undoAction(platform) });
    } catch (e) { toast(`Couldn't take it out: ${err(e)}`, 8000); }
  };
  const edit = (it) => variantDialog({ platform, editing: it.name, was: { name: it.name, title: it.title, tags: it.tags }, original: it.original, suggested: [] });
  const page = (name) => html`<a class="audit-open" href=${gameHash(`${platform}/${name}`)} title=${`Open ${name}'s page`}>${name}</a>`;
  const menu = (e, it) => {
    e.preventDefault();
    openMenu(e, [
      { id: "variant-open", label: "Open its page", icon: "external", run: () => { location.hash = gameHash(`${platform}/${it.name}`); } },
      ...(off ? [] : [{ id: "variant-edit", label: "Edit…", icon: "edit", run: () => edit(it) }, { id: "variant-remove", label: "Remove from the custom DAT", icon: "trash", run: () => remove([it.name]) }]),
      ...(it.path && isDesktop() ? [{ id: "audit-reveal", label: "Show in folder", icon: "folder", run: () => reveal(it.path) }] : []),
    ]);
  };
  return html`<div class="variants-wrap" id="variants">
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${off ? null : html`<div class="unknown-bar">
      <span class="muted">${list.dat ? `${list.name}: ${plural(list.items.length, "game", "games")}.` : "Variants you add from Unknown are kept in the system's custom DAT."}</span>
      ${misnamed ? html`<button type="button" class="ghost compact" id="variants-rename" title="Rename their files to their names in the custom DAT, as one change with Undo" onClick=${() => renameMisnamed(list.dat, platformName)}>Rename ${plural(misnamed, "file", "files")} to their names…</button>` : null}
      ${list.can_undo ? html`<button type="button" class="ghost compact" id="variants-undo" title="Put the custom DAT back as it was before its last change" onClick=${() => undoVariants(platform)}>Undo the last change</button>` : null}
    </div>`}
    ${!items.length ? html`<p class="muted" id="audit-none">${q ? "Nothing here matches the search." : "No variants yet. Files no DAT knows are on the Unknown tab, each with the game it's likely a variant of."}</p>`
      : html`<table class="audit-table" id="variants-table"><thead><tr><th>Name</th><th>Tags</th><th>Variant of</th><th>Status</th><th>Folder</th><th></th></tr></thead>
      <tbody>${items.map((it) => html`<tr class="audit-row" key=${it.id} data-name=${it.name} tabindex="0" onContextMenu=${(e) => menu(e, it)}>
        <td class="audit-name">${page(it.name)}${it.misnamed && it.path ? html` <span class="muted">its file is named otherwise</span>` : null}</td>
        <td><${Tags} tags=${it.tags} /></td>
        <td>${it.original ? page(it.original) : html`<span class="muted">A game of its own</span>`}</td>
        <td>${STATUS[it.status] || it.status}</td>
        <td class="audit-path muted" title=${it.shown || null}>${shortPath(it.shown, 3)}</td>
        <td class="audit-acts">${off ? null : html`<button type="button" class="ghost compact variant-edit" onClick=${() => edit(it)}>Edit…</button>
          <button type="button" class="ghost compact variant-remove" title="Take it out of the custom DAT: its file goes back on Unknown" onClick=${() => remove([it.name])}>Remove</button>`}</td>
      </tr>`)}</tbody></table>`}
  </div>`;
}

/** A variant's name (its title and tags) and the game it's a variant of: for a file before
 *  it's added (`files`), or a variant already in the custom DAT (`editing`, its name). */
function VariantDialog({ platform, files, editing, was, suggested = [], original = null, find = false, onDone }) {
  const [title, setTitle] = useState(was.title || "");
  const [tags, setTags] = useState(was.tags || []);
  const [to, setTo] = useState(original || "");
  const [search, setSearch] = useState(find ? was.title || "" : "");
  const [found, setFound] = useState([]);
  const [list, setList] = useState([]);
  const [typing, setTyping] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { api("variants_tags").then(setList).catch(() => {}); }, []);
  useEffect(() => {
    if (!search.trim()) { setFound([]); return; }
    let gone = false;
    const t = setTimeout(() => api("variants_search", { platform, text: search })
      .then((r) => { if (!gone) { setFound(r); if (find && !to && r.length) setTo(r[0]); } }).catch((e) => { if (!gone) setError(err(e)); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [search]);
  const name = compose(title, tags, was);
  const has = (t) => tags.some((x) => x.toLowerCase() === t.toLowerCase());
  const addTag = (t) => { const v = t.trim(); if (v && !has(v)) setTags([...tags, v]); };
  const newTag = async () => {
    const t = (typing || "").trim();
    if (!t) { setTyping(null); return; }
    try {
      if (!list.some((x) => x.toLowerCase() === t.toLowerCase())) setList(await api("variants_tags_set", { tags: [...list, t] }));
      addTag(t);
      setTyping(null);
      setError("");
    } catch (e) { setError(err(e)); }
  };
  const how = Object.fromEntries(suggested.map((s) => [s.name, HOW[s.how] || s.how]));
  const options = [...new Set([...(to ? [to] : []), ...suggested.map((s) => s.name), ...found])];
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      if (editing) {
        await api("variants_edit", { platform, name: editing, new_name: name, original: to || null });
        changed();
        toast(`Saved ${name}.`, { ms: 8000, actions: undoAction(platform) });
      } else {
        await add(platform, [{ files, name, original: to || null }]);
      }
      close();
      onDone?.(name);
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${editing ? "Edit the variant" : files.length > 1 ? `Add ${files.length} files as one game` : "Add to the custom DAT"} id="variant-dialog" onSubmit=${submit} busy=${busy || !title.trim()} error=${error} submitLabel=${editing ? "Save" : "Add it"}>
    <label class="field-block"><span class="field-label">Title</span>
      <input type="text" id="variant-title" value=${title} onInput=${(e) => setTitle(e.target.value)} /></label>
    <div class="field-block"><span class="field-label">Tags</span>
      <ul class="chip-list" id="variant-tags">${tags.map((t) => html`<li class="chip-item" key=${t}>${t}<button type="button" class="chip-x" aria-label=${`Take off ${t}`} onClick=${() => setTags(tags.filter((x) => x !== t))}>×</button></li>`)}
        <li>${typing === null ? html`<select id="variant-tag-add" class="compact" value="" aria-label="Add a tag" onChange=${(e) => { const v = e.target.value; e.target.value = ""; if (v === "__new") setTyping(""); else if (v) addTag(v); }}>
            <option value="">Add a tag…</option>
            ${list.filter((t) => !has(t)).map((t) => html`<option key=${t} value=${t}>${t}</option>`)}
            <option value="__new">New tag…</option></select>`
          : html`<span class="variant-new-tag"><input type="text" id="variant-new-tag" class="compact" placeholder="The new tag" value=${typing} onInput=${(e) => setTyping(e.target.value)}
              onKeyDown=${(e) => { if (e.key === "Enter") { e.preventDefault(); newTag(); } else if (e.key === "Escape") { e.stopPropagation(); setTyping(null); } }} />
            <button type="button" class="ghost compact" id="variant-new-tag-add" onClick=${newTag}>Add</button></span>`}</li></ul></div>
    <p class="muted" id="variant-name">Its name: <b>${name}</b></p>
    <div class="field-block"><span class="field-label">A variant of</span>
      <input type="search" id="variant-search" class="compact" placeholder="Find the game in the system's DAT" value=${search} onInput=${(e) => setSearch(e.target.value)} />
      <select id="variant-original" class="compact" size="6" value=${to} onChange=${(e) => setTo(e.target.value)}>
        <option value="">None: a game of its own</option>
        ${options.map((n) => html`<option key=${n} value=${n}>${n}${how[n] ? ` (${how[n]})` : ""}</option>`)}
      </select></div>
    <p class="muted">${to ? "It shows under its own name, with its original's details and pictures until it's scraped for itself." : "A game of its own has no original: scrape it for its details."} ${editing ? "" : "Its file isn't renamed; if its name differs, Rename on the Variants tab puts it right."}</p>
  <//>`;
}

export function VariantDialogs({ dialog }) {
  if (dialog.type === "variant") return html`<${VariantDialog} ...${dialog} />`;
  return null;
}
