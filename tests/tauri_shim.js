window.__TAURI_INTERNALS__ = {};
class Channel { constructor() { this.onmessage = () => {}; } }
window.__TAURI__ = { core: { Channel, async invoke(cmd, args, opts) {
  let body, headers = {}, ch = null;
  if (args instanceof Uint8Array || args instanceof ArrayBuffer) { body = args; headers = { ...(opts?.headers || {}), "content-type": "application/octet-stream" }; }
  else { const a = { ...(args || {}) }; for (const k in a) if (a[k] instanceof Channel) { ch = a[k]; a[k] = null; } body = JSON.stringify(a); headers["content-type"] = "application/json"; }
  const r = await fetch("/invoke/" + cmd, { method: "POST", body, headers });
  const ev = r.headers.get("x-events"); if (ev && ch) for (const e of JSON.parse(ev)) ch.onmessage(e);
  if (r.status >= 400 || r.headers.get("x-failed")) throw await r.json();
  if (r.headers.get("content-type") === "application/octet-stream") return await r.arrayBuffer();
  return await r.json();
} } };
// window events (files dragged and dropped on the window): the test sends them with __shimEmit
window.__shimListeners = {};
window.__TAURI__.event = { async listen(name, fn) { (window.__shimListeners[name] ||= []).push(fn); return () => {}; } };
window.__shimEmit = (name, payload) => (window.__shimListeners[name] || []).forEach((fn) => fn({ event: name, payload }));
