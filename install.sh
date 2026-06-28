#!/bin/sh
# Install telex from GitHub Releases.
#
#   curl -fsSL https://raw.githubusercontent.com/lossyrob/telex/main/install.sh | sh
#
# Options (environment variables):
#   TELEX_INSTALL_DIR  install location (default: $HOME/.local/bin)
#   TELEX_VERSION      version tag to install (default: latest)
#   GITHUB_TOKEN       optional, raises GitHub API rate limits
set -eu

REPO="lossyrob/telex"
INSTALL_DIR="${TELEX_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || err "missing required tool: $1"; }

need curl
need tar

# curl wrapper that adds a GitHub token header when available (raises rate limits).
gh_curl() {
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$@"
  else
    curl -fsSL "$@"
  fi
}

os="$(uname -s)"
arch="$(uname -m)"
case "${os}-${arch}" in
  Linux-x86_64|Linux-amd64)   target="x86_64-unknown-linux-gnu" ;;
  Darwin-arm64|Darwin-aarch64) target="aarch64-apple-darwin" ;;
  Darwin-x86_64)               target="x86_64-apple-darwin" ;;
  *)
    err "unsupported platform ${os}-${arch} — install with: cargo install --git https://github.com/${REPO} --features entra" ;;
esac

# Resolve the version tag.
tag="${TELEX_VERSION:-}"
if [ -z "${tag}" ]; then
  tag="$(gh_curl "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' | head -n1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"
  [ -n "${tag}" ] || err "could not determine the latest release tag (is a release published?)"
fi

asset="telex-${tag}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

say "Downloading ${asset} ..."
curl -fsSL "${url}" -o "${tmp}/${asset}" || err "download failed: ${url}"

# Best-effort checksum verification.
if curl -fsSL "${url}.sha256" -o "${tmp}/${asset}.sha256" 2>/dev/null; then
  expected="$(awk '{print $1}' "${tmp}/${asset}.sha256")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${tmp}/${asset}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${tmp}/${asset}" | awk '{print $1}')"
  fi
  [ "${expected}" = "${actual}" ] || err "checksum mismatch for ${asset}"
  say "Checksum OK."
fi

tar -C "${tmp}" -xzf "${tmp}/${asset}"
mkdir -p "${INSTALL_DIR}"
install -m 0755 "${tmp}/telex" "${INSTALL_DIR}/telex"
if [ -d "${tmp}/copilot-plugin" ]; then
  rm -rf "${INSTALL_DIR}/copilot-plugin"
  mkdir -p "${INSTALL_DIR}/copilot-plugin"
  cp -R "${tmp}/copilot-plugin/." "${INSTALL_DIR}/copilot-plugin/"
fi

say ""
say "Installed telex ${tag} to ${INSTALL_DIR}/telex"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) : ;;
  *) say "Add it to your PATH:  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac
say "Next:  telex skill"
if [ -f "${INSTALL_DIR}/copilot-plugin/plugin.json" ]; then
  say "Copilot plugin:  copilot plugin install \"${INSTALL_DIR}/copilot-plugin\""
fi
