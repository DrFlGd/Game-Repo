// Game Repo Server's own window (desktop/server): the panel of Game Repo's Settings ›
// Serve to devices (ui/serve.js), talking to the server it's the window of through its
// "api" command. Its tray icon's Pair a device… and Devices… open it.
import { html, render } from "./lib/html.js";
import { ServePanel } from "./ui/serve.js";

const call = (cmd, args = {}) => globalThis.__TAURI__.core.invoke("api", { cmd, args });

const root = document.documentElement;
root.dataset.skin = "classic";
const theme = matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () => { root.dataset.theme = theme.matches ? "dark" : "light"; };
applyTheme();
theme.addEventListener("change", applyTheme);

render(html`<${ServePanel} call=${call} alone=${true} />`, document.getElementById("server-root"));
