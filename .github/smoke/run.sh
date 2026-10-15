#!/bin/sh
# Starts the whole stack with the smoke-test mod and rAthena's custom NPCs
# switched on and checks it
# the way a player uses it (check.py). Run from the repository root, with
# the rag-wrapper/rathena and rag-wrapper/portal images already there. It
# writes .env, mods/enabled.txt and overrides/custom_npcs.txt, so use a
# scratch checkout. PRERE=no in
# the environment starts it in Renewal.
set -eu

sed -e 's/^ADMIN_USER=.*/ADMIN_USER=smoke_admin/' \
	-e 's/^ADMIN_PASSWORD=.*/ADMIN_PASSWORD=smoke_admin_pw1/' \
	-e "s/^PRERE=.*/PRERE=${PRERE:-yes}/" .env.example > .env

cp -r .github/smoke/mods/smoke-test mods/
echo smoke-test > mods/enabled.txt

# Every one of rAthena's custom NPCs the desktop app offers, the way its
# Mods screen writes them, so a new rAthena commit that drops or breaks one
# fails here.
python3 - > overrides/custom_npcs.txt <<'PY'
import json
for e in json.load(open("desktop/src-tauri/custom-npcs.json")):
    if not e.get("unavailable"):
        for f in e["files"]:
            print(f"{e['mode']}: {f}" if e.get("mode") else f)
PY

docker compose up --detach --no-build

set -a
. ./.env
set +a
python3 .github/smoke/check.py
