"""The app icons: Game Repo, Game Repo Server and Game Repo Go (the "Folder pad" mark,
Jerred's pick on 2026-10-20). A folder with a controller's d-pad and buttons cut in;
each app has its own tile colour, Server a signal badge and Go a download badge.

    python tools/make_icons.py                      # writes the SVG sources and favicons
    python tools/make_icons.py --render <tauri>     # also renders the PNG/ICO sets with `tauri icon`

<tauri> is the Tauri CLI (npm install @tauri-apps/cli@2.12.3, then node_modules/.bin/tauri).
Game Repo Go's Android launcher icon is made in CI from desktop/client/icons/android.json."""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NAVY, YEL, WHITE, TEAL = "#18222d", "#f2b705", "#ffffff", "#0e6b62"
APPS = {
    # tile, folder, badge, badge sign; where its icons live
    "repo":   (NAVY, YEL, None, None, "desktop/src-tauri/icons", "web/favicon.svg"),
    "server": (TEAL, YEL, YEL, TEAL, "desktop/server/icons", "web/favicon-server.svg"),
    "go":     (YEL, NAVY, NAVY, YEL, "desktop/client/icons", "web/favicon-go.svg"),
}
KEEP = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.png", "icon.ico"]


def mark(app):
    tile, folder, badge, sign, _, _ = APPS[app]
    body = (f'<path d="M10 18a3 3 0 0 1 3-3h12l4 5h22a3 3 0 0 1 3 3v24a3 3 0 0 1-3 3H13a3 3 0 0 1-3-3z" fill="{folder}"/>'
            f'<path d="M20 28h5v-5h5v5h5v5h-5v5h-5v-5h-5z" fill="{tile}"/>'
            f'<circle cx="41" cy="28.5" r="3" fill="{tile}"/><circle cx="46" cy="34.5" r="3" fill="{tile}"/>')
    if badge is None:
        return body
    if app == "server":  # a signal: a dot and two arcs
        g = (f'<g fill="none" stroke="{sign}" stroke-width="2.4" stroke-linecap="round">'
             f'<circle cx="48" cy="50.7" r="1.8" fill="{sign}" stroke="none"/>'
             f'<path d="M43.85 48.3A4.8 4.8 0 0 1 52.15 48.3"/><path d="M39.9 46.05A9.3 9.3 0 0 1 56.1 46.05"/></g>')
    else:  # a download arrow
        g = (f'<g fill="none" stroke="{sign}" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round">'
             f'<path d="M48 41V52.9"/><path d="M43.1 48L48 52.9L52.9 48"/></g>')
    return (f'<g transform="translate(-3 -4)">{body}</g>'
            f'<circle cx="48" cy="48" r="13.5" fill="{tile}"/><circle cx="48" cy="48" r="11" fill="{badge}"/>{g}')


def svg(inner):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">{inner}</svg>\n'


def icon_svg(app):
    return svg(f'<rect width="64" height="64" rx="14" fill="{APPS[app][0]}"/>{mark(app)}')


def android_fg_svg(app):
    # the mark alone, inside the middle 61% that every launcher's mask shows (66 of 108 dp)
    return svg(f'<g transform="translate(32 32) scale(.56) translate(-34.2 -36.3)">{mark(app)}</g>')


def write(rel, text):
    path = os.path.join(ROOT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", newline="\n") as f:
        f.write(text)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--render", metavar="TAURI", help="the Tauri CLI, to render the PNG and ICO sets")
    a = ap.parse_args()
    for app, (tile, *_, folder, favicon) in APPS.items():
        write(f"{folder}/icon.svg", icon_svg(app))
        write(favicon, icon_svg(app))
    write("desktop/client/icons/android-fg.svg", android_fg_svg("go"))
    write("desktop/client/icons/android.json", json.dumps(
        {"default": "icon.svg", "bg_color": YEL, "android_fg": "android-fg.svg", "android_fg_scale": 100}, indent=2) + "\n")
    if not a.render:
        return
    for app, (*_, folder, _fav) in APPS.items():
        with tempfile.TemporaryDirectory() as out:
            subprocess.run([a.render, "icon", os.path.join(ROOT, folder, "icon.svg"), "-o", out],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            for name in KEEP:
                shutil.copy(os.path.join(out, name), os.path.join(ROOT, folder, name))
        print(f"{folder}: {', '.join(KEEP)}")


if __name__ == "__main__":
    sys.exit(main())
