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

# The map server shows the desktop app's in-game announcements: its script
# marks a new one shown within a couple of seconds.
chars=ragnarok_prere
if [ "${PRERE:-yes}" = "no" ]; then
	chars=ragnarok_re
fi
chars_sql() { docker compose exec -T -e MYSQL_PWD="${DB_PASSWORD}" db mariadb --user=ragnarok --batch --skip-column-names "${chars}" -e "$1"; }
id=$(chars_sql "INSERT INTO wrapper_announcements (message) VALUES ('Smoke test'); SELECT LAST_INSERT_ID();")
shown=
for _ in $(seq 20); do
	shown=$(chars_sql "SELECT shown_at IS NOT NULL AND skipped = 0 FROM wrapper_announcements WHERE id = ${id}")
	[ "${shown}" = "1" ] && break
	sleep 1
done
if [ "${shown}" != "1" ]; then
	docker compose logs map | tail -n 50
	echo "The map server didn't show an announcement" >&2
	exit 1
fi

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

# An upgrade file that fails partway stops db-init, and later starts stop
# too, naming the file, rather than running it again.
upgrade=sql-files/upgrades/upgrade_99990101.sql
broken() { docker compose run --rm --no-deps -v "${PWD}/.github/smoke/upgrade_broken.sql:/rathena/${upgrade}:ro" db-init > broken.log 2>&1; }
for attempt in first again; do
	if broken; then
		cat broken.log
		echo "db-init carried on after a failed upgrade (${attempt} run)" >&2
		exit 1
	fi
	cat broken.log
	grep -q "upgrade upgrade_99990101.sql didn't finish in the ragnarok database" broken.log
done
# The second run didn't apply the file again: its first statement would
# have failed with "already exists" before the check.
if grep -q "already exists" broken.log; then
	echo "db-init ran the failed upgrade again" >&2
	exit 1
fi
root_sql "DROP TABLE smoke_partial_upgrade; DELETE FROM wrapper_sql_upgrades WHERE file='upgrade_99990101.sql'"
docker compose run --rm --no-deps db-init

# An image built for the other game mode doesn't start.
other=no
if [ "${PRERE:-yes}" = "no" ]; then
	other=yes
fi
if docker compose run --rm --no-deps -e PRERE="${other}" db-init > mode.log 2>&1; then
	cat mode.log
	echo "db-init started with an image built for the other game mode" >&2
	exit 1
fi
cat mode.log
grep -q "This server was built for" mode.log
