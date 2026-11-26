#!/usr/bin/env bash
# Move every non-legacy example to one magicsvm version and refresh the lockfiles.
# Usage: scripts/update-magicsvm-version.sh <version>
set -euo pipefail

VERSION="${1:?usage: $0 <version>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

manifests=()
while IFS= read -r manifest; do
  manifests+=("$manifest")
done < <(
  find . \( -path ./00-LEGACY_EXAMPLES -o -name node_modules -o -name target -o -name .git \) -prune \
    -o -name Cargo.toml -print | xargs grep -lE '^magicsvm[[:space:]]*=' | sort
)

if [[ ${#manifests[@]} -eq 0 ]]; then
  echo "No Cargo.toml depends on magicsvm" >&2
  exit 1
fi

for manifest in "${manifests[@]}"; do
  sed -i.sedbak -E "s/^magicsvm[[:space:]]*=[[:space:]]*\"[^\"]*\"/magicsvm = \"$VERSION\"/" "$manifest"
  rm -f "$manifest.sedbak"
  # Re-resolves only what the edited manifest no longer satisfies; other pins stay put.
  cargo metadata --manifest-path "$manifest" --format-version 1 >/dev/null
  echo "$manifest -> magicsvm $VERSION"
done
