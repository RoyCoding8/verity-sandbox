#!/usr/bin/env bash
set -euo pipefail

destination=${1:?Provide the destination for tla2tools.jar}
curl --fail --location --retry 3 --output "$destination" \
    https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar
printf '%s  %s\n' 936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88 "$destination" | sha256sum --check
