#!/bin/sh
# Face Recognizer for Linux: installs the app for this user (no sudo) and gives it icons.
#
#   sh FaceRecognizer-linux-x64.run               install, or update: Face Recognizer is then in the applications
#                                                 menu and on the desktop
#   sh FaceRecognizer-linux-x64.run --no-desktop  the same, without the desktop icon
#   sh FaceRecognizer-linux-x64.run --uninstall   remove the app and its icons (also: sh ~/.local/lib/FaceRecognizer/uninstall.sh)
#
# Where things go:   the app            ~/.local/lib/FaceRecognizer
#                    the menu entry     ~/.local/share/applications/FaceRecognizer.desktop
#                    its icons          ~/.local/share/icons/hicolor/<size>/apps/FaceRecognizer.png
#                    a command          ~/.local/bin/facerecognizer
# Never touched:     the face models and your data (faces, names, labels) in ~/.local/share/FaceRecognizer, and what
#                    is private to you in ~/.local/share/FaceLabels. Uninstalling leaves both.
#
# Built by "python build_exe.py --installer": this script, the line __APP__, then the app's folder as a .tar.gz.
set -eu
NAME=FaceRecognizer
TITLE="Face Recognizer"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
APP="$HOME/.local/lib/$NAME"
BIN="$HOME/.local/bin"
ENTRY="$SHARE/applications/$NAME.desktop"
ICONS="$SHARE/icons/hicolor"
SIZES="64 256 512"

desktop_dir() {
  d="$(xdg-user-dir DESKTOP 2>/dev/null || true)"
  [ -n "$d" ] && [ "$d" != "$HOME" ] || d="$HOME/Desktop"
  printf '%s' "$d"
}

refresh() {                         # the desktop reads its menu and icons again; fine if these tools aren't there
  update-desktop-database "$SHARE/applications" >/dev/null 2>&1 || true
  gtk-update-icon-cache -q -t -f "$ICONS" >/dev/null 2>&1 || true
}

running() {                         # the installed copy is running: its files can't be replaced or removed under it
  for pid in $(pgrep -u "$(id -u)" -x "$NAME" 2>/dev/null); do
    case "$(readlink "/proc/$pid/exe" 2>/dev/null)" in "$APP"/*) return 0 ;; esac
  done
  return 1
}

uninstall() {
  if running; then echo "$TITLE is running: close it first, then run this again."; exit 1; fi
  rm -rf "$APP"
  rm -f "$ENTRY" "$BIN/facerecognizer" "$(desktop_dir)/$NAME.desktop"
  for s in $SIZES; do rm -f "$ICONS/${s}x${s}/apps/$NAME.png"; done
  refresh
  echo "$TITLE was removed. Its face models and your data were left in $SHARE/$NAME"
  echo "(and what is private to you in $SHARE/FaceLabels): delete those folders yourself if you no longer want them."
}

MODE=install
DESKTOP=yes
case "$0" in *uninstall.sh) MODE=uninstall ;; esac
for arg in "$@"; do
  case "$arg" in
    --uninstall) MODE=uninstall ;;
    --no-desktop) DESKTOP=no ;;
    -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg (see --help)"; exit 2 ;;
  esac
done
if [ "$MODE" = uninstall ]; then uninstall; exit 0; fi

[ "$(uname -s)" = Linux ] || { echo "This installer is for Linux."; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo "This build is for 64-bit Intel and AMD computers (x86_64); this one is $(uname -m)."; exit 1; }
START="$(awk '/^__APP__$/ { print NR + 1; exit }' "$0")"
[ -n "$START" ] || { echo "This file holds no app (it is only the installer's script)."; exit 1; }
if running; then echo "$TITLE is running: close it first, then run this again."; exit 1; fi

echo "Installing $TITLE into $APP ..."
NEW="$APP.new"
rm -rf "$NEW"
mkdir -p "$NEW" "$HOME/.local/lib"
tail -n +"$START" "$0" | tar xzf - -C "$NEW"
[ -x "$NEW/$NAME/$NAME" ] || { rm -rf "$NEW"; echo "The download is damaged (the app isn't inside it): download it again."; exit 1; }
rm -rf "$APP"
mv "$NEW/$NAME" "$APP"
rmdir "$NEW"
awk '/^__APP__$/ { exit } { print }' "$0" > "$APP/uninstall.sh"       # this script without the app: it only removes
chmod +x "$APP/uninstall.sh"

for s in $SIZES; do
  src="$APP/_internal/icons/icon-$s.png"
  [ "$s" = 512 ] && src="$APP/_internal/icons/icon.png"
  if [ -f "$src" ]; then
    mkdir -p "$ICONS/${s}x${s}/apps"
    cp "$src" "$ICONS/${s}x${s}/apps/$NAME.png"
  fi
done
ICON="$ICONS/512x512/apps/$NAME.png"
[ -f "$ICON" ] || ICON="$NAME"

mkdir -p "$SHARE/applications" "$BIN"
cat > "$ENTRY" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=$TITLE
Comment=Recognize the people you have named, live from your camera, and label the faces in your photos
Exec="$APP/$NAME"
Icon=$ICON
Terminal=false
Categories=Graphics;Photography;
Keywords=face;faces;camera;recognition;photos;
StartupNotify=true
StartupWMClass=$NAME
EOF
chmod +x "$ENTRY"
ln -sf "$APP/$NAME" "$BIN/facerecognizer"

if [ "$DESKTOP" = yes ]; then
  DESK="$(desktop_dir)"
  if [ -d "$DESK" ]; then
    cp "$ENTRY" "$DESK/$NAME.desktop"
    chmod +x "$DESK/$NAME.desktop"
    gio set "$DESK/$NAME.desktop" metadata::trusted true >/dev/null 2>&1 || true   # GNOME: no "Allow Launching" step
  fi
fi
refresh

echo "Done. Start it from the applications menu ($TITLE)$([ "$DESKTOP" = yes ] && [ -d "$(desktop_dir)" ] && printf ', the desktop icon') or with: facerecognizer"
if [ -d "$SHARE/$NAME/models" ]; then
  echo "It uses the face models and your data in $SHARE/$NAME, as before."
else
  echo "On first start it downloads the face models into $SHARE/$NAME, where your data is kept too."
fi
echo "To remove it: sh $APP/uninstall.sh"
exit 0                              # the shell must stop here: the packed app follows
