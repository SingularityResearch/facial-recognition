"""The app window's graphics caches are cleared once per new version (an old one froze a newer window: a white page),
and what the pages remember is never touched."""
import app


def make(folder, *parts):
    p = folder.joinpath(*parts)
    p.mkdir(parents=True)
    (p / "data").write_text("x")
    return p


def test_a_new_version_clears_the_graphics_caches_and_keeps_what_pages_remember(tmp_path):
    w = tmp_path / "webview"
    gpu = make(w, "GPUCache")                                   # Qt on Linux: straight in the folder
    dawn = make(w, "DawnGraphiteCache")
    deep = make(w, "EBWebView", "Default", "GPUCache")          # WebView2 on Windows
    shader = make(w, "EBWebView", "ShaderCache")
    keep = make(w, "Local Storage", "leveldb")
    session = make(w, "EBWebView", "Default", "Session Storage")
    removed = app.clear_graphics_caches(w, "9.9")
    assert set(removed) == {gpu, dawn, deep, shader}
    assert not gpu.exists() and not deep.exists() and not shader.exists() and not dawn.exists()
    assert (keep / "data").read_text() == "x" and (session / "data").exists()
    assert (w / app.CACHE_VERSION_FILE).read_text() == "9.9"


def test_the_same_version_leaves_everything_alone(tmp_path):
    w = tmp_path / "webview"
    app.clear_graphics_caches(w, "9.9")                          # first start: nothing there yet, notes the version
    gpu = make(w, "GPUCache")
    assert app.clear_graphics_caches(w, "9.9") == [] and gpu.exists()
    assert app.clear_graphics_caches(w, "10.0") == [gpu] and not gpu.exists()


def test_the_macos_window_keeps_its_pace_when_covered(monkeypatch):
    """macOS's window is WebKit, which takes no Chromium flags: with another window over the app's, a page's timers
    ran once a second and nothing was drawn, so the camera reached the other devices at 1 frame a second (the first
    real Mac, 2026-11-19). WebKit's own switches are set on the window's web view, each only where this WebKit has
    it: a missing one is said, never an error."""
    class Part:
        def __init__(self, has):
            self.has, self.set = set(has), {}

        def respondsToSelector_(self, selector):
            return selector in self.has

        def __getattr__(self, name):                 # _setX_ as pyobjc spells the selector _setX:
            if name.startswith("_set") and name.endswith("_"):
                return lambda value: self.set.__setitem__(name, value)
            raise AttributeError(name)

    class View(Part):
        def __init__(self, has, prefs):
            super().__init__(has)
            self.prefs = prefs

        def configuration(self):
            return self

        def preferences(self):
            return self.prefs

    prefs = Part(app.MAC_HIDDEN_PAGES)
    view = View([app.MAC_OCCLUSION], prefs)
    done, missing = app.mac_keep_running(view)
    assert view.set == {"_setWindowOcclusionDetectionEnabled_": False}                 # covered counts as shown
    assert set(prefs.set) == {s.replace(":", "_") for s in app.MAC_HIDDEN_PAGES} and not any(prefs.set.values())
    assert len(done) == 4 and missing == [] and "setWindowOcclusionDetectionEnabled" in done
    older = View([], Part(app.MAC_HIDDEN_PAGES[:1]))                                   # a WebKit without some of them
    done, missing = app.mac_keep_running(older)
    assert done == ["setHiddenPageDOMTimerThrottlingEnabled"] and len(missing) == 3 and older.set == {}
    monkeypatch.setattr(app.sys, "platform", "linux")
    assert app.keep_running_macos(object()) is False                                   # the Mac's window only
    src = (app.HERE / "app.py").read_text("utf-8")                                     # set before any page runs
    assert "window.events.before_show += lambda: keep_running_macos(window)" in src


def test_the_built_app_lists_the_systems_fonts_once():
    """MediaPipe imports matplotlib, which lists every font of the system into its folder; PyInstaller gives the
    built app a new temporary folder at every start, so each start listed them again (over 5 s on a Mac). The built
    app keeps the folder in the private one; from source matplotlib's own stays."""
    env = {"MPLCONFIGDIR": "/tmp/made-by-the-start-up-hook"}
    app.keep_font_list(env, built=False)
    assert env["MPLCONFIGDIR"] == "/tmp/made-by-the-start-up-hook"
    app.keep_font_list(env, built=True)
    assert env["MPLCONFIGDIR"] == str(app.paths.HOME / "matplotlib")
    src = (app.HERE / "app.py").read_text("utf-8")              # set before the programs (and so MediaPipe) load
    assert src.index("\nkeep_font_list()") < src.index("\nTITLE = ")


def test_the_window_keeps_running_in_the_background():
    """Minimized or covered, the window's browser must not slow its timers or stop drawing: the tracker (and the
    camera other devices watch) froze. Flags already set are kept, and nothing is added twice."""
    env = {"QTWEBENGINE_CHROMIUM_FLAGS": "--ignore-gpu-blocklist"}
    app.keep_running_in_background(env)
    app.keep_running_in_background(env)
    for var in ("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", "QTWEBENGINE_CHROMIUM_FLAGS"):
        assert env[var].count("--disable-background-timer-throttling") == 1
        assert "--disable-backgrounding-occluded-windows" in env[var]
    assert env["QTWEBENGINE_CHROMIUM_FLAGS"].startswith("--ignore-gpu-blocklist ")


def test_the_windows_net_parts_are_loaded_by_full_name_first_and_by_file_when_that_fails(monkeypatch):
    """pywebview asks for them by short name, which failed at some first starts of a new build (no window, the app
    in the browser). By full name, then by the file itself; and whatever fails is said in the log."""
    asked, said = [], []
    assert app.preload_window_parts(say=said.append, add=asked.append, sleep=lambda s: None) == 4
    assert [a.split(",")[0] for a in asked] == [name for name, _ in app.WINDOW_PARTS] and not said
    assert all("Version=4.0.0.0" in a and "PublicKeyToken=" in a for a in asked)        # full names: no guessing

    def flaky(what):
        asked.append(what)
        if what.startswith("System.Reflection,"):
            raise FileNotFoundError("Could not load file or assembly 'System.Reflection' or one of its dependencies.")
    asked.clear()
    assert app.preload_window_parts(say=said.append, add=flaky, sleep=lambda s: None) == 4
    assert asked[-1].endswith(r"GAC_MSIL\System.Reflection\v4.0_4.0.0.0__b03f5f7f11d50a3a\System.Reflection.dll")
    assert len(said) == 1 and "loaded by its file after its full name failed" in said[0]

    def never(what):
        raise FileNotFoundError("gone")
    said.clear()
    assert app.preload_window_parts(say=said.append, add=never, sleep=lambda s: None) == 0
    assert len(said) == 4 and all("couldn't be loaded" in s for s in said)              # the window is still tried
    monkeypatch.setattr(app.sys, "platform", "linux")
    assert app.preload_window_parts(say=said.append) == 0                               # nothing to do elsewhere
