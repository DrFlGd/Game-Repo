// Game Repo Server's own window (desktop/server): the panel of Game Repo's Settings ›
// Serve to devices (ui/serve.js), talking to the server it's the window of through its
// "api" command, and its updates (ui/updates.js). Its tray icon's Pair a device…, Devices…
// and Check for updates… open it.
import { html, render } from "./lib/html.js";
import { ServePanel } from "./ui/serve.js";
import { UpdatePanel } from "./ui/updates.js";

const call = (cmd, args = {}) => globalThis.__TAURI__.core.invoke("api", { cmd, args });

const root = document.documentElement;
root.dataset.skin = "classic";
const theme = matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () => { root.dataset.theme = theme.matches ? "dark" : "light"; };
applyTheme();
theme.addEventListener("change", applyTheme);

// its updates (D137), under the server's panel: installed on its own it updates itself, and
// the copy beside Game Repo says it updates with Game Repo
render(html`<${ServePanel} call=${call} alone=${true} />
  <section class="ls-section" id="server-updates"><h2>Updates</h2><${UpdatePanel} call=${call} settings=${true} /></section>`, document.getElementById("server-root"));
