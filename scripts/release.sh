#!/usr/bin/env bash
# Releases rexurrect: scripts/release.sh <version>
#
# Checks everything CI can't, then bumps the version, commits
# `chore: release vX.Y.Z`, tags it, and pushes the commit and the tag
# together. The tag starts the release workflow (dist), which builds the
# binaries, publishes the GitHub release and, unless the version is a
# prerelease (0.2.0-rc.1), updates the Homebrew formula.
#
# Nothing is changed until every check has passed: a clean working tree, a
# tag that doesn't exist yet, formatting, clippy, and the full test suite.
# The integration tests run against real Rex servers, so `rex` must be on
# PATH. CI never runs them.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "Usage: scripts/release.sh <version>"
  echo "Example: scripts/release.sh 0.2.0"
  exit 1
fi

# Strip a leading 'v' if given.
VERSION="${VERSION#v}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Error: '$VERSION' isn't a semver version (X.Y.Z, or X.Y.Z-pre for a prerelease)"
  exit 1
fi
TAG="v$VERSION"

# Untracked files count: cargo would build and test them, but they wouldn't
# be in the release commit.
if [ -n "$(git status --porcelain)" ]; then
  echo "Error: the working tree is dirty; commit or stash changes first"
  git status --short
  exit 1
fi

if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null ||
  [ -n "$(git ls-remote --tags origin "refs/tags/$TAG")" ]; then
  echo "Error: tag '$TAG' already exists"
  exit 1
fi

if ! command -v rex >/dev/null; then
  echo "Error: no rex on PATH; the integration tests run against real Rex servers"
  exit 1
fi

echo "Checking formatting..."
cargo fmt --check
echo "Running clippy..."
cargo clippy --all-targets --locked -- -D warnings
echo "Running the full test suite, integration tests included..."
cargo test --locked

CURRENT=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
echo "Bumping version: $CURRENT -> $VERSION"
sed -i '' "s/^version = \"$CURRENT\"$/version = \"$VERSION\"/" Cargo.toml
# Updates rexurrect's own entry in Cargo.lock, and nothing else.
cargo update --workspace --offline --quiet

git add Cargo.toml Cargo.lock
git commit -m "chore: release $TAG"
git tag "$TAG"
git push --atomic origin HEAD "$TAG"

echo ""
echo "Released $TAG. Watch the release workflow:"
echo "  https://github.com/cemcatik/rexurrect/actions/workflows/release.yml"
