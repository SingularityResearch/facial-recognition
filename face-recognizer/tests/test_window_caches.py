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
