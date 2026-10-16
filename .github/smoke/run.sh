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

# After going back to an older server version, a database the newer one
# upgraded stops db-init before the servers start, and is left as it is.
root_sql() { docker compose exec -T -e MYSQL_PWD="${DB_ROOT_PASSWORD}" db mariadb --user=root ragnarok -e "$1"; }
root_sql "INSERT INTO wrapper_sql_upgrades (file) VALUES ('upgrade_99991231.sql')"
if docker compose run --rm --no-deps db-init > newer.log 2>&1; then
	cat newer.log
	echo "db-init ran on a database a newer server version upgraded" >&2
	exit 1
fi
cat newer.log
grep -q "upgraded by a newer server version" newer.log
root_sql "DELETE FROM wrapper_sql_upgrades WHERE file='upgrade_99991231.sql'"
docker compose run --rm --no-deps db-init
