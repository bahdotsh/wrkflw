#!/bin/sh
# Install wrkflw's prebuilt binary from a GitHub release, without Rust.
#
#   curl -fsSL https://raw.githubusercontent.com/bahdotsh/wrkflw/main/scripts/install.sh | sh
#
# WRKFLW_VERSION   a release tag such as v0.9.0 (default: the latest release)
# WRKFLW_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#
# The archive is checked against the release's SHA256SUMS; a release
# published without one is refused unless WRKFLW_NO_VERIFY=1.
# Linux on x86_64 gets the static musl build, which runs on any distribution.
# On Windows, download wrkflw-x86_64-pc-windows-msvc.zip from the release page.

set -eu

# Everything runs from main, called on the last line, so a download cut
# short runs nothing.
main() {
  REPO="bahdotsh/wrkflw"
  VERSION="${WRKFLW_VERSION:-latest}"
  INSTALL_DIR="${WRKFLW_INSTALL_DIR:-$HOME/.local/bin}"

  say() { printf 'wrkflw-install: %s\n' "$*" >&2; }
  die() { say "$*"; exit 1; }

  need() { command -v "$1" >/dev/null 2>&1 || die "$1 is required"; }
  need uname
  need tar
  need mkdir

  if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
  elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
  else
    die "curl or wget is required"
  fi

  os=$(uname -s)
  arch=$(uname -m)
  case "$os/$arch" in
    Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-musl ;;
    Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    Darwin/arm64 | Darwin/aarch64) target=aarch64-apple-darwin ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    *) die "no prebuilt binary for $os on $arch: install with \`cargo install wrkflw\`" ;;
  esac

  if [ "$VERSION" = latest ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/$VERSION"
  fi
  archive="wrkflw-$target.tar.gz"

  tmp=$(mktemp -d)
  # A trap on INT or TERM replaces the default exit, so exit from it;
  # the EXIT trap then removes the download.
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  say "downloading $archive ($VERSION)"
  fetch "$base/$archive" "$tmp/$archive" || die "cannot download $base/$archive"

  if fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" 2>/dev/null; then
    want=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
    [ -n "$want" ] || die "SHA256SUMS lists no $archive"
    if command -v sha256sum >/dev/null 2>&1; then
      got=$(sha256sum "$tmp/$archive" | awk '{ print $1 }')
    elif command -v shasum >/dev/null 2>&1; then
      got=$(shasum -a 256 "$tmp/$archive" | awk '{ print $1 }')
    else
      die "sha256sum or shasum is required to check the download"
    fi
    [ "$got" = "$want" ] || die "checksum mismatch for $archive: expected $want, got $got"
    say "checksum verified"
  elif [ "${WRKFLW_NO_VERIFY:-}" = 1 ]; then
    say "this release publishes no SHA256SUMS; installing unverified (WRKFLW_NO_VERIFY=1)"
  else
    die "this release publishes no SHA256SUMS to check the download against; \
set WRKFLW_NO_VERIFY=1 to install it anyway, or pick a newer WRKFLW_VERSION"
  fi

  tar -xzf "$tmp/$archive" -C "$tmp"
  [ -f "$tmp/wrkflw" ] || die "$archive holds no wrkflw binary"
  mkdir -p "$INSTALL_DIR"
  # Moved into place in one step, so a wrkflw that is running isn't
  # overwritten as it reads itself.
  cp "$tmp/wrkflw" "$INSTALL_DIR/.wrkflw.new"
  chmod 755 "$INSTALL_DIR/.wrkflw.new"
  mv -f "$INSTALL_DIR/.wrkflw.new" "$INSTALL_DIR/wrkflw"

  say "installed $("$INSTALL_DIR/wrkflw" --version 2>/dev/null || echo wrkflw) to $INSTALL_DIR/wrkflw"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) say "add $INSTALL_DIR to your PATH to run it as \`wrkflw\`" ;;
  esac
  say "run \`wrkflw doctor\` to check this machine"
}

main "$@"
