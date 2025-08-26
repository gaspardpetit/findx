#!/usr/bin/env bash
set -euo pipefail

# Optional override for callers that need a specific version string
if [[ -n "${OVERRIDE_VERSION:-}" ]]; then
  VERSION="${OVERRIDE_VERSION}"
  DESCRIBE="${VERSION}"
else
  SHORT_SHA=$(git rev-parse --short=8 HEAD)
  DATE_RFC3339=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  DATE_YMD=$(date -u +%Y%m%d)
  if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
    TAG="${GITHUB_REF_NAME}"
  else
    TAG="$(git describe --tags --abbrev=0)"
  fi
  BASE="${TAG#v}"
  if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
    VERSION="${BASE}"
  else
    VERSION="${BASE}-dev.${DATE_YMD}-${SHORT_SHA}"
  fi
  DESCRIBE="${VERSION}"
  SHA="${SHORT_SHA}"
  DATE="${DATE_RFC3339}"
fi

# Write outputs
{
  echo "version=${VERSION}"
  echo "describe=${DESCRIBE}"
  [[ -n "${SHA:-}" ]] && echo "sha=${SHA}"
  [[ -n "${DATE:-}" ]] && echo "date=${DATE}"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"

# Detect semantic version when building tagged releases
if [[ "${GITHUB_REF_TYPE:-}" == "tag" && "${GITHUB_REF_NAME:-}" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  MAJOR="${BASH_REMATCH[1]}"
  MINOR="${BASH_REMATCH[2]}"
  PATCH="${BASH_REMATCH[3]}"
  {
    echo "major=${MAJOR}"
    echo "minor=${MINOR}"
    echo "patch=${PATCH}"
    echo "is_clean_release=true"
  } >> "${GITHUB_OUTPUT:-/dev/stdout}"
else
  echo "is_clean_release=false" >> "${GITHUB_OUTPUT:-/dev/stdout}"
fi

update_cargo_version() {
  local ver="$1"
  local sed_in=("-i")
  if [[ "$(uname)" == "Darwin" ]]; then
    sed_in=("-i" "")
  fi
  if grep -q '^version = ' Cargo.toml; then
    sed "${sed_in[@]}" -E "s/^version = \".*\"/version = \"${ver}\"/" Cargo.toml
  else
    sed "${sed_in[@]}" -E "0,/^name = \"findx\"/s//name = \"findx\"\nversion = \"${ver}\"/" Cargo.toml
  fi
}

if [[ "${UPDATE_CARGO_TOML:-}" == "1" ]]; then
  update_cargo_version "${VERSION}"
fi
