#!/usr/bin/env bash
# Fails when a ref's history adds internal working material that is not meant
# to ship in the open-source distribution: agent instructions, editor rules,
# and the engineering standards module.
#
# Deleting such a file does not unpublish it. Once a commit that added it is
# reachable from a public ref, `git show <commit>:<path>` returns the content
# on a plain clone, forever, whether or not any tree still lists it. So the
# check looks at what history *added*, not at what the tree currently holds.
#
# Two files are grandfathered: they shipped in the v0.1.0 initial public
# release (4065008) and were removed in the 0.3.0 release (426821b), so they
# are already reachable from `main` and cannot be recalled without rewriting
# published release history. They are documented in RELEASING.md. Nothing
# else may join them.
#
# Usage:
#   scripts/check-internal-paths.sh [ref]        # scan a ref's whole history
#   scripts/check-internal-paths.sh --diff BASE  # scan the net diff vs BASE
#
# The --diff form models a squash merge: a branch that adds and then removes
# these files has a net diff of zero and passes, because squashing it into the
# default branch introduces nothing. Plain history on such a branch will fail
# the default form, which is the correct warning — merge it squashed.

set -euo pipefail

# Paths that must never enter the published history.
PATTERN='^(AGENTS\.md|CLAUDE\.md|\.claude/|\.cursor/|docs/standards/)'

# Already reachable from `main`; see the header. Exact paths, not a prefix.
GRANDFATHERED=(
  ".claude/skills/saya-run/SKILL.md"
  ".claude/skills/saya-smoke/SKILL.md"
)

is_grandfathered() {
  local candidate="$1"
  for allowed in "${GRANDFATHERED[@]}"; do
    [[ "$candidate" == "$allowed" ]] && return 0
  done
  return 1
}

mode="history"
target="HEAD"
if [[ "${1:-}" == "--diff" ]]; then
  mode="diff"
  target="${2:?--diff needs a base ref}"
elif [[ -n "${1:-}" ]]; then
  target="$1"
fi

# `mapfile` is bash 4+; macOS ships 3.2, so read the list with a plain loop.
if [[ "$mode" == "diff" ]]; then
  # Files the net diff adds relative to the merge base.
  added=$(git diff --diff-filter=A --name-only "${target}...HEAD" | grep -E "$PATTERN" || true)
  scope="the net diff against ${target}"
else
  # Files any reachable commit adds, whether or not they still exist.
  added=$(git log --diff-filter=A --pretty=format: --name-only "$target" | grep -E "$PATTERN" | sort -u || true)
  scope="the history of ${target}"
fi

offenders=()
while IFS= read -r path; do
  [[ -z "$path" ]] && continue
  is_grandfathered "$path" || offenders+=("$path")
done <<< "$added"

if [[ ${#offenders[@]} -eq 0 ]]; then
  echo "ok: ${scope} adds no internal-only paths beyond the ${#GRANDFATHERED[@]} documented in RELEASING.md"
  exit 0
fi

echo "error: ${scope} adds internal-only material that would become permanently"
echo "       retrievable from a public clone via 'git show <commit>:<path>':"
echo
for path in "${offenders[@]}"; do
  echo "         $path"
done
echo
echo "       Removing the file in a later commit does not undo this."
echo
echo "       If the branch adds and later removes these, squash-merge it — the"
echo "       squashed commit carries the tree, not the intermediate history."
echo "       Verify with: scripts/check-internal-paths.sh --diff origin/main"
exit 1
