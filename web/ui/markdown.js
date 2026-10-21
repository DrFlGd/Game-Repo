// Markdown as this repository writes it (CHANGELOG.md, THIRD_PARTY.md, an update's notes),
// shown as nodes and never as HTML, and the changelog's releases. About and the updates'
// panel (updates.js) show them; it imports nothing of the app's, so the server's window and
// Game Repo Go load it alone.
import { html } from "../lib/html.js";

/** Text with `code`, **bold**, *italic* and [links](https://…), as nodes (never as HTML). */
function inline(text) {
  const out = [];
  const re = /`([^`]+)`|\*\*(.+?)\*\*|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)|\*([^*\s][^*]*)\*/g;
  let at = 0, m;
  while ((m = re.exec(text))) {
    if (m.index > at) out.push(text.slice(at, m.index));
    if (m[1] != null) out.push(html`<code>${m[1]}</code>`);
    else if (m[2] != null) out.push(html`<strong>${inline(m[2])}</strong>`);
    else if (m[3] != null) out.push(html`<a href=${m[4]} target="_blank" rel="noopener noreferrer">${inline(m[3])}</a>`);
    else out.push(html`<em>${m[5]}</em>`);
    at = re.lastIndex;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** The Markdown this repository writes (headings, paragraphs, lists, tables), as nodes. */
export function Markdown({ text }) {
  const blocks = [];
  let para = [], list = [], table = [];
  const flush = () => {
    if (para.length) blocks.push(html`<p>${inline(para.join(" "))}</p>`);
    if (list.length) blocks.push(html`<ul>${list.map((t) => html`<li>${inline(t)}</li>`)}</ul>`);
    if (table.length) {
      const cells = (row) => row.replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      const [head, , ...rows] = table;
      blocks.push(html`<div class="about-table"><table><thead><tr>${cells(head).map((c) => html`<th>${inline(c)}</th>`)}</tr></thead>
        <tbody>${rows.map((r) => html`<tr>${cells(r).map((c) => html`<td>${inline(c)}</td>`)}</tr>`)}</tbody></table></div>`);
    }
    para = []; list = []; table = [];
  };
  for (const line of text.split(/\r?\n/)) {
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) { flush(); blocks.push(h[1].length <= 2 ? html`<h3>${inline(h[2])}</h3>` : html`<h4>${inline(h[2])}</h4>`); }
    else if (/^\s*\|/.test(line)) { if (para.length || list.length) { const t = table; flush(); table = t; } table.push(line.trim()); }
    else if (/^\s*[-*]\s+/.test(line)) { if (para.length || table.length) flush(); list.push(line.replace(/^\s*[-*]\s+/, "")); }
    else if (!line.trim() || /^-{3,}$/.test(line.trim())) flush();
    else if (list.length && /^\s+\S/.test(line)) list[list.length - 1] += " " + line.trim();
    else { if (list.length || table.length) flush(); para.push(line.trim()); }
  }
  flush();
  return html`<div class="about-md">${blocks}</div>`;
}

/** CHANGELOG.md's releases, newest first: [{title, version, body}], without the agents' notes. */
export function releases(text) {
  const parts = text.split(/^## /m).slice(1).map((p) => {
    const nl = p.indexOf("\n");
    const title = (nl < 0 ? p : p.slice(0, nl)).trim();
    return { title, version: /^\d+\.\d+\.\d+/.exec(title)?.[0] || null, body: nl < 0 ? "" : p.slice(nl + 1).trim() };
  });
  return parts.filter((p) => p.version || /^unreleased/i.test(p.title));
}
