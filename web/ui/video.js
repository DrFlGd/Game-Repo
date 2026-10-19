// A game's video: shown with the player's controls, one playing at a time, and opened
// in the system's own player when the app can't play it (an AVI, or a codec it lacks).
import { html, useState, useEffect } from "../lib/html.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { isDesktop } from "./library.js";
import { Icon } from "./icons.js";

/** A library file that is a video (by its name). */
export const isVideo = (rel) => /\.(mp4|m4v|mov|webm|mkv|avi)$/i.test(rel || "");

// A video starting pauses any other: two players of one file sounded twice (Jerred, batch 4).
document.addEventListener("play", (e) => {
  for (const m of document.querySelectorAll("video, audio")) {
    if (m !== e.target && !m.paused) m.pause();
  }
}, true);

/** A video from the library (`rel`), at `src` (the library URL, with any cache key). */
export function Video({ rel, src, cls }) {
  const [broken, setBroken] = useState(false);
  useEffect(() => setBroken(false), [src]);
  const root = ui.get().library?.path;
  if (broken) {
    return html`<span class=${`video-none ${cls || ""}`} data-video="cant-play">${Icon.video(20)}
      <span>This video can't play in the app.</span>
      ${isDesktop() && root ? html`<button type="button" class="ghost compact" onClick=${() => ctx.platform.library.openPath(`${root}/${rel}`)}>${Icon.external(14)} Open it in its own player</button>` : null}</span>`;
  }
  return html`<video class=${cls} src=${src} controls preload="metadata" onError=${() => setBroken(true)}></video>`;
}
