#!/bin/sh
# Usage: entrypoint.sh init|login|char|map|web
#
# Writes rAthena's conf/import overrides from the environment, then either
# prepares the database (init) or runs one server in the foreground.
set -eu

: "${DB_HOST:=db}"
: "${DB_PORT:=3306}"
: "${DB_NAME:=ragnarok}"
: "${DB_USER:=ragnarok}"
: "${DB_PASSWORD:?DB_PASSWORD must be set}"
: "${INTER_USER:?INTER_USER must be set}"
: "${INTER_PASSWORD:?INTER_PASSWORD must be set}"
: "${LOGIN_HOST:=login}"
: "${CHAR_HOST:=char}"
# The address the game client is told to connect to for char and map.
: "${PUBLIC_IP:=127.0.0.1}"

# Optional: the admin account the desktop app asks for on first run.
: "${ADMIN_USER:=}"
: "${ADMIN_PASSWORD:=}"

# These end up in SQL and config files, so keep them to plain characters.
case "${INTER_USER}${INTER_PASSWORD}${ADMIN_USER}${ADMIN_PASSWORD}" in
	*[!A-Za-z0-9_]*)
		echo "INTER_USER, INTER_PASSWORD, ADMIN_USER and ADMIN_PASSWORD may only use letters, digits and _" >&2
		exit 64
		;;
esac

cd /rathena

# conf/user is the overrides folder from the host: the desktop app's
# Settings screen writes there, and so can people by hand. Each file in it
# with the name of one of rAthena's conf/import files is used in its place;
# the files the stack writes below import it first, so the stack's own
# values (database, addresses, passwords) still win.
user_import() {
	if [ -f "conf/user/$1" ]; then
		echo "import: conf/user/$1"
	fi
}

write_imports() {
	for path in conf/import-tmpl/*; do
		file=$(basename "${path}")
		if [ ! -f "conf/user/${file}" ]; then
			cp "${path}" "conf/import/${file}"
		elif [ "${file%.yml}" != "${file}" ]; then
			cp "conf/user/${file}" "conf/import/${file}"
		else
			user_import "${file}" > "conf/import/${file}"
		fi
	done

	{
		user_import inter_conf.txt
		for db in login_server ipban_db char_server map_server web_server log_db; do
			echo "${db}_ip: ${DB_HOST}"
			echo "${db}_port: ${DB_PORT}"
			echo "${db}_id: ${DB_USER}"
			echo "${db}_pw: ${DB_PASSWORD}"
			echo "${db}_db: ${DB_NAME}"
		done
	} > conf/import/inter_conf.txt

	# Passwords are stored as MD5; accounts come from the portal, not _M/_F.
	{ user_import login_conf.txt; cat <<CONF
use_MD5_passwords: yes
new_account: no
CONF
	} > conf/import/login_conf.txt

	{ user_import char_conf.txt; cat <<CONF
userid: ${INTER_USER}
passwd: ${INTER_PASSWORD}
login_ip: ${LOGIN_HOST}
char_ip: ${PUBLIC_IP}
CONF
	} > conf/import/char_conf.txt

	{ user_import map_conf.txt; cat <<CONF
userid: ${INTER_USER}
passwd: ${INTER_PASSWORD}
char_ip: ${CHAR_HOST}
map_ip: ${PUBLIC_IP}
CONF
	} > conf/import/map_conf.txt
}

mysql_cmd() {
	mysql --host="${DB_HOST}" --port="${DB_PORT}" --user="${DB_USER}" \
		--password="${DB_PASSWORD}" "${DB_NAME}" "$@"
}

# rAthena ships schema changes as sql-files/upgrades/upgrade_YYYYMMDD*.sql.
# main.sql already includes every upgrade up to the commit it came from, so
# the first time the tracking table is empty all current files are recorded
# as applied. After that, files that arrive with a newer RATHENA_COMMIT are
# run once each, in date order. Some upstream files are not safe to re-run,
# which is why they are tracked rather than replayed.
apply_upgrades() {
	mysql_cmd -e "CREATE TABLE IF NOT EXISTS wrapper_sql_upgrades (
		file varchar(64) NOT NULL PRIMARY KEY,
		applied_at datetime NOT NULL DEFAULT CURRENT_TIMESTAMP)"

	baseline=
	if [ "$(mysql_cmd -N -e "SELECT COUNT(*) FROM wrapper_sql_upgrades")" = "0" ]; then
		baseline=yes
	fi

	for path in $(ls sql-files/upgrades/upgrade_*.sql | sort); do
		file=$(basename "${path}")
		if [ -n "$(mysql_cmd -N -e "SELECT 1 FROM wrapper_sql_upgrades WHERE file='${file}'")" ]; then
			continue
		fi
		if [ -z "${baseline}" ]; then
			echo "Applying upgrade ${file}"
			mysql_cmd < "${path}"
		fi
		mysql_cmd -e "INSERT INTO wrapper_sql_upgrades (file) VALUES ('${file}')"
	done
}

init_db() {
	if [ -z "$(mysql_cmd -N -e "SHOW TABLES LIKE 'login'")" ]; then
		echo "Empty database: importing rAthena schema"
		for f in main.sql logs.sql web.sql roulette_default_data.sql; do
			echo "  ${f}"
			mysql_cmd < "sql-files/${f}"
		done
	else
		echo "Schema already present: skipping import"
	fi

	apply_upgrades

	# Account 1 is the server-to-server login; replace the stock s1/p1.
	mysql_cmd -e "UPDATE login SET userid='${INTER_USER}', user_pass=MD5('${INTER_PASSWORD}') WHERE account_id=1"

	# Group 99 is Admin in conf/groups.yml. Only created once; later runs
	# leave an existing account alone.
	if [ -n "${ADMIN_USER}" ] && [ -n "${ADMIN_PASSWORD}" ]; then
		if [ -z "$(mysql_cmd -N -e "SELECT 1 FROM login WHERE userid='${ADMIN_USER}'")" ]; then
			mysql_cmd -e "INSERT INTO login (userid, user_pass, sex, email, group_id) VALUES ('${ADMIN_USER}', MD5('${ADMIN_PASSWORD}'), 'M', 'a@a.com', 99)"
			echo "Created admin account ${ADMIN_USER}"
		fi
	fi
	echo "Database ready"
}

write_imports

case "${1:-}" in
	init)  init_db ;;
	login) exec ./login-server ;;
	char)  exec ./char-server ;;
	map)   exec ./map-server ;;
	web)   exec ./web-server ;;
	*)
		echo "usage: $0 init|login|char|map|web" >&2
		exit 64
		;;
esac
