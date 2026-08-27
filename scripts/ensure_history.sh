#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${BASE_REF:-}" ]]; then
  echo "::error::BASE_REF is required"
  exit 1
fi
case "${USE_MERGE_BASE:-false}" in
  true|false) ;;
  *) echo "::error::USE_MERGE_BASE must be true or false"; exit 1 ;;
esac

has_commit() {
  git rev-parse --verify "$1^{commit}" >/dev/null 2>&1
}

is_head_relative_ref() {
  [[ "$1" =~ ^HEAD~[0-9]+$ ]]
}

is_raw_sha() {
  [[ "$1" =~ ^[0-9a-fA-F]{40,64}$ ]]
}

unshallow() {
  if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
    git fetch --no-tags --unshallow origin || true
  fi
}

fetch_raw_sha() {
  local ref="$1"
  git fetch --no-tags --depth=1 origin "$ref" || true
  if ! has_commit "$ref"; then
    unshallow
    git fetch --no-tags origin "$ref"
  fi
}

fetch_symbolic_ref() {
  local ref="$1"
  RESOLVED_REF="$ref"
  if [[ "$ref" == origin/* ]]; then
    local branch="${ref#origin/}"
    git fetch --no-tags --depth=1 origin "refs/heads/$branch:refs/remotes/origin/$branch" || true
    return
  fi
  if [[ "$ref" == refs/heads/* ]]; then
    local branch="${ref#refs/heads/}"
    RESOLVED_REF="refs/remotes/origin/$branch"
    git fetch --no-tags --depth=1 origin "refs/heads/$branch:$RESOLVED_REF" || true
    return
  fi
  if [[ "$ref" == refs/tags/* ]]; then
    git fetch --depth=1 origin "$ref:$ref" || true
    return
  fi

  RESOLVED_REF="refs/remotes/origin/$ref"
  if git fetch --no-tags --depth=1 origin "refs/heads/$ref:$RESOLVED_REF"; then
    return
  fi
  RESOLVED_REF="refs/tags/$ref"
  git fetch --depth=1 origin "$RESOLVED_REF:$RESOLVED_REF" || true
}

RESOLVED_REF="$BASE_REF"
if is_head_relative_ref "$BASE_REF"; then
  if ! has_commit "$BASE_REF"; then
    depth="${BASE_REF#HEAD~}"
    echo "Fetching $((depth + 1)) commits for $BASE_REF..."
    git fetch --no-tags --depth="$((depth + 1))" origin HEAD
  fi
elif is_raw_sha "$BASE_REF"; then
  if ! has_commit "$BASE_REF"; then
    echo "Fetching exact base commit $BASE_REF..."
    fetch_raw_sha "$BASE_REF"
  fi
else
  if ! has_commit "$BASE_REF"; then
    echo "Fetching comparison ref $BASE_REF..."
    fetch_symbolic_ref "$BASE_REF"
  fi
  if ! has_commit "$RESOLVED_REF"; then
    echo "::error::Cannot resolve $BASE_REF after fetching it from origin"
    exit 1
  fi
  if ! git merge-base HEAD "$RESOLVED_REF" >/dev/null 2>&1; then
    echo "Fetching history needed to connect HEAD with $BASE_REF..."
    unshallow
    if [[ "$BASE_REF" == origin/* ]]; then
      branch="${BASE_REF#origin/}"
      git fetch --no-tags origin "refs/heads/$branch:refs/remotes/origin/$branch" || true
    elif [[ "$RESOLVED_REF" == refs/remotes/origin/* ]]; then
      branch="${RESOLVED_REF#refs/remotes/origin/}"
      git fetch --no-tags origin "refs/heads/$branch:$RESOLVED_REF" || true
    fi
  fi
  if ! git merge-base HEAD "$RESOLVED_REF" >/dev/null 2>&1; then
    echo "::error::Cannot resolve merge-base with $BASE_REF after fetching history"
    exit 1
  fi
fi

if ! has_commit "$RESOLVED_REF"; then
  echo "::error::Cannot resolve $BASE_REF after fetching history"
  exit 1
fi

if [[ "${USE_MERGE_BASE:-false}" == true ]]; then
  BASE_COMMIT="$(git merge-base HEAD "$RESOLVED_REF")"
else
  BASE_COMMIT="$(git rev-parse --verify "$RESOLVED_REF^{commit}")"
fi
echo "ref=$BASE_COMMIT" >> "$GITHUB_OUTPUT"
echo "Base commit $BASE_COMMIT verified from $BASE_REF"
