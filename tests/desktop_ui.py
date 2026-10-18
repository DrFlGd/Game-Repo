#!/usr/bin/env python3
"""Drive the built desktop app through tauri-driver (WebDriver) and save screenshots.

    tauri-driver &                     # needs WebKitWebDriver (Linux) or msedgedriver (Windows)
    python3 tests/desktop_ui.py --app desktop/target/release/game-repo --out shots

Checks: the interface loads in the real app (the desktop platform, not the plain
web page), the first start asks where the library goes and, taking the
suggestion, makes it at ~/Game Repo (with only _library/ in it), the
settings page shows it, and the theme setting reaches night. Exits non-zero on any failure.
"""
import argparse
import json
import sys
import time
from pathlib import Path

from selenium import webdriver
from selenium.webdriver.common.options import ArgOptions

ap = argparse.ArgumentParser()
ap.add_argument("--app", required=True)
ap.add_argument("--out", default="shots")
ap.add_argument("--library", help="expected library folder (default: ~/Game Repo)")
ap.add_argument("--driver", default="http://127.0.0.1:4444")
a = ap.parse_args()
out = Path(a.out)
out.mkdir(parents=True, exist_ok=True)
library = Path(a.library) if a.library else Path.home() / "Game Repo"

opts = ArgOptions()
opts.set_capability("browserName", "wry")
opts.set_capability("tauri:options", {"application": str(Path(a.app).resolve())})
print("starting a session…", flush=True)
d = webdriver.Remote(command_executor=a.driver, options=opts)
print("session started", flush=True)
d.set_script_timeout(30)
try:
    d.set_window_size(1400, 900)
except Exception as e:  # some drivers can't resize
    print("resize:", e, flush=True)
failures, report = [], {}


def shot(name):
    d.save_screenshot(str(out / f"{name}.png"))


def wait(js, timeout=60, what=""):
    print(f"waiting for {what or js[:60]}…", flush=True)
    end = time.time() + timeout
    while time.time() < end:
        try:
            if d.execute_script(f"return !!({js})"):
                return True
        except Exception:
            pass
        time.sleep(0.25)
    failures.append(f"timed out waiting for {what or js}")
    try:
        shot(f"timeout-{len(failures):02d}")
        print("page says:", d.execute_script("return document.body.innerText.slice(0, 400)"), flush=True)
        print("errors:", d.execute_script("return window.__errors || []"), flush=True)
        # A static module-import failure prevents app.js from running at all, so
        # window.__errors is never registered. Diagnose that separately from an
        # initialized page that later fails to render.
        print("startup:", d.execute_script("""return {
            url: location.href, readyState: document.readyState,
            errorsHook: typeof window.__errors,
            platform: typeof window.__gamerepo,
            tauri: !!window.__TAURI_INTERNALS__,
            mainHTML: (document.querySelector('#main-root')?.innerHTML || '').slice(0, 300),
            scripts: [...document.querySelectorAll('script[src]')].map(s => s.src),
            resources: performance.getEntriesByType('resource').filter(e => /app\\.js|ui\\//.test(e.name))
                .slice(0, 12).map(e => ({name:e.name, duration:Math.round(e.duration), size:e.transferSize}))
        }"""), flush=True)
        try:
            # Force the existing root module to resolve, reporting its actual
            # syntax/import failure if the original module graph did not load.
            module = d.execute_async_script("""
                const finish = arguments[arguments.length - 1];
                import('./app.js').then(
                    () => finish({loaded:true}),
                    e => finish({loaded:false, error:String(e), stack:String(e?.stack || '')})
                );
            """)
            print("module probe:", module, flush=True)
        except Exception as e:
            print("module probe unavailable:", str(e)[:500], flush=True)
        # Isolate MIME failures in the static module graph. A bare TypeError
        # does not tell us which import URL WebKitGTK rejected.
        try:
            probes = d.execute_async_script("""
                const done = arguments[arguments.length - 1];
                const urls = [
                    'app.js', 'platform.js', 'ui/context.js', 'ui/shell.js',
                    'ui/library.js', 'ui/home.js', 'lib/html.js',
                    'vendor/preact/src/index.js', 'vendor/htm/index.mjs'
                ];
                Promise.all(urls.map(async path => {
                    const url = new URL(path, location.href + '/').href;
                    try {
                        const r = await fetch(url);
                        const text = await r.text();
                        return {path, status:r.status, type:r.headers.get('content-type'),
                            bytes:text.length, prefix:text.slice(0, 35)};
                    } catch(e) { return {path, error:String(e)}; }
                })).then(done, e => done([{error:String(e)}]));
            """)
            print("module MIME responses:", probes, flush=True)
        except Exception as e:
            print("module MIME probe unavailable:", str(e)[:500], flush=True)
    except Exception as e:
        print("couldn't inspect the page:", e, flush=True)
    return False


try:
    if not wait("document.querySelector('#first-run')", 90, "the first-start screen"):
        raise SystemExit("the interface never appeared; skipping the rest")
    time.sleep(0.5)
    shot("00-first-run")
    # the first start asks where the library goes: take the suggested folder
    d.execute_script("document.querySelector('#use-default').click()")
    if not wait("document.querySelector('#home-head h1') && !document.querySelector('#first-run')", 30, "the home page"):
        raise SystemExit("the library wasn't made; skipping the rest")
    time.sleep(0.5)
    shot("00-home")
    report["platform"] = d.execute_script("return window.__gamerepo?.platform?.kind || ''")
    report["title"] = d.execute_script("return document.querySelector('#home-head h1').textContent")
    print("platform:", report["platform"], "| library:", report["title"], flush=True)
    if report["platform"] != "desktop":
        failures.append(f"platform: {report['platform']}")

    # the library folder, made where the first start suggested
    made = {"_library": (library / "_library").is_dir(), "library.json": (library / "_library/library.json").is_file(),
            "nothing else": sorted(x.name for x in library.iterdir()) == ["_library"] if library.is_dir() else False}
    report["library"] = {"path": str(library), **made}
    if not all(made.values()):
        failures.append(f"library folder not made: {made}")

    # settings show it
    d.execute_script("location.hash = '#/settings'")
    if wait("document.querySelector('#library-path')", 20, "the settings page"):
        report["settings_path"] = d.execute_script("return document.querySelector('#library-path').textContent")
        if Path(report["settings_path"]).resolve() != library.resolve():
            failures.append(f"settings show {report['settings_path']}")
    # the index, made on this computer for the library
    if wait("document.querySelector('#index-path')", 20, "the index's place"):
        report["index_path"] = d.execute_script("return document.querySelector('#index-path').textContent")
        if not Path(report["index_path"]).is_file() or str(library) in report["index_path"]:
            failures.append(f"index not made outside the library: {report['index_path']}")
    shot("01-settings")

    # the theme, chosen in Settings › Appearance
    d.execute_script("const s = document.querySelector('#theme-select'); s.value = 'night'; s.dispatchEvent(new Event('change', { bubbles: true }))")
    time.sleep(0.3)
    report["theme"] = d.execute_script("return document.documentElement.dataset.theme")
    if report["theme"] != "night":
        failures.append(f"the theme setting didn't reach night: {report['theme']}")
    time.sleep(0.5)
    shot("02-night")
    errors = d.execute_script("return (window.__errors || []).slice(0, 20)")
    if errors:
        failures.extend(errors)
finally:
    (out / "desktop-ui.json").write_text(json.dumps({"report": report, "failures": failures}, indent=1))
    d.quit()

for f in failures:
    print("FAILED:", f)
sys.exit(1 if failures else 0)
