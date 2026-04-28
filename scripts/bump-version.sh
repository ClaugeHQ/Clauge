#!/bin/bash
# Usage: ./scripts/bump-version.sh 0.2.1
# Bumps version, commits, and pushes — create tag from GitHub UI to trigger release
set -e

VERSION="$1"
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>"
  echo "Example: $0 0.2.1"
  exit 1
fi

# Update all three files
if [[ "$OSTYPE" == "darwin"* ]]; then
  sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json
  sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" package.json
  sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" src-tauri/Cargo.toml
else
  sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" src-tauri/tauri.conf.json
  sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" package.json
  sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" src-tauri/Cargo.toml
fi

echo "Bumped to v$VERSION"

# Commit and push only — tag from GitHub UI after writing release notes
git add src-tauri/tauri.conf.json src-tauri/Cargo.toml package.json
git commit -m "chore: bump version to $VERSION"
git push origin feat/qorix-merge

echo "Done — version bumped to v$VERSION"
echo "Next: Go to GitHub → Releases → Create new release → tag v$VERSION → add notes → Publish"
