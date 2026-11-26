#!/usr/bin/env bash
# Check that every non-legacy example pins the SDK version in SDK_VERSION.
# Covers the crates ephemeral-rollups-sdk and ephemeral-rollups-pinocchio, and the npm
# packages @magicblock-labs/ephemeral-rollups-sdk and -kit. 00-LEGACY_EXAMPLES/ is exempt.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(tr -d '[:space:]' < SDK_VERSION)"
CRATE_RE='^(ephemeral-rollups-sdk|ephemeral-rollups-pinocchio)[[:space:]]*='
NPM_RE='"@magicblock-labs/ephemeral-rollups-(sdk|kit)"[[:space:]]*:'
PRUNE=(-path ./00-LEGACY_EXAMPLES -o -name node_modules -o -name target -o -name .git)

checked=0
errors=()

while IFS= read -r file; do
  while IFS=: read -r line text; do
    # Workspace members inherit the version from the workspace root, which is checked itself.
    [[ "$text" =~ workspace[[:space:]]*=[[:space:]]*true ]] && continue
    checked=$((checked + 1))
    if [[ "$text" =~ (git|path)[[:space:]]*= ]]; then
      errors+=("$file:$line: git/path dependency, want crates.io version $VERSION")
      continue
    fi
    # Either `crate = "X"` or `crate = { version = "X", ... }`.
    found="$(sed -E 's/^[^=]*=[[:space:]]*(\{[^}]*version[[:space:]]*=[[:space:]]*)?"=?([^"]*)".*/\2/' <<<"$text")"
    [[ "$found" == "$VERSION" ]] || errors+=("$file:$line: found $found, want $VERSION")
  done < <(grep -nE "$CRATE_RE" "$file" || true)
done < <(find . \( "${PRUNE[@]}" \) -prune -o -name Cargo.toml -type f -print | sort)

while IFS= read -r file; do
  while IFS=: read -r line text; do
    checked=$((checked + 1))
    found="$(sed -E 's/.*:[[:space:]]*"([^"]*)".*/\1/' <<<"$text")"
    [[ "$found" == "$VERSION" ]] || errors+=("$file:$line: found $found, want $VERSION")
  done < <(grep -nE "$NPM_RE" "$file" || true)
done < <(find . \( "${PRUNE[@]}" \) -prune -o -name package.json -type f -print | sort)

if [ "$checked" -eq 0 ]; then
  echo "No SDK dependencies found; check the patterns in $0" >&2
  exit 1
fi

if [ ${#errors[@]} -gt 0 ]; then
  echo "SDK version mismatches (SDK_VERSION is $VERSION; 00-LEGACY_EXAMPLES/ is exempt):" >&2
  printf '  %s\n' "${errors[@]}" >&2
  echo "Fix with: bash scripts/update-sdk-versions.sh" >&2
  exit 1
fi

echo "All $checked SDK dependencies pin $VERSION"
