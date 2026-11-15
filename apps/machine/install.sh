#!/bin/sh
# Installs Meerkat Machine for the current user, then runs it with any given
# arguments. Published with each release; the release fills in VERSION.
#
#   curl -fsSL https://github.com/what1f/meerkat/releases/download/vVERSION/install.sh \
#     | sh -s -- pair URL CODE --cert-sha256 HEX
#
# MEERKAT_INSTALL_DIR changes where the binaries go (default ~/.meerkat/bin).
# MEERKAT_DOWNLOAD_BASE changes where the release files are downloaded from.
set -eu

VERSION="@VERSION@"
BASE="${MEERKAT_DOWNLOAD_BASE:-https://github.com/what1f/meerkat/releases/download/v$VERSION}"
DIR="${MEERKAT_INSTALL_DIR:-$HOME/.meerkat/bin}"
BINARIES="meerkat-machine meerkat meerkat-agent"

fail() {
	printf '%s\n' "$*" >&2
	exit 1
}

case "$VERSION" in
*@*) fail "This install script is not from a Meerkat release." ;;
esac

system="$(uname -s)"
machine="$(uname -m)"
# A shell running under Rosetta reports x86_64 on Apple silicon.
if [ "$system" = Darwin ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
	machine=arm64
fi
case "$system/$machine" in
Darwin/arm64) platform=macos-arm64 ;;
Darwin/x86_64) platform=macos-x86_64 ;;
Linux/x86_64 | Linux/amd64) platform=linux-x86_64 ;;
Linux/aarch64 | Linux/arm64) platform=linux-arm64 ;;
*) fail "Meerkat Machine is not available for $system $machine yet." ;;
esac
if [ "$system" = Linux ] && ! ls /lib*/ld-linux-* /lib/*/ld-linux-* >/dev/null 2>&1; then
	fail "Meerkat Machine needs a Linux system with glibc."
fi

installed() {
	for binary in $BINARIES; do
		[ -x "$DIR/$binary" ] || return 1
	done
	[ "$("$DIR/meerkat-machine" --version 2>/dev/null || true)" = "meerkat-machine $VERSION" ]
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d ' ' -f 1
	elif command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | cut -d ' ' -f 1
	else
		fail "Install sha256sum or shasum, then run the command again."
	fi
}

# Restarts a background Machine so an upgrade takes effect.
restart_background() {
	case "$system" in
	Darwin) launchctl kickstart -k "gui/$(id -u)/com.meerkat.machine.remote" >/dev/null 2>&1 || true ;;
	Linux) systemctl --user try-restart meerkat-machine.service >/dev/null 2>&1 || true ;;
	esac
}

if ! installed; then
	archive="meerkat-machine-$VERSION-$platform"
	temporary="$(mktemp -d)"
	trap 'rm -rf "$temporary"' EXIT INT TERM
	printf 'Installing Meerkat Machine %s…\n' "$VERSION"
	curl -fsSL "$BASE/$archive.tar.gz" -o "$temporary/$archive.tar.gz" ||
		fail "Could not download Meerkat Machine. Check the connection and run the command again."
	curl -fsSL "$BASE/$archive.tar.gz.sha256" -o "$temporary/$archive.sha256" ||
		fail "Could not download Meerkat Machine. Check the connection and run the command again."
	[ "$(cut -d ' ' -f 1 <"$temporary/$archive.sha256")" = "$(sha256 "$temporary/$archive.tar.gz")" ] ||
		fail "The download was damaged. Run the command again."
	tar -xzf "$temporary/$archive.tar.gz" -C "$temporary"
	mkdir -p "$DIR"
	# Renaming over the old files keeps a running Machine on its current copy.
	for binary in $BINARIES; do
		cp "$temporary/$archive/$binary" "$DIR/.$binary.new"
		chmod 0755 "$DIR/.$binary.new"
		mv -f "$DIR/.$binary.new" "$DIR/$binary"
	done
	rm -rf "$temporary"
	trap - EXIT INT TERM
	installed || fail "Meerkat Machine $VERSION does not run on this computer."
	[ "$#" -gt 0 ] || restart_background
fi

if [ "$#" -gt 0 ]; then
	exec "$DIR/meerkat-machine" "$@"
fi
printf 'Meerkat Machine %s is installed in %s\n' "$VERSION" "$DIR"
