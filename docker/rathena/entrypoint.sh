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

# These end up in SQL and config files, so keep them to plain characters.
case "${INTER_USER}${INTER_PASSWORD}" in
	*[!A-Za-z0-9_]*)
		echo "INTER_USER and INTER_PASSWORD may only use letters, digits and _" >&2
		exit 64
		;;
esac

cd /rathena

write_imports() {
	{
		for db in login_server ipban_db char_server map_server web_server log_db; do
			echo "${db}_ip: ${DB_HOST}"
			echo "${db}_port: ${DB_PORT}"
			echo "${db}_id: ${DB_USER}"
			echo "${db}_pw: ${DB_PASSWORD}"
			echo "${db}_db: ${DB_NAME}"
		done
	} > conf/import/inter_conf.txt

	cat > conf/import/char_conf.txt <<CONF
userid: ${INTER_USER}
passwd: ${INTER_PASSWORD}
login_ip: ${LOGIN_HOST}
char_ip: ${PUBLIC_IP}
CONF

	cat > conf/import/map_conf.txt <<CONF
userid: ${INTER_USER}
passwd: ${INTER_PASSWORD}
char_ip: ${CHAR_HOST}
map_ip: ${PUBLIC_IP}
CONF
}

mysql_cmd() {
	mysql --host="${DB_HOST}" --port="${DB_PORT}" --user="${DB_USER}" \
		--password="${DB_PASSWORD}" "${DB_NAME}" "$@"
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

	# Account 1 is the server-to-server login; replace the stock s1/p1.
	mysql_cmd -e "UPDATE login SET userid='${INTER_USER}', user_pass='${INTER_PASSWORD}' WHERE account_id=1"
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
