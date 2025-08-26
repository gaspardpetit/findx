#!/usr/bin/env bash
set -euo pipefail

SHA=$(git rev-parse --short HEAD)
DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  VERSION="${GITHUB_REF_NAME}"
  DESCRIBE="${GITHUB_REF_NAME}"
else
  DESCRIBE="$(git describe --tags --always --dirty)"
  VERSION="${DESCRIBE}"
fi
{
  echo "version=${VERSION}"
  echo "describe=${DESCRIBE}"
  echo "sha=${SHA}"
  echo "date=${DATE}"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"

if [[ "${GITHUB_REF_TYPE:-}" == "tag" && "${GITHUB_REF_NAME}" =~ ^v([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
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
