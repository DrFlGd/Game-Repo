// BIOS files (docs/PLAN.md, "Phase 5b design", D40): a category of their own, audited
// against a BIOS DAT (libretro's System.dat, or one of your own) system by system, the
// systems you have games for first, every system shown with a search (D69). Importing copies the files the library has into
// <library>/BIOS/<system>/, and an export in a layout with a BIOS folder carries them.
// The app never ships or downloads BIOS files themselves, only the list of them.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, plural, size } from "./library.js";
import { importDats, starterDats, reveal } from "./games.js";
import { CommandBar, SearchNote } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
const STATUS = { have: "You have it", missing: "Missing", wrong: "A file of its name, but not this one" };

/** Import BIOS files: the plan's preview, then Import. */
function GatherDialog({ plan }) {
  const nothing = !plan.ops;
  return html`<${Dialog} title=${plan.label} id="bios-gather-dialog" wide=${true} onSubmit=${() => { close(); if (!nothing) runPlan(plan); }} busy=${plan.short} submitLabel=${nothing ? "Close" : "Import"} cancelLabel=${nothing ? "" : "Cancel"}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to import: ${plan.in_place ? "the BIOS files the library has are in its BIOS folder already." : "the library has none of the files the BIOS DATs list. Add the folder they're in (Import › Folders with games) and scan."}</p>`
      : html`<p class="muted">Each file is copied (or hardlinked) into the library's BIOS folder, in a folder for its system, and checked. The files where they are now aren't changed. The change can be undone from the message afterwards or from History.</p>`}
  <//>`;
}

export function BiosDialogs({ dialog }) {
  if (dialog.type === "bios-gather") return html`<${GatherDialog} key=${dialog.plan.id} ...${dialog} />`;
  return null;
}

function System({ s, open }) {
  const all = s.files.length;
  const names = s.systems.map((x) => x.name).join(", ");
  const badge = s.have === all ? "all" : s.have ? "some" : "none";
  return html`<li><details class="bios-system" data-system=${s.name} data-have=${s.have} data-missing=${s.missing} data-wrong=${s.wrong} open=${open}>
    <summary><strong>${names || s.name}</strong>
      ${names && names !== s.name ? html`<span class="muted" title="Its name in the BIOS DAT">${s.name}</span>` : null}
      ${s.yours ? html`<span class="muted">you have games for it</span>` : null}
      <span class=${`bios-badge ${badge}`}>${s.have} of ${plural(all, "file", "files")}</span></summary>
    <ul class="bios-files">${s.files.map((f, i) => html`<li key=${i} class=${`bios-${f.status}`} data-status=${f.status}>
      <code>${f.name}</code>${f.size ? html` <span class="muted">${size(f.size)}</span>` : null} · <span title=${f.at || null}>${STATUS[f.status]}</span>${f.loose === false ? html` <span class="muted">(inside a ZIP: Import BIOS files… takes it out, and exports carry it then)</span>` : null}</li>`)}</ul>
  </details></li>`;
}

/** Import BIOS files (D73): the plan copying the files the library's folders hold into its BIOS folder. */
export async function importBios() {
  try {
    const plan = await api("bios_gather_plan");
    ui.set({ dialog: { type: "bios-gather", plan } });
  } catch (e) {
    toast(`Couldn't plan it: ${err(e)}`, 8000);
  }
}

export function Bios() {
  const s = useStore(ui, (st) => ({ library: st.library, home: st.home, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [q, setQ] = useState("");
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("bios").then((d) => { if (!gone) setData(d); }).catch((e) => { if (!gone) setData({ error: err(e) }); });
    return () => { gone = true; };
  }, [s.library?.path, s.home, s.changes]);
  const gather = importBios;
  const report = async () => {
    try {
      const r = await api("bios_report");
      toast(`Saved the report to ${r.rel || r.path}.`, { ms: 10000, actions: isDesktop() ? [{ label: "Show", run: () => reveal(r.path) }] : [] });
    } catch (e) {
      toast(`Couldn't save the report: ${err(e)}`, 8000);
    }
  };
  const importOne = () => importDats(null, { "*": "bios" });
  const has = !!data?.dats?.length;
  const menu = () => [
    { id: "bios-starter", label: "Get libretro's list of BIOS files…", icon: "download", disabled: off, run: () => starterDats(["bios"]) },
    ...(isDesktop() ? [{ id: "bios-import", label: "Import a BIOS DAT…", icon: "plus", disabled: off, run: importOne }] : []),
    { sep: true },
    { id: "bios-report", label: "Save a report of what's missing", icon: "file", disabled: off || !has, run: report },
    ...(isDesktop() && data?.folder ? [{ id: "bios-folder", label: "Show the BIOS folder", icon: "folder", run: () => ctx.platform.library.openPath(data.folder) }] : []),
  ];
  const head = html`<${CommandBar} id="bios-head" title="BIOS files" count=${data?.systems ? plural(data.systems.length, "system", "systems") : null}
    primary=${has && !off ? html`<button type="button" class="primary compact" id="bios-gather" title="Copy the BIOS files the library has into its BIOS folder, a folder for each system, with a preview and Undo" onClick=${gather}>${Icon.inbox(15)} Import BIOS files…</button>` : null}
    more=${{ id: "bios-menu", title: "Get the list, import a BIOS DAT, save a report", items: menu }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading the BIOS files…</p></div>`;
  if (data.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${data.error}</p></div>`;
  if (!has) {
    return html`${head}<div class="pages"><div class="library-settings" id="bios-empty">
      <p class="page-lead">${Icon.chip(18)} The BIOS and firmware files emulators need, kept as a category of their own: audited for each system, imported into the library, and exported with a collection's games.</p>
      <section class="ls-section">
        <h2>To start</h2>
        <ol class="coming-list">
          <li>Get libretro's list of BIOS files (its System.dat, from its public copy on GitHub), or import a BIOS DAT of your own. A list names the files and their hashes; the files themselves are yours to find.</li>
          <li>Add the folder your BIOS files are in (<a href=${routeHash("import")}>Import › Folders with games</a>) and scan. They're found by their hashes, whatever they're called.</li>
          <li>Import them into the library, and tick BIOS files when you export a collection to a device.</li>
        </ol>
        <div class="home-actions">
          <button type="button" class="primary" id="bios-get" disabled=${off} onClick=${() => starterDats(["bios"])}>Get libretro's list…</button>
          ${isDesktop() ? html`<button type="button" class="ghost" id="bios-import-first" disabled=${off} onClick=${importOne}>Import a BIOS DAT…</button>` : null}
        </div>
      </section>
    </div></div>`;
  }
  const needle = q.trim().toLowerCase();
  const list = data.systems.filter((x) => !needle || x.name.toLowerCase().includes(needle) || x.systems.some((y) => y.name.toLowerCase().includes(needle)) || x.files.some((f) => f.name.toLowerCase().includes(needle)));
  const yours = data.systems.filter((x) => x.yours);
  const complete = yours.filter((x) => x.have === x.files.length).length;
  return html`${head}<div class="pages bios-page" id="bios-page">
    <p class="page-lead">${Icon.chip(18)} Each system's BIOS files, as ${data.dats.map((d) => (d.name === "System" ? "libretro's System.dat" : d.name)).join(" and ")} list${data.dats.length === 1 ? "s" : ""} them. Imported ones are in the library's <code title=${data.folder}>BIOS</code> folder.</p>
    <div class="audit-totals" id="bios-totals">
      <div class="audit-bar" role="img" aria-label=${`${complete} of ${yours.length} systems`}><span style=${`width:${yours.length ? (100 * complete) / yours.length : 0}%`}></span></div>
      <p>${yours.length ? html`Systems you have games for: <b id="bios-complete">${complete}</b> of <b id="bios-yours">${yours.length}</b> with every BIOS file${data.none ? html`, <b id="bios-none">${data.none}</b> with none` : ""}.`
        : html`None of the systems you have games for has a BIOS file listed.`} <span class="muted">Not every file is needed by every emulator: libretro lists every file any of its emulators can use.</span></p>
    </div>
    <div class="audit-tools">
      <input type="search" id="bios-search" class="compact" placeholder="Search systems and files" aria-label="Search systems and files" value=${q} onInput=${(e) => setQ(e.target.value)} />
    </div>
    <${SearchNote} q=${q} found=${plural(list.length, "system", "systems")} onClear=${() => setQ("")} />
    ${list.length ? null : html`<p class="muted" id="bios-nothing">${q ? "Nothing matches the search." : "The BIOS DATs list no systems."}</p>`}
    <ul class="bios-list" id="bios-list">${list.map((x) => html`<${System} key=${`${x.dat}-${x.name}`} s=${x} open=${list.length <= 3 || !!needle} />`)}</ul>
  </div>`;
}
