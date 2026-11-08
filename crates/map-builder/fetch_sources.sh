#!/usr/bin/env bash
# Download and verify the source datasets into data/source/ (big files not committed;
# the small World Bank snapshots ARE committed so population is reproducible).
set -euo pipefail
cd "$(dirname "$0")/../.."
ne=data/source/natural-earth
mkdir -p "$ne" data/source/etopo
fetch_ne() { # name folder sha256
  local zip="$ne/$1.zip"
  [ -f "$zip" ] || curl -sSfLo "$zip" "https://naciscdn.org/naturalearth/10m/$2/$1.zip"
  echo "$3  $zip" | shasum -a 256 -c -
  [ -d "$ne/$1" ] || { mkdir -p "$ne/$1"; unzip -q -o "$zip" -d "$ne/$1"; }
}
fetch_ne ne_10m_admin_0_countries cultural ce1ac7036499a0edd641fbc093cd209a98f96a49d2eca8480aaacad35138a7f6
fetch_ne ne_10m_admin_1_states_provinces cultural efc59726337323058f9446210adc96673179cd344e053666ee3d28cb58ba2b05
fetch_ne ne_10m_airports cultural a29e335cb67e1e4c131ded1772e0d7074cf1c677bd27d25cf2f2c440c27d7db3
fetch_ne ne_10m_geography_regions_polys physical cb7b9db200284ed1551f20eacc7f3333e9b5f311c19f7cb2670694529f688682
fetch_ne ne_10m_glaciated_areas physical de6ff396ab7eee8cbe2146030cc29278ef67c468e5675c748b1372e241ecbbff
fetch_ne ne_10m_populated_places cultural cd149186f03d2603e0410da399b980a4357d0ac32d3a2305a49ed3dffcc41d7b
fetch_ne ne_10m_ports cultural 1824cccd897a6c2ad546abbaf1f3545d51cff4bddfbda40244a1ad1ab1e4371d
fetch_ne ne_10m_railroads cultural 0bce64d1bb7ce1178810f0a341322b0459acb0689a61016259ef38f75abed0a4
fetch_ne ne_10m_rivers_lake_centerlines physical ded71b01870855ccfe19b51f2ec14c9bb48fae23c0e9f3c11974d426433b5c38
fetch_ne ne_10m_roads cultural 880b235383a1e1ccf33f4ea6f82b6755a2ffbce07e3974e412b43e88fe954ae9
fetch_ne ne_10m_urban_areas cultural 2c593114e954ce1adb297ddee5465f05d9e81fa0a26deb13f8500d58a00d7051

etopo=data/source/etopo/ETOPO_2022_v1_60s_N90W180_surface.tif
[ -f "$etopo" ] || curl -sSfLo "$etopo" "https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/ETOPO_2022_v1_60s_N90W180_surface.tif"
echo "9d27d4b8ea8e76977e2988bca667d7c8fa68b927355feffcddd6b4875a7fd08e  $etopo" | shasum -a 256 -c -

# World Bank snapshots (SP.POP.TOTL, fetched 2026-11-08, CC BY 4.0). Committed; just verify.
wb=data/source/worldbank
echo "c9acf3932785191a649cbc04c3056af4cf64a236e5c3b49d49be25ddb0440bce  $wb/countries.json" | shasum -a 256 -c -
echo "ed6fbdc71ec10343be3ad6fee986b803483c3499e552c049c4c9c8c9c4253227  $wb/pop1990.json" | shasum -a 256 -c -
echo "c4f96838ccdd5cad6326e704caf7ec692ea421497db9b7cf333fc2dd389ee687  $wb/pop2019.json" | shasum -a 256 -c -
