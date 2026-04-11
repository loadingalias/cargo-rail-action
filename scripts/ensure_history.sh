#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${BASE_REF:-}" ]]; then
  echo "::error::BASE_REF is required"
  exit 1
fi

emit_output() {
  local key="$1"
  local value="$2"
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "$key=$value" >> "$GITHUB_OUTPUT"
  fi
}

shallow_file() {
  git rev-parse --git-path shallow
}

is_shallow_clone() {
  [[ -f "$(shallow_file)" ]]
}

has_commit() {
  git rev-parse --verify "$1^{commit}" &>/dev/null
}

is_head_relative_ref() {
  [[ "$1" =~ ^HEAD~[0-9]+$ ]]
}

is_raw_sha() {
  [[ "$1" =~ ^[0-9a-fA-F]{40}$ ]]
}

fetch_branch_ref() {
  local ref="$1"
  local mode="${2:-depth1}"

  if [[ "$ref" == origin/* ]]; then
    local branch="${ref#origin/}"
    if [[ "$mode" == "depth1" ]]; then
      git fetch --no-tags --depth=1 origin "refs/heads/$branch:refs/remotes/origin/$branch"
    else
      git fetch --no-tags origin "refs/heads/$branch:refs/remotes/origin/$branch"
    fi
  elif [[ "$mode" == "depth1" ]]; then
    git fetch --no-tags --depth=1 origin "$ref"
  else
    git fetch --no-tags origin "$ref"
  fi
}

emit_output "shallow" "$(if is_shallow_clone; then echo true; else echo false; fi)"

if is_head_relative_ref "$BASE_REF"; then
  if ! has_commit "$BASE_REF"; then
    depth="${BASE_REF#HEAD~}"
    fetch_depth=$((depth + 1))
    echo "Fetching $fetch_depth commits for $BASE_REF..."
    git fetch --no-tags --depth="$fetch_depth" origin HEAD
  fi
elif is_raw_sha "$BASE_REF"; then
  if ! has_commit "$BASE_REF"; then
    echo "Fetching raw base SHA $BASE_REF directly..."
    fetch_branch_ref "$BASE_REF" depth1 || true

    if ! has_commit "$BASE_REF"; then
      if is_shallow_clone; then
        echo "Direct SHA fetch did not resolve $BASE_REF; unshallowing current checkout..."
        git fetch --no-tags --unshallow origin || true
      fi

      if ! has_commit "$BASE_REF"; then
        echo "Fetching raw base SHA $BASE_REF without depth limit..."
        fetch_branch_ref "$BASE_REF" full
      fi
    fi
  fi
else
  if ! has_commit "$BASE_REF"; then
    echo "Fetching branch ref $BASE_REF..."
    fetch_branch_ref "$BASE_REF" depth1 || true
  fi

  if ! git merge-base HEAD "$BASE_REF" &>/dev/null; then
    echo "Fetching history for merge-base with $BASE_REF..."

    if is_shallow_clone; then
      git fetch --no-tags --unshallow origin || true
    fi

    if ! git merge-base HEAD "$BASE_REF" &>/dev/null; then
      fetch_branch_ref "$BASE_REF" full || true
    fi
  fi
fi

if is_head_relative_ref "$BASE_REF" || is_raw_sha "$BASE_REF"; then
  if ! has_commit "$BASE_REF"; then
    echo "::error::Cannot resolve $BASE_REF after fetch. Check repository state."
    exit 1
  fi
else
  if ! git merge-base HEAD "$BASE_REF" &>/dev/null; then
    echo "::error::Cannot resolve merge-base with $BASE_REF after fetch. Check repository state."
    exit 1
  fi
fi

echo "Base ref $BASE_REF verified"
