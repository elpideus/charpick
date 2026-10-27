#!/usr/bin/env bash
# Cuts a release: sets the version, commits it, tags it and pushes both. The pushed tag starts the
# release workflow (.github/workflows/release.yml), which checks, builds and publishes it.
#
#   scripts/release.sh 1.0.4               a release
#   scripts/release.sh 1.1.0-beta.1        a pre-release (a version with a "-")
#   scripts/release.sh 1.0.4 --no-push     commit and tag only; it prints the push command for later
#
# It refuses unless the working tree is clean, the branch is main and main is not behind
# origin/main, so a release is always a commit of main with nothing left out of it. When the
# version is already the one asked for (the first release of a version set by hand), there is
# nothing to commit and the current commit is tagged.

set -euo pipefail
cd "$(dirname "$0")/.."

usage='Usage: scripts/release.sh <version> [--no-push]   (1.2.3, or 1.2.3-beta.1 for a pre-release)'

fail() {
  echo "$*" >&2
  exit 1
}

# Runs git in the terminal, showing the command, so a signing prompt or the push's progress
# reaches the person.
git_live() {
  echo "\$ git $*"
  git "$@"
}

# Arguments.
push=yes
version=
for arg in "$@"; do
  case $arg in
    -h | --help)
      echo "$usage"
      exit 0
      ;;
    --no-push) push=no ;;
    -*) fail "Unknown option $arg"$'\n'"$usage" ;;
    *)
      [[ -z $version ]] || fail "$usage"
      version=${arg#v}
      ;;
  esac
done
[[ -n $version ]] || fail "$usage"
# The same rule as scripts/version.sh.
id='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
[[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-$id(\.$id)*)?$ ]] ||
  fail "$version is not a version like 1.2.3 or 1.2.3-beta.1"
tag=v$version

# Refusals, before anything changes.
changes=$(git status --porcelain)
[[ -z $changes ]] || fail "The working tree has changes. Commit or stash them first:"$'\n'"$changes"

branch=$(git symbolic-ref --quiet --short HEAD || true)
[[ $branch == main ]] || fail "Releases are cut from main, and this is ${branch:-a detached HEAD}."

git fetch --quiet origin || fail "Could not fetch origin, so there is no telling whether main is up to date."
git rev-parse --verify --quiet refs/remotes/origin/main > /dev/null || fail "origin has no main branch."
behind=$(git rev-list --count HEAD..origin/main)
[[ $behind == 0 ]] || fail "main is $behind commit(s) behind origin/main. Pull first."

if git rev-parse --verify --quiet "refs/tags/$tag" > /dev/null; then fail "The tag $tag already exists."; fi
[[ -z $(git ls-remote --tags origin "refs/tags/$tag") ]] || fail "origin already has the tag $tag."

# The release. scripts/version.sh checks the version before it changes anything.
scripts/version.sh "$version" || fail 'Stopped before committing. `git restore .` puts the version files back.'
committed=no
if [[ -n $(git status --porcelain) ]]; then
  # The tree was clean, so every change is the version and `-a` takes exactly those files.
  git_live commit -a -m "Release $tag" || fail 'Stopped before committing. `git restore .` puts the version files back.'
  committed=yes
else
  echo "The version is already $version: tagging the current commit."
fi

if ! git_live tag -a "$tag" -m "Charpick $version"; then
  [[ $committed == yes ]] && fail 'The release commit is made but not tagged. `git reset --hard HEAD~1` undoes it.'
  fail "Could not tag $tag."
fi

push_command="git push --atomic origin main $tag"
if [[ $push == no ]]; then
  echo
  echo "Committed and tagged $tag, not pushed. To publish it: $push_command"
  exit 0
fi
git_live push --atomic origin main "$tag" ||
  fail $'\n'"$tag is committed and tagged but not pushed. Once the problem is fixed: $push_command"
echo
echo "Pushed $tag. GitHub Actions now builds and publishes it: https://github.com/elpideus/charpick/actions"
