#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${VERSION:-}" ]]; then
  echo "::error::VERSION is required"
  exit 1
fi

if [[ "${GITHUB_REF:-}" != "refs/heads/main" ]]; then
  echo "::error::Releases must run from main, got ${GITHUB_REF:-<unset>}"
  exit 1
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "::error::Invalid version format. Use x.y.z (e.g., 1.0.2)"
  exit 1
fi

VERSION_TAG="v$VERSION"
MAJOR_TAG="v${VERSION%%.*}"
RELEASE_SHA="$(git rev-parse HEAD)"

git config user.name "github-actions[bot]"
git config user.email "github-actions[bot]@users.noreply.github.com"
git fetch --force --tags origin

if TAG_SHA="$(git rev-parse --verify "$VERSION_TAG^{commit}" 2>/dev/null)"; then
  if [[ "$TAG_SHA" != "$RELEASE_SHA" ]]; then
    echo "::error::Tag $VERSION_TAG points to $TAG_SHA, expected $RELEASE_SHA"
    exit 1
  fi
  echo "Tag $VERSION_TAG already points to the release commit"
else
  git tag -a "$VERSION_TAG" -m "$VERSION_TAG"
fi

if MAJOR_SHA="$(git rev-parse --verify "$MAJOR_TAG^{commit}" 2>/dev/null)" && [[ "$MAJOR_SHA" == "$RELEASE_SHA" ]]; then
  echo "Tag $MAJOR_TAG already points to the release commit"
else
  git tag -fa "$MAJOR_TAG" -m "$MAJOR_TAG: Latest ${MAJOR_TAG}.x release"
fi
git push --atomic origin \
  "refs/tags/$VERSION_TAG:refs/tags/$VERSION_TAG" \
  "+refs/tags/$MAJOR_TAG:refs/tags/$MAJOR_TAG"

if gh release view "$VERSION_TAG" >/dev/null 2>&1; then
  echo "GitHub release $VERSION_TAG already exists"
else
  gh release create "$VERSION_TAG" \
    --title "$VERSION_TAG" \
    --generate-notes \
    --latest
fi

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "## Released $VERSION_TAG"
    echo ""
    echo "- Tag: \`$VERSION_TAG\`"
    echo "- Floating tag: \`$MAJOR_TAG\` → $VERSION_TAG"
  } >> "$GITHUB_STEP_SUMMARY"
fi
