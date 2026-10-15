#!/bin/bash
# Gets the rAthena and portal images for .env.example's settings: the
# published prebuilt image when there is one for exactly these files (named
# the way images.yml and the app name them), otherwise a build here.
# PRERE=no in the environment picks Renewal.
set -euo pipefail

REGISTRY=ghcr.io/akreao/rag-wrapper
setting() { sed -n "s/^$1=//p" .env.example; }
dir_hash() { (cd "$1" && find . -type f -not -path '*/.*' -printf '%P\n' | LC_ALL=C sort | xargs -d '\n' sha256sum | sha256sum | cut -c1-12); }

packetver=$(setting PACKETVER)
commit=$(setting RATHENA_COMMIT)
export PRERE=${PRERE:-$(setting PRERE)}
mode=$([ "${PRERE}" = yes ] && echo prere || echo re)

fetch() { # remote local build-service
	if docker pull "$1"; then
		docker tag "$1" "$2"
	else
		echo "No published $1; building it here."
		cp -n .env.example .env
		docker compose build "$3"
	fi
}

fetch "$REGISTRY-rathena:$packetver-$mode-${commit:0:12}-$(dir_hash docker/rathena)" "rag-wrapper/rathena:$packetver" db-init
fetch "$REGISTRY-portal:$(dir_hash portal)" rag-wrapper/portal portal
