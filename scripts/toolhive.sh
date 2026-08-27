#!/usr/bin/env bash
# Sets Fiber up as a containerised MCP server under ToolHive, in one command.
#
#   curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash
#
# Pass a directory to serve a collections repo instead of the desktop app's own
# collections:
#
#   ... | bash -s -- ~/work/api-collections
#
# If Fiber is installed on this machine and you only want it for yourself, you
# do not need any of this — the app is already an MCP server. See the README.
# ToolHive is for the other case: a shared collections repo, a server, a proxy
# and an audit log in front of your agents.

set -euo pipefail

# Overridable for a mirror, a pinned version, or a local build under test.
IMAGE="${FIBER_IMAGE:-ghcr.io/mathiaswp/fiber-mcp:latest}"
NAME="fiber"
# The sealing key lives in ToolHive's store; the credentials it seals live in
# the mounted collections directory, so the app can keep them current.
SECRET_KEY="fiber-key"
SECRETS_FILE="mcp-secrets.enc"

die() {
	echo "$*" >&2
	exit 1
}

command -v thv > /dev/null 2>&1 ||
	die "ToolHive (thv) is not installed. See https://docs.stacklok.com/toolhive."
docker info > /dev/null 2>&1 ||
	die "No container runtime is running. Start Docker (or Podman/Colima) and run this again."

# The collections directory, in order of preference: the argument, then the
# environment, then wherever the desktop app keeps its own.
if [ "$#" -gt 0 ]; then
	data="$1"
elif [ -n "${FIBER_DATA_DIR:-}" ]; then
	data="$FIBER_DATA_DIR"
elif [ "$(uname -s)" = "Darwin" ]; then
	data="$HOME/Library/Application Support/dev.fiber.app"
else
	data="${XDG_DATA_HOME:-$HOME/.local/share}/dev.fiber.app"
fi

[ -d "$data" ] || die "No collections directory at $data — pass one as an argument."
# Not an error: a repo of section files is a perfectly good thing to serve, and
# it may not have been opened by the app yet.
[ -d "$data/sections" ] ||
	echo "Note: $data has no sections/ directory yet, so nothing will be shared."

# The app's binary, if this machine has one. It is only needed to read the
# keychain — a server that just serves a git repo of collections has no
# credentials to export and no app installed. FIBER_BIN points at a build you
# have not installed, which is mostly useful when working on Fiber itself.
if [ -n "${FIBER_BIN:-}" ]; then
	app="$FIBER_BIN"
elif [ "$(uname -s)" = "Darwin" ]; then
	app="/Applications/Fiber.app/Contents/MacOS/fiber"
else
	app="$(command -v fiber || true)"
fi

# A rerun should replace the workload rather than collide with it. `thv rm`
# takes the container away; the collections and the secret both outlive it.
if thv list --all 2> /dev/null | grep -q "^$NAME[[:space:]]"; then
	echo "Replacing the existing '$NAME' workload..."
	thv stop "$NAME" > /dev/null 2>&1 || true
	thv rm "$NAME" > /dev/null 2>&1 || true
fi

secret_args=()
if [ -x "$app" ]; then
	echo "Copying credentials out of the keychain..."
	# Two pieces, and the split is the point. The *key* goes into ToolHive's
	# encrypted store, where it sits unchanged for the life of the workload. The
	# *credentials* go into a file inside the collections directory we are about
	# to mount, sealed with that key — so signing in again in Fiber rewrites the
	# file, the running container reads it on its next 401, and nothing has to be
	# re-exported or restarted. Before this, a container held whatever was true
	# when it started.
	#
	# The key never reaches the mount and the credentials never reach the
	# terminal: each goes straight down a pipe or straight to a 0600 file.
	#
	# `< /dev/null` is not decoration. Under `curl | bash` this script *is*
	# bash's stdin, so a child inherits the rest of it — and a copy of Fiber too
	# old to know these commands would take that for MCP traffic and sit there
	# reading. With stdin closed the worst case is an immediate empty result,
	# which the check below turns into an explanation.
	if ! "$app" mcp file-key < /dev/null | thv secret set "$SECRET_KEY" > /dev/null; then
		echo "Could not store the sealing key." >&2
		echo "  - if ToolHive has no secrets provider yet: run 'thv secret setup'" >&2
		echo "  - if Fiber said nothing about file-key: update it, that command is newer" >&2
		exit 1
	fi
	# FIBER_DATA_DIR so it reports on the collections we are about to serve,
	# which is not the app's own directory when a repo was named.
	if ! FIBER_DATA_DIR="$data" "$app" mcp export-secrets --to "$data/$SECRETS_FILE" \
		< /dev/null; then
		echo "Could not write the credentials file." >&2
		exit 1
	fi
	secret_args=(
		--secret "$SECRET_KEY,target=FIBER_SECRETS_KEY"
		--env "FIBER_SECRETS_FILE=/data/$SECRETS_FILE"
	)
else
	echo "Fiber is not installed here, so there are no credentials to copy."
	echo "Authenticated collections will need FIBER_SECRETS — see deploy/toolhive.md."
fi

echo "Starting $NAME..."
# --transport stdio is not optional. Without it ToolHive assumes the image
# speaks streamable-HTTP, starts the container with no stdin at all, and the
# server reads EOF and exits — over and over, as `unhealthy`.
#
# The two branches are not duplication for its own sake: macOS still ships bash
# 3.2, where expanding an empty array under `set -u` is an unbound-variable
# error rather than nothing at all.
if [ "${#secret_args[@]}" -eq 0 ]; then
	thv run --name "$NAME" --transport stdio --volume "$data:/data" "$IMAGE"
else
	thv run --name "$NAME" --transport stdio --volume "$data:/data" \
		"${secret_args[@]}" "$IMAGE"
fi

echo
echo "Done. '$NAME' is serving $data."
echo "Check it with:  thv list"
echo "Point a client at it with:  thv client setup"
