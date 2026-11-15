"""
desktop.py - the few things the app asks of the operating system, on Windows, macOS and Linux: show a folder in the
file manager, draw the user's eye to the app window when an alert comes in (its taskbar button flashes on
Windows, its Dock icon bounces on macOS, its window is marked urgent on Linux), and say whether the screen is
locked. Each does nothing, quietly, where the system or the window toolkit can't; nothing here is needed for the
app to work.
"""
import os
import struct
import subprocess
import sys
import traceback


def screen_locked(platform=sys.platform, info=None):
    """True while this desktop session's screen is locked, False while it isn't, None where that can't be told
    (macOS, Linux, or Windows didn't say). On Windows a locked session gets no camera: a running one's picture
    stops where it was and a start never finishes (seen 2026-11-15), so the tracker page waits for the unlock
    instead of loading itself and the app again (/api/screen)."""
    if platform != "win32":
        return None
    try:
        return _session_locked((info or _session_info)())
    except Exception:                                         # asked every 10 s while a camera is down: no log line
        return None


def _session_locked(info):
    """Locked or not, from the start of Windows' WTSINFOEXW for a session: Level (4 bytes, then 4 unused), and
    level 1's SessionId, SessionState and SessionFlags (0 locked, 1 unlocked, anything else unknown)."""
    if len(info) < 20 or struct.unpack_from("<I", info, 0)[0] != 1:
        return None
    return {0: True, 1: False}.get(struct.unpack_from("<i", info, 16)[0])


def _session_info():
    import ctypes
    from ctypes import wintypes
    wts = ctypes.WinDLL("wtsapi32")
    wts.WTSQuerySessionInformationW.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.c_int,
                                                ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.DWORD)]
    wts.WTSQuerySessionInformationW.restype = wintypes.BOOL
    wts.WTSFreeMemory.argtypes = [ctypes.c_void_p]
    buf, n = ctypes.c_void_p(), wintypes.DWORD()
    # this computer, this session (WTS_CURRENT_SESSION), WTSSessionInfoEx
    if not wts.WTSQuerySessionInformationW(None, 0xFFFFFFFF, 25, ctypes.byref(buf), ctypes.byref(n)):
        return b""
    try:
        return ctypes.string_at(buf, min(n.value, 20))
    finally:
        wts.WTSFreeMemory(buf)


def open_folder(path, platform=sys.platform, run=subprocess.Popen):
    """Show a folder in the system's file manager."""
    path = str(path)
    if platform == "win32":
        os.startfile(path)                                    # noqa: S606 - a folder of ours, not user input
    elif platform == "darwin":
        run(["open", path])
    else:
        run(["xdg-open", path])


def attention(window, platform=sys.platform):
    """Draw the user's eye to the app window (pywebview window): until it's brought to the front where the system
    allows it. Errors are printed, never raised."""
    try:
        if platform == "win32":
            _flash_windows(window)
        elif platform == "darwin":
            from AppKit import NSApp                          # pyobjc, which pywebview's macOS window uses
            NSApp.requestUserAttention_(0)                    # NSCriticalRequest: bounces until the app is active
        else:
            native = getattr(window, "native", None)
            if hasattr(native, "set_urgency_hint"):           # GTK window
                native.set_urgency_hint(True)
            else:                                             # Qt window
                from qtpy.QtWidgets import QApplication
                QApplication.alert(native)
    except Exception:
        traceback.print_exc()


def _flash_windows(window):
    import ctypes
    from ctypes import wintypes

    class FLASHWINFO(ctypes.Structure):
        _fields_ = [("cbSize", wintypes.UINT), ("hwnd", wintypes.HWND), ("dwFlags", wintypes.DWORD),
                    ("uCount", wintypes.UINT), ("dwTimeout", wintypes.DWORD)]
    hwnd = int(window.native.Handle.ToInt64())
    # FLASHW_ALL | FLASHW_TIMERNOFG: caption and taskbar button, until the window comes to the front
    info = FLASHWINFO(ctypes.sizeof(FLASHWINFO), hwnd, 0x3 | 0xC, 0, 0)
    ctypes.windll.user32.FlashWindowEx(ctypes.byref(info))
