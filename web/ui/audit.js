// Audit: one platform's set against one of its DATs (docs/PLAN.md, "Phase 1
// design"). The platform and DAT are chosen at the top (the DAT picked stays the
// platform's, for Home too), the counts are tabs, and each list pages through the
// index with its search, so a list of 40,000 missing games loads a page at a time.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, plural } from "./library.js";
import { importDats, starterDats, reveal, scan } from "./games.js";
import { CommandBar, SearchNote } from "./layout.js";
import { organiseDialog, convertDialog, renameMisnamed, setAsideDuplicates } from "./organise.js";
import { Icon } from "./icons.js";
import { addGameDialog } from "./pcgames.js";

const PAGE = 200;

/** The lists, with the summary count each shows. */
export const TABS = [
  ["have", "Have", "have"],
  ["partial", "Partly there", "partial"],
  ["missing", "Missing", "missing"],
  ["matched", "Named right", "matched"],
  ["misnamed", "Misnamed", "misnamed"],
  ["baddump", "Bad dumps", "baddump"],
  ["unknown", "Unknown", "unknown"],
  ["duplicates", "Duplicates", "duplicates"],
];

const HELP = {
  have: "Games with every ROM their DAT lists.",
  partial: "Games with some of their ROMs (a multi-file game missing a part).",
  missing: "Games the DAT lists that none of your files match. Export fixdat writes them as a DAT.",
  matched: "Files that match the DAT and are named as it names them.",
  misnamed: "Files that match the DAT under another name.",
  baddump: "Files named as one of the DAT's ROMs that don't match it: likely corrupt, trimmed or patched copies.",
  unknown: "Files of this platform that no DAT for it knows: hacks, translations, homebrew or files for another system.",
  duplicates: "Files that match the same ROM as another file. Setting them aside keeps the copy that's named right (or the first).",
};

const prefs = () => ctx.platform?.store?.prefs;

function Row({ it, tab, onConvert }) {
  const games = tab === "have" || tab === "partial" || tab === "missing";
  const meta = games
    ? [it.regions, it.flags, it.clone_of ? `clone of ${it.clone_of}` : null, tab === "partial" ? `${it.have} of ${plural(it.roms, "ROM", "ROMs")}` : null].filter(Boolean).join(" · ")
    : [it.game && tab !== "unknown" && tab !== "baddump" ? it.game : null, it.method?.startsWith("crc") ? "matched by CRC" : null, it.baddump ? "a bad dump in the DAT" : null, it.offline ? "folder not found on this computer" : null].filter(Boolean).join(" · ");
  return html`<li class="audit-row" data-name=${it.name}>
    <div class="audit-main">
      <span class="audit-name">${it.name}</span>
      ${meta ? html`<span class="muted audit-meta">${meta}</span>` : null}
      ${tab === "misnamed" ? html`<span class="audit-expected">→ ${it.expected}</span>` : null}
      ${it.shown ? html`<span class="audit-path muted" title=${it.shown}>${it.shown}</span>` : null}
    </div>
    ${onConvert && tab === "have" ? html`<button type="button" class="ghost compact audit-convert-one" title=${`Convert ${it.name} to another format, or extract it`} onClick=${() => onConvert(it.name)}>Convert…</button>` : null}
    ${it.path && isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label=${`Show ${it.name} in its folder`} onClick=${() => reveal(it.path)}>${Icon.folder(14)}</button>` : null}
  </li>`;
}

export function Audit() {
  const s = useStore(ui, (st) => ({ library: st.library, home: st.home, changes: st.changes }));
  const [platform, setPlatform] = useState(() => prefs()?.get("gr-audit", {})?.platform || null);
  const [datId, setDatId] = useState(null);
  const [data, setData] = useState(null);
  const [tab, setTab] = useState(() => prefs()?.get("gr-audit", {})?.tab || "missing");
  const [q, setQ] = useState("");
  const [list, setList] = useState({ total: 0, items: [], loading: true });

  // the summary: again when the platform or DAT changes, and after a scan or import (Home's data is read again then)
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("audit_summary", { platform, dat: datId })
      .then((d) => { if (!gone) setData(d); })
      .catch((e) => { if (!gone) setData({ error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, platform, datId, s.home, s.changes]);

  const sum = data?.summary;
  const dat = sum?.dat?.id;
  useEffect(() => {
    if (!dat) return;
    let gone = false;
    setList((l) => ({ ...l, loading: true }));
    const t = setTimeout(() => {
      api("audit_list", { dat, tab, search: q, offset: 0, limit: PAGE })
        .then((r) => { if (!gone) setList({ ...r, tab, loading: false }); })
        .catch((e) => { if (!gone) setList({ total: 0, items: [], tab, loading: false, error: e.message || String(e) }); });
    }, q ? 200 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [dat, tab, q, sum]);

  const more = async () => {
    const r = await api("audit_list", { dat, tab, search: q, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  const choosePlatform = (p) => { setPlatform(p); setDatId(null); prefs()?.set("gr-audit", { platform: p, tab }); };
  const chooseTab = (t) => { setTab(t); prefs()?.set("gr-audit", { platform: sum?.dat?.platform || platform, tab: t }); };

  const fixdat = async () => {
    try {
      const r = await api("audit_fixdat", { dat });
      toast(`Wrote a fixdat of ${plural(r.games, "game", "games")} to ${r.rel || r.path}.`, { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.path) }] : [] });
    } catch (e) { toast(`Couldn't write the fixdat: ${e.message || e}`, 8000); }
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
    { id: "export-fixdat", label: "Export fixdat", icon: "download", disabled: !dat, run: fixdat },
    { id: "copy-report", label: "Copy report", icon: "copy", disabled: !dat, run: copyReport },
    { id: "save-report", label: "Save report", icon: "file", disabled: !dat, run: () => saveReport("txt") },
    { id: "save-report-csv", label: "Save report as CSV", icon: "table", disabled: !dat, run: () => saveReport("csv") },
    { sep: true },
    { id: "add-game", label: "Add a game…", icon: "plus", disabled: !!s.library?.read_only, run: () => addGameDialog({ platform: sum?.dat.platform && sum.dat.platform !== "pc" ? sum.dat.platform : "" }) },
    { id: "make-dat", label: "Make a DAT from a set…", icon: "stack", run: () => ui.set({ dialog: { type: "make-dat" } }) },
    ...(isDesktop() ? [{ id: "import-dats", label: "Import DATs…", icon: "plus", run: () => importDats() }] : []),
  ];

  const pName = sum && (sum.platform_name || data.platforms.find((p) => p.id === sum.dat.platform)?.name);
  const canConvert = !!(sum && !s.library?.read_only && ui.get().platforms?.find((p) => p.id === sum.dat.platform)?.formats?.length);
  const convert = (games) => convertDialog({ dat, platform: sum.dat.platform, platformName: pName, games });
  const head = html`<${CommandBar} id="audit-head" title="Audit" count=${sum ? `${sum.dat.name} (${sum.dat.version})` : null}
    primary=${html`<span class="cmd-buttons">${sum && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-organise" title="Sort this set by its DAT: copy, rename or move, with a preview and Undo"
        onClick=${() => organiseDialog({ dat, platform: sum.dat.platform, platformName: pName })}>${Icon.stack(15)} Organise…</button>` : null}
      ${canConvert ? html`<button type="button" class="ghost compact" id="audit-convert" title="Convert this platform's games to another format (CHD, RVZ, CSO, ZIP), or extract them, with a preview and Undo"
        onClick=${() => convert(null)}>${Icon.convert(15)} Convert…</button>` : null}
      <button type="button" class=${sum ? "primary compact" : "ghost compact"} id="audit-scan" disabled=${!!ui.get().scanning || !!s.library?.read_only} onClick=${() => scan()}>${Icon.refresh(15)} Scan</button></span>`}
    more=${{ id: "audit-menu", title: "Export fixdat, reports, make a DAT", items: menu }} />`;

  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading the audit…</p></div>`;
  if (data.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${data.error}</p></div>`;
  if (!sum) {
    return html`${head}<div class="pages"><div class="library-settings" id="audit-empty">
      <p class="page-lead">${Icon.audit(18)} Check your ROM sets against DAT files from No-Intro, Redump, TOSEC or your own.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <ol class="coming-list">
          <li>Add the folders your games are in: <a href=${routeHash("settings")}>Settings › Folders with games</a>.</li>
          <li>Get the starter DATs (No-Intro's, from libretro's public copy), or import your own: a No-Intro daily pack, a single <code>.dat</code> or <code>.zip</code>.</li>
          <li>Scan. Each file is read once; a second scan only reads what changed.</li>
        </ol>
        <div class="home-actions">
          <button type="button" class="primary" id="audit-starter" disabled=${!!s.library?.read_only} onClick=${() => starterDats()}>Get starter DATs…</button>
          ${isDesktop() ? html`<button type="button" class="ghost" id="audit-import" disabled=${!!s.library?.read_only} onClick=${() => importDats()}>Import DATs…</button>` : null}
          <button type="button" class="ghost" disabled=${!!s.library?.read_only} onClick=${() => ui.set({ dialog: { type: "make-dat" } })}>Make a DAT from a set…</button>
        </div>
      </section>
    </div></div>`;
  }

  const n = (k) => sum[k] || 0;
  const platformName = data.platforms.find((p) => p.id === sum.dat.platform)?.name;
  return html`${head}<div class="pages audit-page" id="audit-page">
    <div class="audit-pick">
      <label class="field-inline"><span>Platform</span>
        <select id="audit-platform" class="compact" value=${sum.dat.platform} onChange=${(e) => choosePlatform(e.target.value)}>
          ${data.platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
        </select></label>
      <label class="field-inline"><span>DAT</span>
        <select id="audit-dat" class="compact" value=${String(sum.dat.id)} disabled=${data.dats.length < 2} title=${data.dats.length < 2 ? "Import another DAT for this platform to choose between them" : "The DAT this platform is audited against (kept for Home too)"}
          onChange=${(e) => setDatId(Number(e.target.value))}>
          ${data.dats.map((d) => html`<option key=${d.id} value=${String(d.id)}>${d.name} (${d.version})</option>`)}
        </select></label>
    </div>
    <div class="audit-totals" id="audit-totals">
      <div class="audit-bar" role="img" aria-label=${`${n("have")} of ${n("games")} games`}><span style=${`width:${n("games") ? (100 * n("have")) / n("games") : 0}%`}></span></div>
      <p><b id="audit-have">${n("have")}</b> of <b id="audit-games">${n("games")}</b> games
        · 1G1R <b id="audit-1g1r">${n("groups_have")} of ${n("groups")}</b>
        ${n("by_crc") ? html` · <span class="muted" title="Files inside ZIPs matched by the CRC the ZIP records. Settings › Scan ZIPs › Unpack and hash checks them in full.">${plural(n("by_crc"), "file", "files")} matched by CRC</span>` : null}</p>
      ${n("needs_tool") ? html`<p class="warn-note" id="audit-needs-tool">${plural(n("needs_tool"), "RVZ file waits", "RVZ files wait")} for dolphin-tool before ${n("needs_tool") === 1 ? "it" : "they"} can be checked. Set where it is in <a href=${routeHash("settings")}>Settings › Tools</a>, then scan.</p>` : null}
    </div>
    <div class="seg audit-tabs" role="tablist" id="audit-tabs">${TABS.map(([k, label, key]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${tab === k ? "true" : "false"} onClick=${() => chooseTab(k)}>
      ${label} <span class="tab-count">${n(key)}</span></button>`)}</div>
    <div class="audit-tools">
      <input type="search" id="audit-search" class="compact" placeholder="Search this list" aria-label="Search this list" value=${q} onInput=${(e) => setQ(e.target.value)} />
      <span class="muted audit-help">${HELP[tab]}</span>
      ${tab === "misnamed" && n("misnamed") && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-rename" onClick=${() => renameMisnamed(dat, platformName)}>Rename these…</button>` : null}
      ${tab === "duplicates" && n("duplicates") && !s.library?.read_only ? html`<button type="button" class="ghost compact" id="audit-aside" onClick=${() => setAsideDuplicates(dat, platformName)}>Set aside duplicates…</button>` : null}
    </div>
    <${SearchNote} q=${q} found=${list.loading ? null : plural(list.total, "match", "matches")} onClear=${() => setQ("")} />
    ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${!list.loading && !list.items.length ? html`<p class="muted" id="audit-none">${q ? "Nothing here matches the search." : "Nothing in this list."}</p>` : null}
    <ul class="audit-list" id="audit-list" data-tab=${list.tab} data-total=${list.total} aria-busy=${list.loading ? "true" : "false"}>
      ${list.items.map((it) => html`<${Row} key=${`${it.id}-${it.name}`} it=${it} tab=${list.tab} onConvert=${canConvert ? (g) => convert([g]) : null} />`)}
    </ul>
    ${list.items.length < list.total ? html`<div class="home-actions"><button type="button" class="ghost" id="audit-more" onClick=${more}>Show ${Math.min(PAGE, list.total - list.items.length)} more (${list.total - list.items.length} left)</button></div>` : null}
  </div>`;
}
