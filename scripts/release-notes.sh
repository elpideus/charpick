#!/usr/bin/env bash
# The body of a GitHub release, in Markdown: what changed since the previous release, then how to
# install it. The release workflow publishes it next to the downloads.
#
#   scripts/release-notes.sh v1.0.4
#
# "What's changed" is the subject of every commit since the previous release (every commit up to
# the tag, for the first release), leaving out the "Release v..." commits that only set the
# version. A pre-release counts from the previous v* tag of any kind. A stable release counts from
# the previous stable one, so it also lists what its pre-releases already shipped.

set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
  echo "$*" >&2
  exit 1
}

# The same rule as scripts/version.sh.
id='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
semver="^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-$id(\.$id)*)?$"

tag=${1:-}
[[ $tag =~ $semver ]] ||
  fail "Usage: scripts/release-notes.sh v<version>   (v1.2.3, or v1.2.3-beta.1 for a pre-release)"
git rev-parse --verify --quiet "refs/tags/$tag" > /dev/null || fail "There is no tag $tag"

repo=${GITHUB_REPOSITORY:-elpideus/charpick}

# The release before this one: the previous v* tag for a pre-release, the previous stable tag
# (one without a "-") for a stable release. None for the first release.
exclude=()
[[ $tag == *-* ]] || exclude=(--exclude 'v*-*')
previous=$(git describe --tags --abbrev=0 --match 'v*' "${exclude[@]}" "$tag^" 2> /dev/null || true)

changes=$(git log --format=%s "${previous:+$previous..}$tag" | grep -v -e '^Release v' -e '^[[:space:]]*$' | sed 's/^/- /' || true)

# One line per paragraph: GitHub shows every line break in a release body.
cat << EOF
## What's changed

${changes:-- No changes besides the version.}

## Install

**Windows 10 and 11:** download \`Charpick-Setup.exe\` below and run it. If Windows SmartScreen says it protected your PC, choose **More info**, then **Run anyway**: Charpick is not code-signed. If Charpick is already installed, Setup updates it and keeps your settings.

**Linux (KDE Plasma 6 on Wayland):** open Konsole, paste this line and press Enter. The same line updates Charpick later.

\`\`\`
curl -fsSL https://github.com/$repo/releases/download/$tag/charpick-linux-x86_64.tar.gz | tar xz -C /tmp && sh /tmp/charpick/install.sh
\`\`\`

The [README](https://github.com/$repo#readme) explains every step and how to use Charpick. \`SHA256SUMS\` lists the checksums of the downloads.
EOF
