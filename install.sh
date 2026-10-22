#!/bin/sh
# Install or update rgrip from the latest GitHub release:
#   curl -fsSL https://raw.githubusercontent.com/yoouTaro/rgrip/main/install.sh | sh
# Puts rgrip.app in /Applications (or ~/Applications if that isn't writable,
# or $INSTALL_DIR if set).
set -eu

APP=rgrip
REPO=yoouTaro/$APP

say() { printf '%s\n' "$*"; }
fail() { say "error: $*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || fail "$APP is a macOS app."
[ "$(uname -m)" = arm64 ] || fail "$APP is built for Apple silicon. On other Macs, build from source: https://github.com/$REPO"

# The releases feed lists pre-releases too and has no API rate limit.
feed=$(curl -fsSL "https://github.com/$REPO/releases.atom") || fail "could not reach GitHub."
entry=$(printf '%s' "$feed" | awk '/<entry>/{n++} n==1' | sed '/<\/entry>/q')
tag=$(printf '%s' "$entry" | sed -n 's#.*/releases/tag/\([^"]*\)".*#\1#p' | head -1)
[ -n "$tag" ] || fail "no release found."
version=${tag#v}
sha=$(printf '%s' "$entry" | grep -oE 'SHA-256: (&lt;code&gt;|`)?[0-9a-f]{64}' | grep -oE '[0-9a-f]{64}' | head -1 || true)

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
zip="$tmp/$APP.zip"
say "Downloading ${APP} ${version}…"
curl -fL --progress-bar -o "$zip" "https://github.com/$REPO/releases/download/$tag/$APP-$version-macos-arm64.zip" || fail "download failed."

if [ -n "$sha" ]; then
  got=$(shasum -a 256 "$zip" | cut -d' ' -f1)
  [ "$got" = "$sha" ] || fail "checksum mismatch (expected $sha, got $got)."
  say "Checksum OK."
fi

ditto -x -k "$zip" "$tmp" || fail "could not unzip."
[ -d "$tmp/$APP.app" ] || fail "$APP.app not found in the download."

dest=${INSTALL_DIR:-/Applications}
[ -n "${INSTALL_DIR:-}" ] && mkdir -p "$dest"
[ -w "$dest" ] || { dest="$HOME/Applications"; mkdir -p "$dest"; }

# Quit the copy being replaced, if it's running.
if pgrep -fq "$dest/$APP.app/Contents/MacOS/$APP"; then
  say "Quitting the running ${APP}…"
  osascript -e "quit app \"$dest/$APP.app\"" >/dev/null 2>&1 || pkill -f "$dest/$APP.app/Contents/MacOS/$APP" || true
  sleep 1
fi
rm -rf "$dest/$APP.app"
mv "$tmp/$APP.app" "$dest/"
# Not notarized: clear the quarantine flag so Gatekeeper doesn't block it.
xattr -dr com.apple.quarantine "$dest/$APP.app" 2>/dev/null || true

say "Installed $APP $version in $dest."
open "$dest/$APP.app"
