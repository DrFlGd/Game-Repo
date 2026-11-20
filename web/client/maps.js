// Maps and guides (docs/PLAN.md, "Maps and Guides notes, part 8", D225): every pack of maps
// and guides for Atlas the library holds, beside the games, grouped by the game each is for
// (the library's game it's linked to, else the game its description names, else "No game
// yet"), with a search. Install puts a pack in Atlas's packs folder on its own, through
// Downloads, whatever games this device has, so maps for a game installed some other way
// (Steam's, say) can go to Atlas too; Remove takes it out again, unless a game this device
// has still has it ticked, Atlas changed it here, or it isn't Game Repo Go's. Show in Atlas
// starts Atlas with it. Windows and Linux only, as a game's packs are (not on Android until
// the Atlas pack contract's trial T2 says how).
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call, info, pictureUrl } from "./backend.js";
import { go, say } from "./state.js";
import { ago, bytes, message, plural } from "./format.js";
import { holds } from "./packs.js";

/** Whether Maps and guides is offered on this device (beside the games in the menu). */
export const showMaps = (status) => info.kind !== "android" && !!status?.atlas;

/** Where a pack is from, as a game's page says it. */
const from = (p) => (p.source ? `from ${p.source}` : p.made_in ? `made in ${p.made_in}` : p.sort === "app" ? "came with an app" : "");

/** A pack's picture (its own cover, or the one chosen for it in Game Repo), else a map. */
function Cover({ p }) {
  const [bad, setBad] = useState(false);
  useEffect(() => setBad(false), [p.picture]);
  return html`<span class="go-pack-cover" aria-hidden="true">${p.picture && !bad
    ? html`<img src=${pictureUrl(`~pack/${p.pack_id}`, p.picture, 256)} alt="" decoding="async" draggable="false" onError=${() => setBad(true)} />`
    : Icon.map(30)}</span>`;
}

/** Where a pack stands on this device, in a line. */
function standing(p) {
  if (p.coming) return ["coming", "Coming: see Downloads"];
  if (p.newer_here) return ["newer-here", `Newer on this device: kept. Game Repo's copy is older, so it isn't put over it.${p.alone ? " Installed on its own." : ""}`];
  if (p.theirs) return ["theirs", `${p.theirs.newer ? "A newer copy" : "The same version"} is in Atlas's packs folder already (${p.theirs.file}), one Game Repo Go didn't put there. Atlas shows that one, so Go leaves it as it is.${p.alone ? " Installed on its own." : ""}`];
  if (p.alone) return ["alone", `Installed on its own${p.with?.length ? `, and with ${p.with.join(", ")}` : ""}`];
  if (p.here && p.with?.length) return ["with", `In Atlas's packs folder with ${p.with.join(", ")}`];
  if (p.here) return ["here", "In Atlas's packs folder"];
  return ["", ""];
}

function Pack({ p, first, busy, install, remove, show }) {
  const [said, line] = standing(p);
  const sub = [from(p), p.system, bytes(p.size)].filter(Boolean).join(" · ");
  return html`<li class="go-pack" data-pack=${p.pack_id} data-here=${p.here ? "yes" : "no"} data-alone=${p.alone ? "yes" : "no"}>
    <${Cover} p=${p} />
    <span class="go-pack-what">
      <strong>${p.title}</strong>
      <span class="muted">${sub}</span>
      ${holds(p) ? html`<small class="muted">${holds(p)}</small>` : null}
      ${p.own_things ? html`<small class="muted" data-said="mine">${p.own_things}</small>` : null}
      ${p.personal_use ? html`<small class="muted" data-said="personal">For your own use only</small>` : null}
      ${line ? html`<small class=${said === "alone" || said === "here" || said === "with" ? "go-pack-on" : "muted"} data-said=${said}>${line}</small>` : null}
      ${p.newer_in_library && !p.coming ? html`<small class="muted" data-said="newer-in-library">The library has a newer copy${p.alone ? ": it comes the next time the library is read" : ""}.</small>` : null}
    </span>
    <span class="go-pack-do">
      ${!p.alone && !p.coming ? html`<button type="button" class=${p.here ? "ghost" : "primary"} data-action="install" data-first=${first ? "" : null}
          data-say=${p.here ? "Kept in Atlas's packs folder on its own, whatever happens to the games" : "Into Atlas's packs folder on its own, through Downloads"}
          disabled=${!!busy} onClick=${() => install(p)}>${busy === p.pack_id ? "Asking…" : p.here ? "Keep on its own" : `Install · ${bytes(p.size)}`}</button>` : null}
      ${p.alone ? html`<button type="button" class="ghost danger" data-action="remove" data-first=${first ? "" : null} disabled=${!!busy} onClick=${() => remove(p)}>${busy === p.pack_id ? "Removing…" : "Remove"}</button>` : null}
      ${p.here ? html`<button type="button" class="ghost" data-action="show" data-say="Atlas comes to the front with this pack" disabled=${!!busy} onClick=${() => show(p)}>Show in Atlas</button>` : null}
    </span>
  </li>`;
}

/** The packs that match a search: by their title, the title they're grouped under, where
 *  they're from, their system, and their games. */
function matching(items, q) {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return items;
  return items.filter((p) => {
    const text = [p.title, p.group, p.source, p.made_in, p.system, ...(p.games || []).map((g) => g.title)].join(" ").toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

export function MapsPage() {
  const s = useStore(go, (x) => ({ deviceRev: x.deviceRev, queueRev: x.queueRev }));
  const [v, setV] = useState(null);
  const [error, setError] = useState("");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState("");
  const [said, setSaid] = useState("");
  const coming = useRef(false);
  const load = () => call("packs_library").then((r) => { setV(r); setError(""); coming.current = (r.items || []).some((p) => p.coming); }).catch((e) => setError(message(e)));
  useEffect(() => { load(); }, [s.deviceRev]);
  // a pack coming: read again as Downloads moves on
  useEffect(() => { if (coming.current) load(); }, [s.queueRev]);
  const run = async (p, f) => {
    setBusy(p.pack_id);
    setSaid("");
    try {
      await f();
    } catch (e) {
      say(message(e), /Settings/.test(message(e)) ? "settings" : null);
    } finally {
      setBusy("");
      load();
    }
  };
  const install = (p) => run(p, async () => {
    await call("pack_install", { pack: p.pack_id });
    setSaid(p.here ? `${p.title} is kept in Atlas's packs folder on its own now.` : `${p.title} is coming into Atlas's packs folder: see Downloads.`);
  });
  const remove = (p) => run(p, async () => {
    const r = await call("pack_remove", { pack: p.pack_id });
    const left = "It's no longer kept on its own.";
    setSaid({
      removed: `${p.title} taken out of Atlas's packs folder.`,
      kept_for: `${p.title} stays in Atlas's packs folder: ${(r.games || []).join(", ")} on this device ${(r.games || []).length === 1 ? "has" : "have"} it ticked. ${left}`,
      changed: `${p.title} stays in Atlas's packs folder: Atlas changed it on this device since Game Repo Go put it there. ${left}`,
      not_gos: `${p.title} stays in Atlas's packs folder: Game Repo Go didn't put that copy there. ${left}`,
    }[r.result] || `${p.title}: ${left}`);
  });
  const show = (p) => run(p, async () => {
    const r = await call("atlas_show", { pack: p.pack_id });
    say(`Atlas was asked to show ${r.title}.`);
  });
  if (!v) return html`<section class="go-page go-maps" id="go-maps">${error ? html`<p class="form-error" role="alert">${error}</p>` : html`<p class="muted">Opening maps and guides…</p>`}</section>`;
  if (!v.offered) return html`<section class="go-page go-maps" id="go-maps"><h1>Maps and guides</h1><p class="empty">Maps and guides for Atlas aren't offered on this device yet.</p></section>`;
  const shown = matching(v.items, text.trim());
  const groups = [];
  for (const p of shown) {
    const last = groups[groups.length - 1];
    if (last && last.title === p.group) last.packs.push(p);
    else groups.push({ title: p.group, packs: [p] });
  }
  return html`<section class="go-page go-maps" id="go-maps">
    <div class="go-page-head">
      <h1>Maps and guides</h1>
      <label class="search small go-search">${Icon.search(16)}<span class="visually-hidden">Search maps and guides</span>
        <input type="search" id="maps-search" placeholder="Search maps and guides" value=${text} onInput=${(e) => setText(e.currentTarget.value)}
          onKeyDown=${(e) => { if (e.key === "Escape" && text) { e.preventDefault(); setText(""); } }} />
      </label>
    </div>
    <p class="muted" id="maps-note">Install puts a pack in Atlas's packs folder, <span class="go-file">${v.folder}</span>, on its own: it stays there whatever happens to the games on this device, and comes again when the library has a newer copy. For maps of games installed another way, such as Steam's.${v.on ? "" : " Maps and guides for Atlas in Settings needn't be on for this: that brings each game's ticked packs with it."}</p>
    ${v.offline ? html`<div class="go-note warn-note" id="maps-offline" role="status"><span>${v.error}${v.at ? ` This is the library's list from ${ago(v.at)}.` : ""}</span>
      <button type="button" class="ghost" onClick=${load}>Try again</button></div>` : null}
    ${v.unreadable ? html`<p class="muted" id="maps-unreadable">${plural(v.unreadable, "pack")} in the library can't be read by Game Repo, so ${v.unreadable === 1 ? "it isn't" : "they aren't"} listed.</p>` : null}
    ${said ? html`<p id="maps-said" role="status">${said}</p>` : null}
    ${!v.items.length ? html`<p class="empty" id="maps-none">${v.offline ? "No list of the library's maps and guides is kept on this device yet." : "The library has no maps and guides yet. Game Repo keeps them in its Maps and Guides section, where Atlas Importer writes them."}</p>` : null}
    ${v.items.length && !shown.length ? html`<p class="empty" id="maps-no-match">No maps and guides match “${text.trim()}”.</p>` : null}
    ${groups.map((g, i) => html`<section class="go-pack-group" key=${g.title} data-group=${g.title}>
      <h2>${g.title} <span class="muted">${plural(g.packs.length, "pack")}</span></h2>
      <ul class="go-pack-rows">${g.packs.map((p, j) => html`<${Pack} key=${p.pack_id} p=${p} first=${i === 0 && j === 0} busy=${busy} install=${install} remove=${remove} show=${show} />`)}</ul>
    </section>`)}
  </section>`;
}
