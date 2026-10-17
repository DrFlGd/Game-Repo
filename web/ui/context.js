// What the interface needs from the rest of the app, set once at start-up by
// app.js: { platform }.
export const ctx = {};
export function setContext(values) { Object.assign(ctx, values); }

/** The pages, by route. The ones still to come (`phase`, coming.js) show what they'll do. */
export const PAGES = ["home", "audit", "collections", "pc", "metadata", "documents", "settings", "about"];

/** Where a route lives in the URL, and back. */
export const routeHash = (route) => (route === "home" ? "#/" : `#/${route}`);

export function routeFromHash(hash) {
  const first = hash.replace(/^#\/?/, "").split(/[/?]/)[0];
  return PAGES.includes(first) ? first : "home";
}
