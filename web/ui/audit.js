// Audit: one platform's set against one of its DATs (docs/PLAN.md, "Phase 1
// design"). The systems are in the ribbon beside the menu, with All systems for a table
// of every one's counts (`#/audit/all`, `#/audit/<system>`, D69); the DAT is chosen at
// the top (the DAT picked stays the platform's, for Home too), the counts are tabs, and
// each list pages through the index with its search, so a list of 40,000 missing games
// loads a page at a time. A games list can be narrowed to a region and sorted by name,
// region or year. Every list is a table (D74), whose rows open the game's page; Convert…
// and Show in folder are in a row's right-click menu. Unknown suggests the game each file
// is likely a variant of, and Variants lists the system's custom DAT (variants.js, D119–D123).
// DATs… sets the main DAT and the DATs the system is audited with too (D118), each with its
// counts under the totals and Show its lists.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, plural, shortPath } from "./library.js";
import { importDats, starterDats, reveal, scan } from "./games.js";
import { CommandBar, SearchNote, SortMenu } from "./layout.js";
import { useSidePanel, useRouteArg, argHash } from "./panels.js";
import { organiseDialog, convertDialog, renameMisnamed, setAsideDuplicates } from "./organise.js";
import { Icon } from "./icons.js";
import { addGameDialog } from "./pcgames.js";
import { gameHash } from "./browse.js";
import { openMenu } from "./actions.js";
import { importFrom } from "./importpage.js";
import { UnknownTable, VariantsList } from "./variants.js";
import { Dialog, close } from "./dialogs.js";

const PAGE = 200;

/** The lists, with the summary count each shows. */
export const TABS = [
  // the games
  ["have", "Have", "have"],
  ["partial", "Partly there", "partial"],
  ["missing", "Missing", "missing"],
  // the files
  ["matched", "Named right", "matched"],
  ["misnamed", "Misnamed", "misnamed"],
  ["baddump", "Bad dumps", "baddump"],
  ["unknown", "Unknown", "unknown"],
  ["duplicates", "Duplicates", "duplicates"],
];

/** An arcade system's BIOS sets and devices (Phase 7, D58), a tab of their own. */
const SETS_TAB = ["sets", "BIOS and devices", "sets"];

/** A system's variants, its custom DAT's games (D119); arcade systems have none. */
const VARIANTS_TAB = ["variants", "Variants", "variants"];

/** How an arcade set's ZIPs are made up (D60). */
export const SET_TYPES = [
  ["non-merged", "Non-merged", "Every game's ZIP holds all the ROMs it needs (but its BIOS's), so any ZIP can be copied on its own."],
  ["split", "Split", "A clone's ZIP holds only the ROMs its parent's doesn't."],
  ["merged", "Merged", "A parent's ZIP holds its clones' ROMs too; clones have no ZIP of their own."],
];

const HELP = {
  have: "Games with every ROM their DAT lists.",
  partial: "Games with some of their ROMs (a multi-file game missing a part).",
  missing: "Games the DAT lists that none of your files match. Save a list of what's missing writes them as a DAT (a fixdat).",
  matched: "Files that match the DAT and are named as it names them.",
  misnamed: "Files that match the DAT under another name.",
  baddump: "Files named as one of the DAT's ROMs that don't match it: likely corrupt, trimmed or patched copies.",
  unknown: "Files of this system that no DAT for it knows, each with the game it's likely a variant of. They stay out of the Library until they're added as variants or games of their own.",
  unknown_arcade: "Files of this system that no DAT for it knows: hacks, bootlegs or files for another system.",
  variants: "Translations, hacks, prototypes and games no DAT lists, kept in the system's custom DAT. Each shows under its own name, with its tags, and the details and pictures of the game it's a variant of.",
  duplicates: "Files that match the same ROM as another file. Setting them aside keeps the copy that's named right (or the first).",
  sets: "The BIOS sets and devices the games lean on. They're kept as ZIPs of their own and copied with the games that need them.",
};

const WHY = { bios: "its BIOS set", device: "a device", parent: "its parent" };

const prefs = () => ctx.platform?.store?.prefs;
const GAME_TABS = ["have", "partial", "missing", "sets"];
const SORTS = [["name", "Name"], ["region", "Region"], ["year", "Year"]];
/** The overview's columns: [tab, heading]. */
const COLUMNS = [["have", "Have"], ["partial", "Partly there"], ["missing", "Missing"], ["misnamed", "Misnamed"], ["baddump", "Bad dumps"], ["unknown", "Unknown"], ["duplicates", "Duplicates"]];

/** A list as a table: a games list's columns, or a files list's. A row's name opens its
 *  game's page; its right-click menu has Convert… and Show in folder. */
function Table({ items, tab, arcade, platform, onConvert }) {
  const games = GAME_TABS.includes(tab);
  const keyOf = (it) => (games ? it.name : it.game) || null;
  const name = (it, text) => {
    const k = keyOf(it);
    return k && platform ? html`<a class="audit-open" href=${gameHash(`${platform}/${k}`)} title=${`Open ${k}'s page`}>${text}</a>` : text;
  };
  const menu = (e, it) => {
    const k = keyOf(it);
    const items = [
      ...(k && platform ? [{ id: "audit-open", label: "Open its page", icon: "external", run: () => { location.hash = gameHash(`${platform}/${k}`); } }] : []),
      ...(onConvert && tab === "have" ? [{ id: "audit-convert-one", label: "Convert…", icon: "convert", run: () => onConvert(it.name) }] : []),
      ...(it.path && isDesktop() ? [{ id: "audit-reveal", label: "Show in folder", icon: "folder", run: () => reveal(it.path) }] : []),
    ];
    if (!items.length) return;
    e.preventDefault();
    openMenu(e, items);
  };
  const show = (it) => it.path && isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label=${`Show ${it.name} in its folder`} onClick=${() => reveal(it.path)}>${Icon.folder(14)}</button>` : null;
  const open = (it) => { const k = keyOf(it); if (k && platform) location.hash = gameHash(`${platform}/${k}`); };
  const row = (it, cells) => html`<tr class="audit-row" key=${`${it.id}-${it.name}`} data-name=${it.name} tabindex="0"
    onDblClick=${() => open(it)} onKeyDown=${(e) => { if (e.key === "Enter" && e.target === e.currentTarget) open(it); }} onContextMenu=${(e) => menu(e, it)}>${cells}<td class="audit-acts">${show(it)}</td></tr>`;
  if (tab === "sets") {
    return html`<table class="audit-table" id="audit-list-table"><thead><tr><th>Name</th><th>Kind</th><th>Status</th><th>Maker</th><th></th></tr></thead>
      <tbody>${items.map((it) => row(it, html`<td class="audit-name">${it.name}${it.title && it.title !== it.name ? html` <span class="muted">${it.title}</span>` : null}</td>
        <td>${it.kind === "bios" ? "BIOS set" : "Device"}</td><td>${it.status === "have" ? "Have" : it.status === "partial" ? "Partly there" : "Missing"}</td><td>${it.manufacturer || ""}</td>`))}</tbody></table>`;
  }
  if (games) {
    return html`<table class="audit-table" id="audit-list-table"><thead><tr><th>Name</th><th>Region</th><th>Year</th>${arcade ? html`<th>Maker</th>` : null}<th>Flags</th><th>ROMs</th><th></th></tr></thead>
      <tbody>${items.map((it) => row(it, html`<td class="audit-name" title=${it.shown || null}>${name(it, it.name)}${arcade && it.title && it.title !== it.name ? html` <span class="muted">${it.title}</span>` : null}${it.clone_of ? html` <span class="muted">clone of ${it.clone_of}</span>` : null}
          ${it.needs?.length ? html`<span class="audit-needs" title="It runs once these are in your folders too">Needs ${it.needs.map((n) => `${n.name} (${WHY[n.why] || n.why})`).join(", ")}</span>` : null}
          ${it.samples?.length ? html`<span class="muted audit-samples" title=${`Its sounds, in samples/${it.name}.zip; it runs without them`}>No sounds for ${it.samples.join(", ")}</span>` : null}</td>
        <td>${it.regions || ""}</td><td>${it.year || ""}</td>${arcade ? html`<td>${it.manufacturer || ""}</td>` : null}<td>${it.flags || ""}</td>
        <td>${tab === "partial" ? `${it.have} of ${it.roms}` : it.roms ?? ""}</td>`))}</tbody></table>`;
  }
  const note = (it) => [it.method?.startsWith("crc") ? "matched by the ZIP's checksum" : null, it.baddump ? "a bad dump in the DAT" : null, it.offline ? "folder not found on this computer" : null].filter(Boolean).join(" · ");
  return html`<table class="audit-table" id="audit-list-table"><thead><tr><th>File</th>${tab === "unknown" ? null : html`<th>${tab === "misnamed" ? "Should be" : "Game"}</th>`}<th>Folder</th><th></th></tr></thead>
    <tbody>${items.map((it) => row(it, html`<td class="audit-name">${it.name}${note(it) ? html` <span class="muted">${note(it)}</span>` : null}</td>
      ${tab === "unknown" ? null : html`<td>${tab === "misnamed" ? name(it, it.expected) : name(it, it.game || "")}</td>`}
      <td class="audit-path muted" title=${it.shown || null}>${shortPath(it.shown, 3)}</td>`))}</tbody></table>`;
}

/** A system's DATs: the main one, whose counts are the totals, and those it's audited with
 *  too (D118), which take the files they know off Unknown and their games into the Library. */
function AuditDatsDialog({ platform, platformName, dats, main, custom, onDone }) {
  const [m, setM] = useState(main);
  const [also, setAlso] = useState(() => dats.filter((d) => d.also).map((d) => d.name));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const mainName = dats.find((d) => d.id === m)?.name;
  const tick = (n, on) => setAlso(on ? [...also.filter((x) => x !== n), n] : also.filter((x) => x !== n));
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await api("audit_dats_set", { platform, main: m, also: also.filter((n) => n !== mainName) });
      close();
      ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
      onDone?.(m);
    } catch (e) {
      setError(e.message || String(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`${platformName}'s DATs`} id="audit-dats-dialog" onSubmit=${submit} busy=${busy} error=${error} submitLabel="Save">
    <p class="muted">The main DAT's counts are the totals. A DAT it's audited with too (a translation group's, say) knows files the main one doesn't: they're off Unknown, and its games are in the Library and collections beside the main DAT's.</p>
    <ul class="ls-list" id="audit-dats-list">${dats.map((d) => html`<li key=${d.id} data-dat=${d.name}>
      <label class="check-row"><input type="radio" name="audit-main" checked=${m === d.id} onChange=${() => setM(d.id)} /> ${d.name} <span class="muted">${d.version}</span>${m === d.id ? html` <span class="muted">(main)</span>` : null}</label>
      <label class="check-row"><input type="checkbox" class="audit-also" disabled=${m === d.id} checked=${m !== d.id && also.includes(d.name)} onChange=${(e) => tick(d.name, e.target.checked)} /> Audit with it too</label></li>`)}
      ${custom ? html`<li class="muted" data-dat=${custom}>${custom}: the system's variants, always audited with it</li>` : null}</ul>
    ${isDesktop() ? html`<button type="button" class="ghost compact" id="audit-dats-import" onClick=${() => { close(); importFrom("settings-dats", () => importDats()); }}>${Icon.plus(14)} Import a DAT…</button>` : null}
  <//>`;
}

export function AuditDialogs({ dialog }) {
  if (dialog.type === "audit-dats") return html`<${AuditDatsDialog} ...${dialog} />`;
  return null;
}

/** Every system's counts, a row each. */
function Overview({ systems, onOpen }) {
  if (!systems) return html`<p class="muted">Reading the systems…</p>`;
  const tot = (k) => systems.reduce((a, x) => a + (x[k] || 0), 0);
  return html`<div class="audit-overview-page">
    <p class="page-lead">${Icon.audit(18)} ${plural(systems.length, "system", "systems")} with a DAT: <b id="audit-all-have">${tot("have").toLocaleString()}</b> of ${tot("games").toLocaleString()} games. Click a number for that list.</p>
    <table class="audit-table audit-overview" id="audit-overview"><thead><tr><th>System</th><th>DAT</th><th class="num">Games</th>${COLUMNS.map(([, h]) => html`<th class="num" key=${h}>${h}</th>`)}<th>Complete</th></tr></thead>
      <tbody>${systems.map((x) => {
        const pct = x.games ? Math.floor((100 * x.have) / x.games) : 0;
        return html`<tr key=${x.platform} data-platform=${x.platform}>
          <td><a href=${argHash("audit", x.platform)} onClick=${() => onOpen(x.platform, null)}>${x.platform_name}</a></td>
          <td class="muted">${x.dat?.name || ""}</td><td class="num">${(x.games || 0).toLocaleString()}</td>
          ${COLUMNS.map(([k]) => html`<td class="num" key=${k} data-tab=${k}>${x[k] ? html`<a href=${argHash("audit", x.platform)} onClick=${() => onOpen(x.platform, k)}>${x[k].toLocaleString()}</a>` : html`<span class="muted">0</span>`}</td>`)}
          <td><span class="audit-bar small" role="img" aria-label=${`${pct}%`}><span style=${`width:${pct}%`}></span></span> <span class="muted">${pct}%</span></td></tr>`;
      })}</tbody></table>
  </div>`;
}

export function Audit() {
  const s = useStore(ui, (st) => ({ library: st.library, home: st.home, changes: st.changes }));
  const arg = useRouteArg("audit");
  const all = arg === "all";
  const [saved, setSaved] = useState(() => prefs()?.get("gr-audit", {})?.platform || null);
  // the system in the address, else the one last shown
  const platform = arg && !all ? arg : saved;
  const [datId, setDatId] = useState(null);
  // a DAT it's audited with too, its lists looked at (D118)
  const [viewing, setViewing] = useState(null);
  const [data, setData] = useState(null);
  const [tab, setTab] = useState(() => prefs()?.get("gr-audit", {})?.tab || "missing");
  const [q, setQ] = useState("");
  const [list, setList] = useState({ total: 0, items: [], loading: true });
  const [tick, setTick] = useState(0);
  const [sets, setSets] = useState(null);
  const [systems, setSystems] = useState(null);
  const [region, setRegion] = useState("");
  const [sort, setSort] = useState("name");
  const [regions, setRegions] = useState([]);
  useEffect(() => {
    if (!arg || all || arg === saved) return;
    setSaved(arg);
    setDatId(null);
    setViewing(null);
    prefs()?.set("gr-audit", { ...(prefs()?.get("gr-audit", {}) || {}), platform: arg });
  }, [arg]);
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("audit_overview").then((o) => { if (!gone) setSystems(o.systems); }).catch(() => { if (!gone) setSystems([]); });
    return () => { gone = true; };
  }, [s.library?.path, s.home, s.changes, tick]);

  // the summary: again when the platform or DAT changes, and after a scan or import (Home's data is read again then)
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("audit_summary", viewing ? { platform, dat: viewing, view: true } : { platform, dat: datId })
      .then((d) => {
        if (gone) return;
        // the system remembered has no DAT now: the first that has one (review H23)
        if (!d.summary && platform && !arg && d.platforms?.length) {
          prefs()?.set("gr-audit", { ...(prefs()?.get("gr-audit", {}) || {}), platform: null });
          setSaved(null);
          return;
        }
        setData(d);
      })
      .catch((e) => { if (!gone) setData({ error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, platform, datId, viewing, s.home, s.changes, tick]);
  useEffect(() => {
    if (s.library) api("arcade_sets").then(setSets).catch(() => setSets(null));
  }, [s.library?.path, tick]);

  const sum = data?.summary;
  const dat = sum?.dat?.id;
  // an arcade system has no Variants tab
  useEffect(() => { if ((sum?.arcade || viewing) && tab === "variants") setTab("missing"); if (viewing && tab === "unknown") setTab("have"); }, [sum, viewing]);
  const games = GAME_TABS.includes(tab);
  useEffect(() => { setRegion(""); setRegions([]); }, [dat, tab]);
  const by = games ? { region, sort } : {};
  useEffect(() => {
    if (!dat || all || tab === "variants") return;
    let gone = false;
    setList((l) => ({ ...l, loading: true }));
    const t = setTimeout(() => {
      api("audit_list", { dat, tab, search: q, ...by, offset: 0, limit: PAGE })
        .then((r) => { if (!gone) { setList({ ...r, tab, by: games ? `${region}|${sort}` : "", loading: false }); if (r.regions) setRegions(r.regions); } })
        .catch((e) => { if (!gone) setList({ total: 0, items: [], tab, loading: false, error: e.message || String(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [dat, tab, q, sum, region, sort, all]);

  const more = async () => {
    const r = await api("audit_list", { dat, tab, search: q, ...by, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  // from the overview: the system, at one of its lists
  const openSystem = (p, t) => { if (t) { setTab(t); prefs()?.set("gr-audit", { platform: p, tab: t }); } };
  useSidePanel(systems?.length ? {
    id: "audit-panel", route: "audit", title: "Systems", icon: "audit", count: systems.length, current: all ? "" : sum?.dat?.platform,
    all: { label: "All systems", href: argHash("audit", "all"), icon: "table" },
    // one line each, the count beside the name (D74)
    items: systems.map((x) => ({ id: x.platform, label: x.platform_name, count: `${(x.have || 0).toLocaleString()} of ${(x.games || 0).toLocaleString()}`, href: argHash("audit", x.platform) })),
  } : null);
  const chooseTab = (t) => { setTab(t); prefs()?.set("gr-audit", { platform: sum?.dat?.platform || platform, tab: t }); };

  const fixdat = async () => {
    try {
      const r = await api("audit_fixdat", { dat });
      toast(`Wrote a fixdat of ${plural(r.games, "game", "games")} to ${r.rel || r.path}.`, { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.path) }] : [] });
    } catch (e) { toast(`Couldn't save the list of what's missing: ${e.message || e}`, 8000); }
  };
  const copyReport = async () => {
    const { text } = await api("audit_report", { dat });
    try { await navigator.clipboard.writeText(text); toast("Copied the report."); } catch { toast("Couldn't reach the clipboard.", 5000); }
  };
  const saveReport = async (format) => {
    try {
      const r = await api("audit_save_report", { dat, format });
      toast(`Saved the report to ${r.rel || r.path}.`, { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.path) }] : [] });
    } catch (e) { toast(`Couldn't save the report: ${e.message || e}`, 8000); }
  };
  const menu = () => [
    { id: "export-fixdat", label: "Save a list of what's missing", title: "A DAT of the missing games (a fixdat), for other tools", icon: "download", disabled: !dat, run: fixdat },
    { id: "copy-report", label: "Copy the report", icon: "copy", disabled: !dat, run: copyReport },
    { id: "save-report", label: "Save the report", icon: "file", disabled: !dat, run: () => saveReport("txt") },
    { id: "save-report-csv", label: "Save the report as CSV", icon: "table", disabled: !dat, run: () => saveReport("csv") },
    { sep: true },
    // the imports are the Import page's (D78)
    { id: "add-game", label: "Import a game file…", icon: "plus", disabled: !!s.library?.read_only, run: () => importFrom("import-copy", () => addGameDialog({ platform: sum?.dat.platform && sum.dat.platform !== "pc" ? sum.dat.platform : "" })) },
    { id: "make-dat", label: "Make a DAT from a folder…", icon: "stack", run: () => ui.set({ dialog: { type: "make-dat" } }) },
    ...(isDesktop() ? [{ id: "import-dats", label: "Import DATs…", icon: "plus", run: () => importFrom("settings-dats", () => importDats()) }] : []),
  ];

  const pName = sum && (sum.platform_name || data.platforms.find((p) => p.id === sum.dat.platform)?.name);
  const canConvert = !!(sum && !s.library?.read_only && ui.get().platforms?.find((p) => p.id === sum.dat.platform)?.formats?.length);
  const convert = (games) => convertDialog({ dat, platform: sum.dat.platform, platformName: pName, games });
  const head = html`<${CommandBar} id="audit-head" title=${all ? "Audit: all systems" : pName ? `Audit: ${pName}` : "Audit"} count=${sum && !all ? `${sum.dat.name} (${sum.dat.version})` : null}
    primary=${html`<span class="cmd-buttons">${sum && !all && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-organise" title="Sort this system's games by its DAT: copy, rename or move, with a preview and Undo"
        onClick=${() => organiseDialog({ dat, platform: sum.dat.platform, platformName: pName })}>${Icon.stack(15)} Organise…</button>` : null}
      ${canConvert && !all ? html`<button type="button" class="ghost compact" id="audit-convert" title="Convert this system's games to another format (CHD, RVZ, CSO, ZIP), or unpack them, with a preview and Undo"
        onClick=${() => convert(null)}>${Icon.convert(15)} Convert…</button>` : null}
      <button type="button" class=${sum ? "primary compact" : "ghost compact"} id="audit-scan" disabled=${!!ui.get().scanning || !!s.library?.read_only} onClick=${() => scan()}>${Icon.refresh(15)} Scan</button></span>`}
    more=${{ id: "audit-menu", title: "What's missing, reports, make a DAT", items: menu }} />`;

  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading the audit…</p></div>`;
  if (data.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${data.error}</p></div>`;
  if (!sum) {
    return html`${head}<div class="pages"><div class="library-settings" id="audit-empty">
      <p class="page-lead">${Icon.audit(18)} Check your games against each system's DAT, the list of every game there is: a DAT from No-Intro, Redump, TOSEC or your own.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <ol class="coming-list">
          <li>Add the folders your games are in: <a href=${routeHash("import")}>Import › Folders with games</a>.</li>
          <li>Get the starter DATs (No-Intro's, from libretro's public copy), or import your own: a No-Intro daily pack, a single <code>.dat</code> or <code>.zip</code>.</li>
          <li>Scan. Each file is read once; a second scan only reads what changed.</li>
        </ol>
        <div class="home-actions">
          <button type="button" class="primary" id="audit-starter" disabled=${!!s.library?.read_only} onClick=${() => importFrom("settings-dats", () => starterDats())}>Get starter DATs…</button>
          ${isDesktop() ? html`<button type="button" class="ghost" id="audit-import" disabled=${!!s.library?.read_only} onClick=${() => importFrom("settings-dats", () => importDats())}>Import DATs…</button>` : null}
          <button type="button" class="ghost" disabled=${!!s.library?.read_only} onClick=${() => ui.set({ dialog: { type: "make-dat" } })}>Make a DAT from a folder…</button>
        </div>
      </section>
    </div></div>`;
  }
  if (all) return html`${head}<div class="pages audit-page" id="audit-page"><${Overview} systems=${systems} onOpen=${openSystem} /></div>`;

  const arc = sum.arcade;
  const n = (k) => (k === "sets" ? arc?.sets : k === "variants" ? data.variants?.games : sum[k]) || 0;
  const platformName = data.platforms.find((p) => p.id === sum.dat.platform)?.name;
  // (Unknown and Variants are the system's, not one DAT's: not there while another's lists are looked at)
  const tabs = (arc ? [...TABS, SETS_TAB] : [...TABS.slice(0, 3), VARIANTS_TAB, ...TABS.slice(3)]).filter(([k]) => !viewing || (k !== "unknown" && k !== "variants"));
  const shelf = (k) => GAME_TABS.includes(k) || k === "variants";
  const variants = tab === "variants" && !arc;
  const setType = sets?.systems?.[sum.dat.platform];
  const chooseSetType = async (t) => {
    try {
      await api("arcade_set", { platform: sum.dat.platform, set_type: t });
      setTick((x) => x + 1);
      toast(`${platformName} is kept as a ${t} set now. Files in the wrong ZIP are listed as misnamed: Rename to the DAT's names… puts them right.`, 8000);
    } catch (e) { toast(`Couldn't change the set type: ${e.message || e}`, 8000); }
  };
  return html`${head}<div class="pages audit-page" id="audit-page">
    <div class="audit-pick" id="audit-pick" data-platform=${sum.dat.platform}>
      ${data.dats.length > 1 ? html`<label class="field-inline"><span>DAT</span>
        <select id="audit-dat" class="compact" value=${String(data.main ?? sum.dat.id)} disabled=${data.dats.length < 2} title=${data.dats.length < 2 ? "Import another DAT for this system to choose between them" : "The DAT this system is audited against (kept for Home too)"}
          onChange=${(e) => { setViewing(null); setDatId(Number(e.target.value)); }}>
          ${data.dats.map((d) => html`<option key=${d.id} value=${String(d.id)}>${d.name} (${d.version})</option>`)}
        </select></label>` : null}
      <button type="button" class="ghost compact" id="audit-dats" title="The main DAT, the DATs it's audited with too, and Import a DAT…" disabled=${!!s.library?.read_only}
        onClick=${() => ui.set({ dialog: { type: "audit-dats", platform: sum.dat.platform, platformName, dats: data.dats, main: data.main, custom: data.variants?.dat?.name, onDone: (m) => { setViewing(null); setDatId(m); } } })}>${Icon.stack(14)} DATs…</button>
      ${arc && setType ? html`<label class="field-inline"><span>Set type</span>
        <select id="audit-set-type" class="compact" value=${setType} disabled=${!!s.library?.read_only} title=${SET_TYPES.find((t) => t[0] === setType)?.[2]}
          onChange=${(e) => chooseSetType(e.target.value)}>
          ${SET_TYPES.map(([k, label]) => html`<option key=${k} value=${k}>${label}</option>`)}
        </select></label>` : null}
    </div>
    <div class="audit-totals" id="audit-totals">
      <div class="audit-bar" role="img" aria-label=${`${n("have")} of ${n("games")} games`}><span style=${`width:${n("games") ? (100 * n("have")) / n("games") : 0}%`}></span></div>
      <p><b id="audit-have">${n("have")}</b> of <b id="audit-games">${n("games")}</b> games
        ${arc ? html` · <b id="audit-playable" title="Games had, with the BIOS sets and devices they need">${arc.playable}</b> playable
          · BIOS and devices <b id="audit-sets">${arc.sets_have} of ${arc.sets}</b>` : null}
        ${" · "}<span title="1G1R: titles (a game and its clones) you have at least one version of">One per title</span> <b id="audit-1g1r">${n("groups_have")} of ${n("groups")}</b>
        ${n("by_crc") ? html` · <span class="muted" title="Files inside ZIPs matched by the checksum the ZIP records. Settings › Scanning › Scan ZIPs › Unpack and hash every file checks them in full.">${plural(n("by_crc"), "file", "files")} matched by the ZIP's checksum</span>` : null}</p>
      ${data.also?.length && !viewing ? html`<p class="muted" id="audit-also">Audited with it too: ${data.also.map((a, i) => html`<span key=${a.dat.id} data-dat=${a.dat.name}>${i ? " · " : ""}${a.dat.name} (${a.dat.version}) <b>${(a.have || 0).toLocaleString()}</b> of ${(a.games || 0).toLocaleString()}${" "}<button type="button" class="linkish audit-also-view" onClick=${() => setViewing(a.dat.id)}>Show its lists</button></span>`)}</p>` : null}
      ${viewing && sum.dat.id === viewing ? html`<p class="warn-note" id="audit-viewing">These are the lists of ${sum.dat.name}, which ${pName} is audited with too.
        <button type="button" class="linkish" id="audit-viewing-back" onClick=${() => setViewing(null)}>Back to ${data.dats.find((d) => d.id === data.main)?.name || "the main DAT"}</button></p>` : null}
      ${n("needs_tool") ? html`<p class="warn-note" id="audit-needs-tool">${plural(n("needs_tool"), "RVZ file waits", "RVZ files wait")} for dolphin-tool before ${n("needs_tool") === 1 ? "it" : "they"} can be checked. Set where it is in <a href=${routeHash("settings")}>Settings › Tools</a>, then scan.</p>` : null}
    </div>
    <div class="audit-tab-groups" id="audit-tabs" role="tablist">${[["Games", tabs.filter(([k]) => shelf(k))], ["Files", tabs.filter(([k]) => !shelf(k))]].map(([group, list]) => html`<div class="audit-tab-group" key=${group}>
      <span class="audit-tab-label">${group}</span><div class="seg audit-tabs">${list.map(([k, label, key]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${tab === k ? "true" : "false"} onClick=${() => chooseTab(k)}>
      ${label} <span class="tab-count">${n(key)}</span></button>`)}</div></div>`)}</div>
    <div class="audit-tools">
      <input type="search" id="audit-search" class="compact" placeholder="Search this list" aria-label="Search this list" value=${q} onInput=${(e) => setQ(e.target.value)} />
      ${games && (regions.length > 1 || region) ? html`<select id="audit-region" class="compact" aria-label="Region" value=${region} onChange=${(e) => setRegion(e.target.value)}>
        <option value="">Every region</option>
        ${regions.map((r) => html`<option key=${r.region} value=${r.region}>${r.region} (${r.count.toLocaleString()})</option>`)}</select>` : null}
      ${games ? html`<${SortMenu} id="audit-sort" options=${SORTS} value=${sort} onChange=${setSort} />` : null}
      <span class="muted audit-help">${HELP[tab === "unknown" && arc ? "unknown_arcade" : tab]}</span>
      ${tab === "misnamed" && n("misnamed") && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-rename" onClick=${() => renameMisnamed(dat, platformName)}>Rename to the DAT's names…</button>` : null}
      ${tab === "duplicates" && n("duplicates") && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-aside" onClick=${() => setAsideDuplicates(dat, platformName)}>Set aside duplicates…</button>` : null}
    </div>
    ${variants ? html`<div class="audit-list-wrap" id="audit-list" data-tab="variants"><${VariantsList} platform=${sum.dat.platform} platformName=${platformName} q=${q} off=${!!s.library?.read_only} /></div>` : html`
    <${SearchNote} q=${q} found=${list.loading ? null : plural(list.total, "match", "matches")} onClear=${() => setQ("")} />
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${!list.loading && !list.items.length ? html`<p class="muted" id="audit-none">${q ? "Nothing here matches the search." : "Nothing in this list."}</p>` : null}
    <div class="audit-list-wrap" id="audit-list" data-tab=${list.tab} data-by=${list.by || null} data-total=${list.total} aria-busy=${list.loading ? "true" : "false"}>
      ${!list.items.length ? null : list.tab === "unknown" && !arc ? html`<${UnknownTable} items=${list.items} platform=${sum.dat.platform} off=${!!s.library?.read_only} />`
        : html`<${Table} items=${list.items} tab=${list.tab} arcade=${!!arc} platform=${sum.dat.platform} onConvert=${canConvert ? (g) => convert([g]) : null} />`}
    </div>
    ${list.items.length < list.total ? html`<div class="home-actions"><button type="button" class="ghost" id="audit-more" onClick=${more}>Show ${Math.min(PAGE, list.total - list.items.length)} more (${list.total - list.items.length} left)</button></div>` : null}`}
  </div>`;
}
