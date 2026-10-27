#!/usr/bin/env bash
# The version is written in several files that must agree: Cargo.toml, Charpick's entry in
# Cargo.lock, the Windows resources (charpick.rc) and the installer (installer/charpick.iss).
# Cargo.toml is the one the others follow.
#
#   scripts/version.sh                  print the version
#   scripts/version.sh 1.1.0            set it everywhere (1.1.0-beta.1 for a pre-release)
#   scripts/version.sh --check [1.1.0]  fail when a file disagrees, or differs from 1.1.0

set -euo pipefail
cd "$(dirname "$0")/.."

# Semantic versions without build metadata. A numeric pre-release identifier has no leading
# zero (beta.1, never beta.01): Cargo refuses to read such a version.
id='(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
semver="^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-$id(\.$id)*)?$"

fail() {
  echo "$*" >&2
  exit 1
}

valid() {
  [[ $1 =~ $semver ]] || fail "$1 is not a version like 1.2.3 or 1.2.3-beta.1"
}

# The version in Cargo.toml's [package] table.
cargo_version() {
  awk -F'"' '/^\[/ { table = $0 } table == "[package]" && /^version *=/ { print $2; exit }' Cargo.toml
}

# Every place the version is written: "<place>|<version found there>". The Windows resources'
# numeric FILEVERSION and PRODUCTVERSION are 1,2,3,0 for 1.2.3 and for 1.2.3-beta.1 alike.
versions() {
  echo "Cargo.toml|$(cargo_version)"
  echo "Cargo.lock|$(awk -F'"' '/^name = "charpick"$/ { getline; print $2; exit }' Cargo.lock)"
  echo "installer/charpick.iss|$(sed -n 's/^#define AppVersion "\(.*\)"$/\1/p' installer/charpick.iss)"
  echo "charpick.rc (FileVersion)|$(sed -n 's/.*VALUE "FileVersion", "\(.*\)"$/\1/p' charpick.rc)"
  echo "charpick.rc (ProductVersion)|$(sed -n 's/.*VALUE "ProductVersion", "\(.*\)"$/\1/p' charpick.rc)"
  echo "charpick.rc (FILEVERSION)|$(sed -n 's/^FILEVERSION //p' charpick.rc)"
  echo "charpick.rc (PRODUCTVERSION)|$(sed -n 's/^PRODUCTVERSION //p' charpick.rc)"
}

# 1,2,3,0 for 1.2.3 and 1.2.3-beta.1.
numeric() {
  local core=${1%%-*}
  echo "${core//./,},0"
}

check() {
  local want=$1 wrong=0 count=0 place found expected
  while IFS='|' read -r place found; do
    count=$((count + 1))
    expected=$want
    [[ $place == *VERSION\) ]] && expected=$(numeric "$want")
    if [[ $found != "$expected" ]]; then
      echo "$place: ${found:-missing} (expected $expected)" >&2
      wrong=1
    fi
  done < <(versions)
  [[ $wrong == 0 ]] || exit 1
  echo "$want in $count places"
}

set_version() {
  local v=$1 n
  n=$(numeric "$v")
  sed -i '/^\[package\]/,/^\[/ s/^version = ".*"/version = "'"$v"'"/' Cargo.toml
  sed -i '/^name = "charpick"$/ { n; s/^version = ".*"/version = "'"$v"'"/ }' Cargo.lock
  sed -i 's/^#define AppVersion ".*"/#define AppVersion "'"$v"'"/' installer/charpick.iss
  sed -i -e "s/^FILEVERSION .*/FILEVERSION $n/" -e "s/^PRODUCTVERSION .*/PRODUCTVERSION $n/" \
    -e 's/\(VALUE "FileVersion", \)".*"/\1"'"$v"'"/' -e 's/\(VALUE "ProductVersion", \)".*"/\1"'"$v"'"/' charpick.rc
}

case ${1:-} in
  --check)
    if [[ -n ${2:-} ]]; then
      want=${2#v}
      valid "$want"
    else
      want=$(cargo_version)
    fi
    check "$want"
    ;;
  -h | --help) sed -n '2,8s/^# \{0,1\}//p' "$0" ;;
  -*) fail "Unknown option $1" ;;
  '') cargo_version ;;
  *)
    v=${1#v}
    valid "$v"
    set_version "$v"
    check "$v"
    ;;
esac
