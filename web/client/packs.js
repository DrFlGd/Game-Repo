// A game's packs of maps and guides for Atlas on its page (D222): each with its size and a
// tick, all ticked at first. Installing the game with Maps and guides for Atlas on brings
// its ticked packs into Atlas's packs folder; ticking one later on a game this device has
// brings it, and unticking one takes it out again, if Game Repo Go put it there. A pack
// whose file on this device is newer than the library's (Atlas changed it here) is kept and
// said so. *Show in Atlas* starts Atlas on the game when one of its packs is in Atlas's
// packs folder here (D224). A pack installed on its own from Maps and guides (D225) stays when
// it's unticked here. A game's pack of guidebooks (D238) lists each guidebook with its size
// and a tick of its own: the pack comes again with those ticked, and none ticked takes it out.
// Windows and Linux only: not offered on Android yet.
import { html, useEffect, useState } from "../lib/html.js";
import { call, info } from "./backend.js";
import { closeDialog, open, say } from "./state.js";
import { Dialog } from "./dialog.js";
import { bytes, message, plural } from "./format.js";

/** What a pack holds, as people say it: "2 maps · 340 locations · 12 guide pages". */
export const holds = (p) => p.sort === "magazines" ? [
  plural(p.issues || 0, "issue"),
  p.guide_pages ? plural(p.guide_pages, "page") : null,
].filter(Boolean).join(" · ") : p.sort === "guidebooks" ? [
  plural(p.guidebooks?.length || 0, "guidebook"),
  p.guide_pages ? plural(p.guide_pages, "page") : null,
].filter(Boolean).join(" · ") : [
  p.maps ? plural(p.maps, "map") : null,
  p.locations ? plural(p.locations, "location") : null,
  p.guide_pages ? plural(p.guide_pages, "guide page") : null,
  p.wiki_pages ? plural(p.wiki_pages, "wiki page") : null,
].filter(Boolean).join(" · ");

/** Whether the game's page shows its packs at all. */
export const showPacks = (g) => !!(g?.packs?.offered && info.kind !== "android" && g.packs.items?.length);

/** Show in Atlas (D224): Atlas started showing the game's pack that's in its packs folder
 *  here (`packs.show`, the backend's pick), brought to the front; Windows and Linux only.
 *  When Atlas isn't found, it says where to set it, with Open Settings. */
export function ShowInAtlas({ k, packs, big = false }) {
  const [busy, setBusy] = useState(false);
  if (!packs?.offered || info.kind === "android" || !packs.show) return null;
  const show = async () => {
    setBusy(true);
    try {
      const r = await call("atlas_show", { key: k });
      say(`Atlas was asked to show ${r.title}.`);
    } catch (e) {
      const m = message(e);
      say(m, /Settings/.test(m) ? "settings" : null);
    } finally {
      setBusy(false);
    }
  };
  return html`<button type="button" class="ghost" id="go-game-atlas-show" data-pack=${packs.show} data-say=${big ? "Atlas comes to the front with this game's maps" : null} disabled=${busy} onClick=${show}>${busy ? "Starting Atlas…" : "Show in Atlas"}</button>`;
}

export function Packs({ k, packs, onDevice, onChange, atlasButton = true }) {
  const [busy, setBusy] = useState("");
  const [said, setSaid] = useState("");
  // a tick shown as it was clicked until the page's details are read again
  const [ticks, setTicks] = useState({});
  useEffect(() => setTicks({}), [packs]);
  if (!packs?.offered || info.kind === "android" || !packs.items?.length) return null;
  const isTicked = (p) => ticks[p.pack_id] ?? !!p.ticked;
  const run = async (what, f) => {
    setBusy(what);
    setSaid("");
    try {
      await f();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy("");
      onChange?.();
    }
  };
  const tick = (p, on) => run(p.pack_id, async () => {
    setTicks((t) => ({ ...t, [p.pack_id]: on }));
    const r = await call("pack_tick", { key: k, pack: p.pack_id, on }).catch((e) => { setTicks((t) => ({ ...t, [p.pack_id]: !on })); throw e; });
    if (r.queued?.length) setSaid(`${p.title} is coming: see Downloads.`);
    else if (r.removed) setSaid(`${p.title} taken out of Atlas's packs folder.`);
  });
  const get = () => run("get", async () => {
    const r = await call("packs_get", { key: k });
    setSaid(r.queued?.length ? "Its packs are coming: see Downloads." : "Its packs are already in the queue.");
  });
  // a guidebook in a game's pack of guidebooks ticked or unticked (D238)
  const gbTicked = (p, g) => ticks[`${p.pack_id}/${g.id}`] ?? !!g.ticked;
  const gbTick = (p, g, on) => run(`${p.pack_id}/${g.id}`, async () => {
    const at = `${p.pack_id}/${g.id}`;
    setTicks((t) => ({ ...t, [at]: on }));
    const r = await call("guidebook_tick", { key: k, pack: p.pack_id, guidebook: g.id, on }).catch((e) => { setTicks((t) => ({ ...t, [at]: !on })); throw e; });
    if (r.queued?.length) setSaid(`${p.title} is coming again with the ticked guidebooks: see Downloads.`);
    else if (r.removed) setSaid(`${p.title} taken out of Atlas's packs folder: no guidebook is ticked.`);
  });
  const missing = packs.items.filter((p) => isTicked(p) && !p.here && !p.theirs);
  return html`<section class="go-packs" id="go-game-packs" data-on=${packs.on ? "yes" : "no"}>
    <h2>Maps and guides for Atlas</h2>
    <p class="muted" id="go-game-packs-note">${packs.on
      ? onDevice ? html`Ticked packs are in Atlas's packs folder, <span class="go-file">${packs.folder}</span>. Untick one to take it out.` : "The ticked packs come with the game, into Atlas's packs folder."
      : html`Turn on Maps and guides for Atlas in Settings for the ticked packs to come with the game. <button type="button" class="ghost compact" id="go-game-packs-settings" onClick=${() => { closeDialog(); open({ page: "settings" }); }}>Open Settings</button>`}</p>
    ${packs.show && atlasButton ? html`<div class="go-game-do"><${ShowInAtlas} k=${k} packs=${packs} /></div>` : null}
    <ul class="go-sizes go-pack-list">${packs.items.map((p) => html`<li key=${p.pack_id} data-pack=${p.pack_id} data-here=${p.here ? "yes" : "no"}>
      <label class="check-row"><input type="checkbox" checked=${isTicked(p)} disabled=${!!busy} aria-label=${`Bring ${p.title}`} onChange=${(e) => tick(p, e.currentTarget.checked)} />
        <span><strong>${p.title}</strong>${p.source ? html` <span class="muted">from ${p.source}</span>` : p.made_in ? html` <span class="muted" data-said="made-in">made in ${p.made_in}</span>` : p.sort === "app" ? html` <span class="muted" data-said="came-with-app">came with an app</span>` : p.sort === "guidebooks" ? html` <span class="muted" data-said="guidebooks">a pack of guidebooks</span>` : null}
          ${holds(p) ? html`<br /><small class="muted">${holds(p)}</small>` : null}
          ${p.own_things ? html`<br /><small class="muted" data-said="mine">${p.own_things}</small>` : null}
          ${p.newer_here ? html`<br /><small class="muted" data-said="newer-here">Newer on this device: kept. Game Repo's copy is older, so it isn't put over it.</small>`
            : p.theirs ? html`<br /><small class="muted" data-said="theirs">${p.theirs.newer ? "A newer copy" : "The same version"} is in Atlas's packs folder already (<span class="go-file">${p.theirs.file}</span>), one Game Repo Go didn't put there. Atlas shows that one, so Go leaves it as it is and doesn't download its own.</small>`
            : p.here ? html`<br /><small class="muted" data-said="here">In Atlas's packs folder</small>` : null}
          ${p.alone ? html`<br /><small class="muted" data-said="alone">Installed on its own too, from Maps and guides: it stays when unticked here.</small>` : null}
          ${p.personal_use ? html`<br /><small class="muted" data-said="personal">For your own use only</small>` : null}
          ${p.error ? html`<br /><small class="muted" data-said="error">Game Repo can't read this pack (${p.error}); it's sent as it is.</small>` : null}</span></label>
      <span class="muted">${bytes(p.size)}</span>
      ${p.guidebooks?.length ? html`<ul class="go-sizes go-guidebooks" data-pack=${p.pack_id}>${p.guidebooks.map((g) => html`<li key=${g.id} data-guidebook=${g.id}>
        <label class="check-row"><input type="checkbox" checked=${!g.cannot_go && gbTicked(p, g)} disabled=${!!busy || !!g.cannot_go || !isTicked(p)} aria-label=${`Bring ${g.title}`} onChange=${(e) => gbTick(p, g, e.currentTarget.checked)} />
          <span>${g.title} <span class="muted">${[g.kind?.toUpperCase(), g.pages ? plural(g.pages, "page") : null].filter(Boolean).join(" · ")}</span>
            ${g.cannot_go ? html`<br /><small class="muted" data-said="cannot-go">${g.cannot_go}</small>` : null}</span></label>
        <span class="muted">${bytes(g.size)}</span></li>`)}</ul>` : null}</li>`)}</ul>
    ${packs.on && onDevice && missing.length ? html`<div class="go-game-do"><button type="button" class="ghost" id="go-game-packs-get" disabled=${!!busy} onClick=${get}>${busy === "get" ? "Asking…" : `Download ${missing.length === 1 ? "its ticked pack" : `its ${missing.length} ticked packs`} (${bytes(missing.reduce((n, p) => n + (p.size || 0), 0))})`}</button></div>` : null}
    ${said ? html`<p id="go-game-packs-said">${said}</p>` : null}
  </section>`;
}

/** The big screen's Maps and guides…: the same list, as a panel from the right. */
export function PacksDialog({ gameKey, title }) {
  const [g, setG] = useState(null);
  const [rev, setRev] = useState(0);
  useEffect(() => { call("game", { key: gameKey }).then(setG).catch((e) => say(message(e))); }, [gameKey, rev]);
  return html`<${Dialog} title=${`Maps and guides for ${title}`} id="packs-dialog" onSubmit=${closeDialog} cancelLabel="Close">
    ${g ? html`<${Packs} k=${gameKey} packs=${g.packs} onDevice=${g.on_device} atlasButton=${false} onChange=${() => setRev((r) => r + 1)} />` : html`<p class="muted">Opening…</p>`}
  <//>`;
}
