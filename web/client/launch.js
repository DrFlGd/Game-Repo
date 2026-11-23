// Atlas started with a game in Steam (docs/PLAN.md, "Maps and Guides notes, part 11",
// D241–D243), on Linux and the Steam Deck: the Atlas pack contract's launch line, built by
// the backend from the AppImage Atlas's settings name (`linuxAppImage`), else the usual
// ~/Applications/Atlas.AppImage, shown with Copy for any game in Steam; *Start Atlas with my
// games*, which puts it in the launch options of the games Game Repo Go adds to Steam, never
// over options you changed in Steam; and each game's own switch on its page. A path the line
// can't hold gets no line: why is said instead. Not on Windows (the line is bash's) nor
// Android.
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call, info } from "./backend.js";
import { go, say } from "./state.js";
import { message } from "./format.js";
import { InstallLink } from "./atlas.js";

/** `text` on the clipboard: the web view's, else the selected line copied the old way.
 *  False when neither could (the line is left selected). */
export async function copyText(text, input) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // (no clipboard for the page here: the old way, from the selected line)
  }
  try {
    input?.focus();
    input?.select();
    if (document.execCommand("copy")) return true;
  } catch {
    // (nor that)
  }
  return false;
}

/** The line, whole, in a box of its own, with Copy (reachable by a controller: a button). */
export function AtlasLine({ line, id }) {
  const box = useRef(null);
  const [copied, setCopied] = useState("");
  useEffect(() => setCopied(""), [line]);
  const copy = async () => {
    const ok = await copyText(line, box.current);
    setCopied(ok ? "yes" : "no");
    say(ok ? "Copied. In Steam, paste it into the game's Properties › General › Launch Options." : "Press Ctrl+C to copy the line, now it's selected.");
  };
  return html`<div class="go-atlas-line" id=${id}>
    <textarea class="go-file" readonly rows="3" tabindex="-1" value=${line} ref=${box} aria-label="The launch line" spellcheck="false" data-say="The line for a game's launch options in Steam" onFocus=${(e) => e.currentTarget.select()}></textarea>
    <button type="button" class="ghost" id=${`${id}-copy`} data-copied=${copied || null} data-say="Copies the line, to paste into a game's launch options in Steam" onClick=${copy}>Copy</button>
  </div>`;
}

/** What the line can't hold in a path. */
const FORBIDDEN = "' \" $ ` \\";

/** Why there's no line, in a sentence. */
const refusedSay = (a) => html`No line: Atlas's AppImage is at <span class="go-file">${a.path}</span>, and ${a.refused}, so the line can't name it. Move the AppImage to a folder whose path has none of ${FORBIDDEN} or a line break, and start Atlas from it once.`;

/** The games whose launch options were left as you set them, or joined, in a sentence. */
function gamesSay(games) {
  const names = (o) => games.filter((g) => g.switch && g.outcome === o).map((g) => g.name);
  const theirs = names("theirs");
  const joined = names("joined");
  const cannot = names("cannot_join");
  return [
    theirs.length ? html`<p class="warn-note" id="set-atlas-start-theirs">Launch options you changed in Steam are left as they are, so Atlas doesn't start with ${theirs.join(", ")}: copy the line into them yourself.</p>` : null,
    joined.length ? html`<p class="muted" id="set-atlas-start-joined">The line was put in front of the launch options you set for ${joined.join(", ")}: they're plain words, so they mean the same after it.</p>` : null,
    cannot.length ? html`<p class="warn-note" id="set-atlas-start-cannot">Not put in for ${cannot.join(", ")}: the arguments Game Repo gives the game can't follow the line for certain. Copy it into its launch options yourself.</p>` : null,
  ];
}

/** Settings › Maps and guides for Atlas, on Linux: the line with Copy, and *Start Atlas with
 *  my games*. */
export function AtlasLaunchSettings({ settings, set }) {
  const rev = useStore(go, (x) => x.settings);
  const [a, setA] = useState(null);
  useEffect(() => { if (info.kind === "linux") call("atlas_launch").then(setA).catch(() => setA(null)); }, [rev]);
  if (info.kind !== "linux" || !a?.offered) return null;
  const where = a.from === "atlas"
    ? html`The AppImage Atlas last started from, as Atlas's settings say: <span class="go-file">${a.path}</span>.`
    : html`Atlas's settings don't name its AppImage yet (Atlas says where it is once it's started from it), so this is the usual place, <span class="go-file">${a.path}</span>: change the path in the line if it's elsewhere.`;
  return html`<div class="go-atlas-start" id="set-atlas-launch" data-from=${a.from}>
    <h3>Start Atlas with a game in Steam</h3>
    <p class="muted">Put this line in a game's launch options in Steam (its Properties › General › Launch Options) and Atlas starts beside the game, without taking the focus: in Game Mode the Steam button switches between them.</p>
    ${a.line ? html`<${AtlasLine} line=${a.line} id="set-atlas-line" />
      <p class=${a.exists ? "muted" : "warn-note"} id="set-atlas-appimage">${where}${a.exists ? "" : " It isn't there now."}${!a.exists && a.installable ? html` <${InstallLink} id="set-atlas-appimage-install" />` : null}</p>`
      : html`<p class="warn-note" id="set-atlas-refused">${refusedSay(a)}</p>`}
    <label class="check-row"><input type="checkbox" id="set-atlas-start" checked=${a.on} disabled=${!a.line} onChange=${() => set({ atlas_with_games: !a.on })} />
      <span>Start Atlas with my games</span></label>
    <p class="muted" id="set-atlas-start-note">The line goes in the launch options of the PC games Game Repo Go adds to Steam${settings.atlas_with_games == null ? `, ${a.exists ? "on as Atlas's AppImage is there" : "off until Atlas's AppImage is there"}` : ""}; each game's page can turn it off for that game. Launch options you changed in Steam are never written over, and when the AppImage moves, the line follows it. Steam's own games aren't changed: copy the line into theirs.</p>
    ${gamesSay(a.games || [])}
  </div>`;
}

/** A PC game's own switch on its page (D242): Atlas started with it through its Steam
 *  shortcut, with how its launch options came out, and the line to copy when they're yours. */
export function AtlasStart({ k, a, onChange, big = false }) {
  const [busy, setBusy] = useState(false);
  if (!a || info.kind !== "linux") return null;
  const flip = async () => {
    setBusy(true);
    try {
      await call("atlas_launch_game", { key: k, on: !a.switch });
      onChange?.();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
    }
  };
  const said = !a.on ? (a.refused ? "There's no launch line for Atlas's AppImage: Settings › Maps and guides for Atlas says why." : "Start Atlas with my games is off in Settings › Maps and guides for Atlas.")
    : !a.switch ? "Atlas doesn't start with it."
    : a.outcome === "theirs" ? "You changed its launch options in Steam, so they're left as they are. Copy the line into them yourself to start Atlas with it:"
    : a.outcome === "cannot_join" ? "The arguments it starts with can't follow the line for certain, so it isn't put in. Copy it into its launch options yourself:"
    : a.waiting ? "Its launch options in Steam get the line once Steam is closed (Settings › Steam)."
    : a.outcome === "joined" ? "Atlas starts beside it when you play it from Steam. The line is in front of the launch options you set, which mean the same after it."
    : "Atlas starts beside it when you play it from Steam, without taking the focus.";
  return html`<div class="go-atlas-start" id="go-atlas-start" data-outcome=${a.outcome} data-waiting=${a.waiting ? "yes" : "no"}>
    <label class="check-row"><input type="checkbox" id="go-atlas-start-on" checked=${a.switch && a.on} disabled=${busy || !a.on}
      data-say=${big ? "Atlas starts beside this game in Steam" : null} onChange=${flip} /> <span>Start Atlas with this game</span></label>
    <p class="muted" id="go-atlas-start-said">${said}</p>
    ${a.on && a.switch && (a.outcome === "theirs" || a.outcome === "cannot_join") && a.line ? html`<${AtlasLine} line=${a.line} id="go-atlas-start-line" />` : null}
  </div>`;
}
