"""Game Repo Go's artwork for Steam's library (the "Folder pad" mark of tools/make_icons.py,
in Go's yellow and navy): the portrait grid capsule, the wide capsule, the hero, the logo
and the icon, at the sizes Steam asks for.

    python tools/make_steam_art.py            # writes desktop/client/steam/*.png

It draws each picture as a page and photographs it with Playwright's Chromium
(pip install playwright; python -m playwright install chromium), with the app's own
font, Archivo, from Google Fonts, so it needs the internet. The PNGs are committed:
Game Repo Go puts them on its own Steam shortcut (Linux), and each release carries them
as `Game Repo Go_<version>_steam-artwork.zip` for anyone adding it to Steam by hand."""
import os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from make_icons import NAVY, YEL, WHITE, mark  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "desktop", "client", "steam")
DARK_YEL = "#e2a800"   # the pattern on yellow
NAVY_2 = "#223040"     # the pattern on navy
FONT = "https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@75..100,400..900&display=block"

# Steam's names for them, as Game Repo Go writes them in Steam's grid folder (<appid><suffix>)
ART = {
    # file: (width, height, transparent)
    "cover.png": (600, 900, False),    # <appid>p.png, the portrait capsule
    "banner.png": (920, 430, False),   # <appid>.png, the wide capsule
    "hero.png": (1920, 620, False),    # <appid>_hero.png, behind the game's page
    "logo.png": (1080, 400, True),     # <appid>_logo.png, over the hero
    "icon.png": (256, 256, True),      # <appid>_icon.png and the shortcut's icon
}


def mark_svg(size, x=0, y=0, extra=""):
    """The mark alone (folder, pad, buttons, download badge), as Go's icon draws it."""
    return (f'<svg x="{x}" y="{y}" width="{size}" height="{size}" viewBox="6 10 58 54" {extra}>'
            f'{mark("go")}</svg>')


def pattern(pid, fill, step=96, scale=1.0, opacity=1.0):
    """A wallpaper of small d-pads and buttons, the mark's own shapes."""
    s = scale
    return (f'<pattern id="{pid}" width="{step}" height="{step}" patternUnits="userSpaceOnUse" '
            f'patternTransform="rotate(-12)">'
            f'<g fill="{fill}" opacity="{opacity}" transform="scale({s})">'
            f'<path d="M14 22h8v-8h8v8h8v8h-8v8h-8v-8h-8z"/>'
            f'<circle cx="66" cy="62" r="5"/><circle cx="78" cy="74" r="5"/></g></pattern>')


def page(w, h, body, transparent=False):
    bg = "transparent" if transparent else NAVY
    return f"""<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="{FONT}">
<style>
html, body {{ margin: 0; width: {w}px; height: {h}px; overflow: hidden; background: {bg}; }}
.word {{ font-family: Archivo, sans-serif; font-variation-settings: "wdth" 75; font-weight: 900;
  line-height: .86; letter-spacing: -.01em; white-space: nowrap; }}
.small {{ font-weight: 800; letter-spacing: .02em; }}
</style></head><body>{body}</body></html>"""


def cover():
    w, h = 600, 900
    return page(w, h, f"""
<svg width="{w}" height="{h}" style="position:absolute;inset:0">
  <defs>{pattern("p", DARK_YEL, step=110)}</defs>
  <rect width="{w}" height="{h}" fill="{YEL}"/>
  <rect width="{w}" height="{h}" fill="url(#p)" opacity=".55"/>
  <rect y="{h - 330}" width="{w}" height="330" fill="{NAVY}"/>
  {mark_svg(440, 80, 92)}
</svg>
<div class="word" style="position:absolute;left:48px;bottom:200px;font-size:72px;color:{WHITE}">Game Repo</div>
<div class="word" style="position:absolute;left:44px;bottom:28px;font-size:200px;color:{YEL}">Go</div>
""")


def banner():
    w, h = 920, 430
    return page(w, h, f"""
<svg width="{w}" height="{h}" style="position:absolute;inset:0">
  <defs>{pattern("p", DARK_YEL, step=110)}</defs>
  <rect width="{w}" height="{h}" fill="{YEL}"/>
  <rect width="{w}" height="{h}" fill="url(#p)" opacity=".55"/>
  <path d="M430 0H{w}V{h}H350z" fill="{NAVY}"/>
  {mark_svg(330, 40, 46)}
</svg>
<div class="word" style="position:absolute;left:470px;top:96px;font-size:84px;color:{WHITE}">Game Repo</div>
<div class="word" style="position:absolute;left:462px;top:178px;font-size:190px;color:{YEL}">Go</div>
""")


def hero():
    # no words: Steam puts the logo over it, bottom left by default
    w, h = 1920, 620
    return page(w, h, f"""
<svg width="{w}" height="{h}" style="position:absolute;inset:0">
  <defs>
    {pattern("p", NAVY_2, step=120)}
    <linearGradient id="fade" x1="0" x2="1"><stop offset="0" stop-color="{NAVY}"/><stop offset=".55" stop-color="{NAVY}" stop-opacity="0"/></linearGradient>
  </defs>
  <rect width="{w}" height="{h}" fill="{NAVY}"/>
  <rect width="{w}" height="{h}" fill="url(#p)"/>
  <path d="M1180 0H{w}V{h}H1020z" fill="{YEL}"/>
  <path d="M1120 0H1150L990 {h}H960z" fill="{YEL}"/>
  <rect width="{w}" height="{h}" fill="url(#fade)"/>
  {mark_svg(560, 1230, 40)}
</svg>
""")


def logo():
    w, h = 1080, 400
    tile = (f'<svg x="0" y="0" width="{h}" height="{h}" viewBox="0 0 64 64">'
            f'<rect width="64" height="64" rx="14" fill="{YEL}"/>{mark("go")}</svg>')
    return page(w, h, f"""
<svg width="{w}" height="{h}" style="position:absolute;inset:0">{tile}</svg>
<div class="word" style="position:absolute;left:440px;top:52px;font-size:132px;color:{WHITE};
  text-shadow:0 3px 10px rgba(0,0,0,.45)">Game Repo</div>
<div class="word" style="position:absolute;left:434px;top:166px;font-size:232px;color:{YEL};
  text-shadow:0 3px 10px rgba(0,0,0,.45)">Go</div>
""", transparent=True)


def icon():
    w = 256
    return page(w, w, f"""
<svg width="{w}" height="{w}" viewBox="0 0 64 64" style="position:absolute;inset:0">
  <rect width="64" height="64" rx="14" fill="{YEL}"/>{mark("go")}</svg>
""", transparent=True)


def main():
    from playwright.sync_api import sync_playwright
    os.makedirs(OUT, exist_ok=True)
    pages = {"cover.png": cover, "banner.png": banner, "hero.png": hero, "logo.png": logo, "icon.png": icon}
    with sync_playwright() as p:
        b = p.chromium.launch(executable_path=os.environ.get("CHROMIUM") or None)   # CHROMIUM: a Chromium of your own
        for name, (w, h, transparent) in ART.items():
            pg = b.new_page(viewport={"width": w, "height": h}, device_scale_factor=1)
            pg.set_content(pages[name](), wait_until="networkidle")
            if not pg.evaluate("document.fonts.load('900 72px Archivo').then(f => f.length > 0)"):
                sys.exit("Archivo didn't load (no internet?): the pictures would have the wrong font")
            path = os.path.join(OUT, name)
            pg.screenshot(path=path, omit_background=transparent)
            print(f"{os.path.relpath(path, ROOT)}: {w}x{h}")
        b.close()


if __name__ == "__main__":
    main()
