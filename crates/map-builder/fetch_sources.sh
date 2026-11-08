#!/usr/bin/env bash
# Download and verify the source datasets into data/source/ (not committed).
set -euo pipefail
cd "$(dirname "$0")/../.."
dest=data/source/natural-earth
mkdir -p "$dest"
fetch() { # name sha256
  local zip="$dest/$1.zip"
  [ -f "$zip" ] || curl -sSfLo "$zip" "https://naciscdn.org/naturalearth/10m/cultural/$1.zip"
  echo "$2  $zip" | shasum -a 256 -c -
  [ -d "$dest/$1" ] || { mkdir -p "$dest/$1"; unzip -q -o "$zip" -d "$dest/$1"; }
}
fetch ne_10m_admin_0_countries ce1ac7036499a0edd641fbc093cd209a98f96a49d2eca8480aaacad35138a7f6
fetch ne_10m_admin_1_states_provinces efc59726337323058f9446210adc96673179cd344e053666ee3d28cb58ba2b05
