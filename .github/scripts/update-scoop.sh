#!/usr/bin/env bash
#
# Write the Scoop manifest for one belaf release into a checked-out
# scoop-bucket clone and push it.
#
# Lives in a script rather than inline YAML because two workflows drive it: the
# `publish-scoop-manifest` job in release.yml (the normal path, on the tag push)
# and the manual `workflow_dispatch` in update-scoop.yml (re-running a past
# release). Inlining it twice would let the two copies drift.
#
# Usage: update-scoop.sh <release-tag> <bucket-dir>
set -euo pipefail

RELEASE_TAG="${1:?usage: update-scoop.sh <release-tag> <bucket-dir>}"
BUCKET_DIR="${2:?usage: update-scoop.sh <release-tag> <bucket-dir>}"

VERSION="${RELEASE_TAG#v}"
WINDOWS_ZIP="belaf-x86_64-pc-windows-msvc.zip"
URL="https://github.com/ilblu/belaf/releases/download/${RELEASE_TAG}/${WINDOWS_ZIP}"

# The release assets are uploaded by an earlier job in the same run, but GitHub
# serves them through a CDN that can lag behind the API by a few seconds. Retry
# rather than publish a manifest with an empty hash, which would make every
# `scoop install belaf` fail its integrity check.
HASH=""
for attempt in 1 2 3 4 5; do
  HASH="$(curl -sfL "${URL}.sha256" | cut -d' ' -f1 || true)"
  if [ -n "$HASH" ]; then
    echo "got checksum on attempt ${attempt}"
    break
  fi
  echo "checksum not available yet (attempt ${attempt}), retrying" >&2
  sleep "$((attempt * 3))"
done

if [ -z "$HASH" ]; then
  echo "could not fetch ${URL}.sha256 — refusing to publish a manifest without a hash" >&2
  exit 1
fi

cat > "${BUCKET_DIR}/belaf.json" <<EOF
{
  "version": "${VERSION}",
  "description": "Release management CLI for monorepos",
  "homepage": "https://github.com/ilblu/belaf",
  "license": "MIT",
  "architecture": {
    "64bit": {
      "url": "${URL}",
      "hash": "${HASH}"
    }
  },
  "bin": "belaf.exe",
  "checkver": {
    "github": "https://github.com/ilblu/belaf"
  },
  "autoupdate": {
    "architecture": {
      "64bit": {
        "url": "https://github.com/ilblu/belaf/releases/download/v\$version/belaf-x86_64-pc-windows-msvc.zip"
      }
    }
  }
}
EOF

cd "$BUCKET_DIR"
git config user.name "ilblu-bot"
git config user.email "bot@ilblu.dev"
git add belaf.json

# Re-running a release that is already published is a no-op, not a failure.
if git diff --cached --quiet; then
  echo "belaf.json already at ${VERSION}, nothing to push"
  exit 0
fi

git commit -m "chore: update belaf to v${VERSION}"
git push
