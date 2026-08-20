#!/usr/bin/env bash
# Installs Fiber on macOS, with no Gatekeeper prompt to click through.
#
#   curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/install-macos.sh | bash
#
# The prompt everyone hits — "Apple could not verify Fiber is free of malware" —
# is not really about the signature. It is about `com.apple.quarantine`, an
# extended attribute that the *downloading application* attaches. Browsers set
# it; curl does not. macOS only asks Gatekeeper about a file that carries it, so
# an app that arrives this way is never questioned.
#
# Nothing here disables or works around a security check. Notarising the app is
# what would make a browser download quiet too, and that needs a paid Apple
# Developer account.

set -euo pipefail

REPO="MathiasWP/fiber"
APP="/Applications/Fiber.app"

if [ "$(uname -s)" != "Darwin" ]; then
	echo "This installer is macOS-only. See the README for Linux and Windows." >&2
	exit 1
fi

# Before anything is fetched: refusing after a 20MB download is rude, and the
# bundle cannot be replaced underneath a running copy anyway.
#
# `ps -Awwo comm=` lists executable paths, one per line, so an exact literal
# whole-line match settles it. `pgrep -f` was tempting and wrong: it runs a
# regex over whole command lines, matching this script's own arguments as
# readily as a running app.
#
# Note the absence of `grep -q`. With `pipefail` set, `-q` makes grep exit at
# the first match, `ps` takes a SIGPIPE, and the pipeline reports 141 — so the
# test reads as false and the guard silently never fires. Letting grep drain
# stdin costs nothing and keeps the exit status honest.
if ps -Awwo comm= | grep -xF "$APP/Contents/MacOS/fiber" > /dev/null; then
	echo "Fiber is running. Quit it first, then run this again." >&2
	exit 1
fi

echo "Finding the latest release..."
# --location because the API hands out a redirect to the CDN.
dmg_url="$(
	curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
		grep -o '"browser_download_url": *"[^"]*universal\.dmg"' |
		cut -d'"' -f4
)"

if [ -z "$dmg_url" ]; then
	echo "No universal .dmg on the latest release. Has it finished publishing?" >&2
	exit 1
fi

version="$(basename "$dmg_url" | sed -E 's/^Fiber_(.+)_universal\.dmg$/\1/')"
echo "Downloading Fiber ${version}..."

# Everything lands in a temp directory that goes away however this exits, so a
# failed install leaves nothing behind.
work="$(mktemp -d)"
mount=""
cleanup() {
	[ -n "$mount" ] && hdiutil detach "$mount" -quiet 2>/dev/null || true
	rm -rf "$work"
}
trap cleanup EXIT

curl -fSL --progress-bar "$dmg_url" -o "$work/Fiber.dmg"

# A private mountpoint rather than /Volumes/Fiber: no clash with a copy the user
# already has mounted, and nothing appears in Finder mid-install.
mount="$work/mnt"
mkdir -p "$mount"
hdiutil attach -nobrowse -quiet -mountpoint "$mount" "$work/Fiber.dmg"

if [ ! -d "$mount/Fiber.app" ]; then
	echo "That .dmg does not contain Fiber.app - refusing to guess." >&2
	exit 1
fi

echo "Installing to ${APP}..."
rm -rf "$APP"
cp -R "$mount/Fiber.app" "$APP"

# The whole point of the exercise, so it is worth checking rather than assuming.
if xattr -p com.apple.quarantine "$APP" > /dev/null 2>&1; then
	echo "Warning: the copy is quarantined after all. Clearing it." >&2
	xattr -dr com.apple.quarantine "$APP"
fi

echo
echo "Fiber $version is installed. Open it from Applications or Spotlight."
echo "It will update itself from here on."
