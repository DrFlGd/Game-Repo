// The panels beside a page (from Model Library and SCAD Workshop, D69): on the left, next
// to the menu, a page's own list (the collections, the audited systems, the magazines),
// resizable and folded to a strip with its name ([ folds it); on the right, the
// inspector: a pane with tabs and a rail along the window's edge with each tab's name
// written down it (] folds it). In a narrow window the left list goes into the menu's
// drawer, under its page, and the inspector's pane covers the page when it's opened.
import { html, useEffect, useLayoutEffect, useRef, useState } from "../lib/html.js";
import { createStore, useStore } from "../lib/store.js";
import { ui, setPref } from "./state.js";
import { Icon } from "./icons.js";
import { typing } from "./actions.js";
import { useMedia, NARROW } from "./chrome.js";

/** The page's list: { route, title, icon, count, items: [{ id, label, sub, count, href,
 *  children }], current, all: { label, href, count } (an entry above the items for all of
 *  them), actions: [{ id, label, icon, run, disabled }] }, or null. An item's `children`
 *  ([{ id, label, count, run }], a collection's groups) are listed indented under it. */
export const sidePanel = createStore({ spec: null });

/** Show `spec` as the page's list while the page is shown. */
export function useSidePanel(spec) {
  const mine = useRef(null);
  useLayoutEffect(() => { mine.current = spec; sidePanel.set({ spec }); });
  // (only its own: the next page's may be up already)
  useEffect(() => () => { if (sidePanel.get().spec === mine.current) sidePanel.set({ spec: null }); }, []);
}

/** The second segment of the address (`#/collections/<id>` → id), decoded. */
export function routeArg(hash = location.hash) {
  const rest = hash.replace(/^#\/?/, "").split("?")[0].split("/").slice(1).join("/");
  try { return rest ? decodeURIComponent(rest) : ""; } catch { return rest; }
}

/** `#/<route>/<arg>`. */
export const argHash = (route, arg) => `#/${route}${arg ? `/${encodeURIComponent(arg)}` : ""}`;

/** The address's second segment, kept up as it changes. */
export function useRouteArg(route) {
  const [arg, setArg] = useState(() => routeArg());
  useEffect(() => {
    const on = () => { if (location.hash.replace(/^#\/?/, "").split(/[/?]/)[0] === route) setArg(routeArg()); };
    addEventListener("hashchange", on);
    return () => removeEventListener("hashchange", on);
  }, [route]);
  return arg;
}

const MIN_W = 180, MAX_W = 420;

function Rows({ spec, onPicked }) {
  return html`<ul class="nav-list cat-panel-tree" id=${`${spec.id}-list`}>
    ${spec.all ? html`<li class="nav-row panel-all"><a class=${`nav-link${spec.current === "" ? " active" : ""}`} href=${spec.all.href} data-id="" aria-current=${spec.current === "" ? "page" : null} onClick=${onPicked}>
      <span class="nav-icon">${Icon[spec.all.icon || spec.icon]?.(15)}</span><span class="nav-label">${spec.all.label}</span>${spec.all.count != null ? html`<span class="nav-count">${spec.all.count}</span>` : null}</a></li>` : null}
    ${spec.items.map((x) => html`<li class="nav-row" key=${x.id}>
      <a class=${`nav-link panel-link${spec.current === x.id ? " active" : ""}`} href=${x.href} data-id=${x.id} aria-current=${spec.current === x.id ? "page" : null} title=${x.sub ? `${x.label}: ${x.sub}` : x.label} onClick=${onPicked}>
        <span class="panel-link-text"><span class="nav-label">${x.label}</span>${x.sub ? html`<span class="panel-sub">${x.sub}</span>` : null}</span>
        ${x.count != null ? html`<span class="nav-count">${x.count}</span>` : null}</a></li>
      ${(x.children || []).map((k) => html`<li class="nav-row panel-child-row" key=${`${x.id}/${k.id}`} style="--depth:1">
        <a class="nav-link panel-child" href=${x.href} data-id=${k.id} title=${k.label} onClick=${(e) => { e.preventDefault(); k.run?.(); onPicked?.(); }}>
          <span class="nav-label">${k.label}</span>${k.count != null ? html`<span class="nav-count">${k.count}</span>` : null}</a></li>`)}`)}
  </ul>`;
}

/** The page's list beside the menu (the Sidebar draws it after the menu). */
export function SidePanel() {
  const spec = useStore(sidePanel, (s) => s.spec);
  const route = useStore(ui, (s) => s.route);
  const prefs = useStore(ui, (s) => [s.sidePanel, s.sidePanelWidth]);
  const narrow = useMedia(NARROW);
  const medium = useMedia("(max-width: 1250px)");
  const stopResize = useRef(null);
  useEffect(() => () => stopResize.current?.(), []);
  const folded = prefs[0] === "folded" || (prefs[0] == null && medium);
  useEffect(() => {
    const key = (e) => {
      if (e.key !== "[" || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || typing(e) || ui.get().dialog || ui.get().menu || !sidePanel.get().spec) return;
      e.preventDefault();
      setPref({ sidePanel: document.querySelector(".category-panel.folded") ? "open" : "folded" });
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, []);
  if (!spec || spec.route !== route || narrow) return null;
  const width = Math.max(MIN_W, Math.min(MAX_W, prefs[1] || 240));
  const resize = (e) => {
    e.preventDefault();
    stopResize.current?.();
    const start = e.clientX, before = width;
    const move = (ev) => setPref({ sidePanelWidth: Math.max(MIN_W, Math.min(MAX_W, before + ev.clientX - start)) });
    const stop = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", stop); removeEventListener("pointercancel", stop); };
    stopResize.current = stop;
    addEventListener("pointermove", move); addEventListener("pointerup", stop); addEventListener("pointercancel", stop);
  };
  if (folded) {
    return html`<aside class="category-panel folded" id=${spec.id} aria-label=${spec.title}>
      <button type="button" class="ghost panel-strip" id=${`${spec.id}-open`} title=${`Show ${spec.title.toLowerCase()} ([)`} aria-label=${`Show ${spec.title.toLowerCase()} ([)`} onClick=${() => setPref({ sidePanel: "open" })}>
        <span class="panel-strip-icon">${Icon[spec.icon]?.(16)}</span><span class="panel-strip-name">${spec.title}</span>${Icon.chevronsRight(13)}</button>
    </aside>`;
  }
  return html`<aside class="category-panel" id=${spec.id} style=${{ width: `${width}px` }} aria-label=${spec.title}>
    <header class="cat-panel-head">
      <span class="cat-panel-title"><span class="cat-icon">${Icon[spec.icon]?.(17)}</span><span>${spec.title}</span>${spec.count != null ? html`<span class="nav-count">${spec.count}</span>` : null}</span>
      ${(spec.actions || []).map((a) => html`<button type="button" class="ghost panel-fold" key=${a.id} id=${a.id} title=${a.label} aria-label=${a.label} disabled=${!!a.disabled} onClick=${a.run}>${Icon[a.icon](14)}</button>`)}
      <button type="button" class="ghost panel-fold" id=${`${spec.id}-close`} title=${`Hide ${spec.title.toLowerCase()} ([)`} aria-label=${`Hide ${spec.title.toLowerCase()} ([)`} onClick=${() => setPref({ sidePanel: "folded" })}>${Icon.chevronsLeft(14)}</button>
    </header>
    <${Rows} spec=${spec} />
    ${!spec.items.length && spec.empty ? html`<p class="muted nav-note">${spec.empty}</p>` : null}
    <div class="cat-panel-resize" role="separator" aria-label=${`${spec.title} width`} aria-orientation="vertical" aria-valuemin=${MIN_W} aria-valuemax=${MAX_W} aria-valuenow=${width} tabIndex="0"
      onPointerDown=${resize} onKeyDown=${(e) => { if (["ArrowLeft", "ArrowRight"].includes(e.key)) { e.preventDefault(); setPref({ sidePanelWidth: Math.max(MIN_W, Math.min(MAX_W, width + (e.key === "ArrowLeft" ? -20 : 20))) }); } }}></div>
  </aside>`;
}

/** In the narrow window's drawer: the page's list under its place in the menu. */
export function DrawerRows({ route }) {
  const spec = useStore(sidePanel, (s) => s.spec);
  if (!spec || spec.route !== route) return null;
  return html`<li class="nav-inline"><${Rows} spec=${spec} onPicked=${() => ui.set({ navOpen: false })} /></li>`;
}

// ------------------------------------------------------------ the inspector

const MIN = 260, MAX = 560;

/** `page`: whose settings these are. `tabs`: [{ id, label, icon, body }] (a tab without
 *  a body isn't shown). `open`: whether it starts open on this page. */
export function Inspector({ page, tabs, open: startOpen = true, label = "Details" }) {
  const st = useStore(ui, (s) => ({ open: s.inspector?.[page], tab: s.inspectorTab?.[page], width: s.inspectorWidth }));
  const list = tabs.filter((t) => t.body !== undefined && t.body !== false && t.body !== null);
  const narrow = useMedia("(max-width: 899px)");
  const [localOpen, setLocalOpen] = useState(false);
  useLayoutEffect(() => setLocalOpen(false), [narrow]);
  const isOpen = narrow ? localOpen : st.open == null ? startOpen : st.open === "open";
  const tab = list.find((t) => t.id === st.tab) || list[0];
  const width = Math.max(MIN, Math.min(MAX, st.width || 360));
  const set = (patch) => {
    const prefs = {};
    if ("tab" in patch) prefs.inspectorTab = { ...(ui.get().inspectorTab || {}), [page]: patch.tab };
    if ("open" in patch) {
      if (narrow) setLocalOpen(patch.open);
      else prefs.inspector = { ...(ui.get().inspector || {}), [page]: patch.open ? "open" : "folded" };
    }
    setPref(prefs);
  };
  const toggle = () => set({ open: !isOpen });
  useEffect(() => {
    const key = (e) => {
      if (e.key !== "]" || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || typing(e) || ui.get().dialog || ui.get().menu) return;
      e.preventDefault();
      toggle();
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  });
  const stopResize = useRef(null);
  useEffect(() => () => stopResize.current?.(), []);
  const resize = (e) => {
    e.preventDefault();
    stopResize.current?.();
    const start = e.clientX, before = width;
    const move = (ev) => setPref({ inspectorWidth: Math.max(MIN, Math.min(MAX, before + start - ev.clientX)) });
    const stop = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", stop); removeEventListener("pointercancel", stop); };
    stopResize.current = stop;
    addEventListener("pointermove", move); addEventListener("pointerup", stop); addEventListener("pointercancel", stop);
  };
  if (!list.length) return null;
  const pick = (t) => (isOpen && tab?.id === t.id ? set({ open: false }) : set({ open: true, tab: t.id }));
  return html`<div class=${`inspector-side${isOpen ? " open" : ""}`} id="inspector" data-page=${page} data-tab=${isOpen ? tab?.id : null}>
    ${isOpen && tab ? html`<section class="inspector-pane" style=${{ width: `${width}px` }} aria-label=${tab.label} id=${`pane-${tab.id}`}>
      <header class="pane-head"><strong>${tab.label}</strong>
        <button type="button" class="ghost panel-fold" id="inspector-fold" title=${`Hide ${label.toLowerCase()} (])`} aria-label=${`Hide ${label.toLowerCase()} (])`} onClick=${toggle}>${Icon.chevronsRight(14)}</button></header>
      <div class="pane-body">${tab.body}</div>
      <div class="pane-resize" role="separator" aria-label="Width" aria-orientation="vertical" aria-valuemin=${MIN} aria-valuemax=${MAX} aria-valuenow=${width} tabIndex="0"
        onPointerDown=${resize} onKeyDown=${(e) => { if (["ArrowLeft", "ArrowRight"].includes(e.key)) { e.preventDefault(); setPref({ inspectorWidth: Math.max(MIN, Math.min(MAX, width + (e.key === "ArrowLeft" ? 20 : -20))) }); } }}></div>
    </section>` : null}
    <nav class="rail rail-right" aria-label=${label}>
      ${list.map((t) => html`<button type="button" key=${t.id} class="rail-tab" id=${`rail-${t.id}`} data-tab=${t.id} aria-pressed=${isOpen && tab?.id === t.id ? "true" : "false"}
        title=${isOpen && tab?.id === t.id ? `Hide ${t.label} (])` : `Show ${t.label} (])`} onClick=${() => pick(t)}>
        ${Icon[t.icon] ? Icon[t.icon](16) : null}<span class="rail-name">${t.label}</span></button>`)}
    </nav>
  </div>`;
}
