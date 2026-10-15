#!/bin/sh
# Starts the whole stack with the smoke-test mod switched on and checks it
# the way a player uses it (check.py). Run from the repository root, with
# the rag-wrapper/rathena and rag-wrapper/portal images already there. It
# writes .env and mods/enabled.txt, so use a scratch checkout. PRERE=no in
# the environment starts it in Renewal.
set -eu

sed -e 's/^ADMIN_USER=.*/ADMIN_USER=smoke_admin/' \
	-e 's/^ADMIN_PASSWORD=.*/ADMIN_PASSWORD=smoke_admin_pw1/' \
	-e "s/^PRERE=.*/PRERE=${PRERE:-yes}/" .env.example > .env

cp -r .github/smoke/mods/smoke-test mods/
echo smoke-test > mods/enabled.txt

docker compose up --detach --no-build

set -a
. ./.env
set +a
python3 .github/smoke/check.py
