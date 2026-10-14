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

# mods/ is the mods folder from the host, one folder per mod, and
# mods/enabled.txt names the ones that are on. Enabled mods load in
# alphabetical order, so where two change the same database entry the
# later name wins. In each mod:
#   npc/**/*.txt         scripts, loaded after rAthena's own (and kept by
#                        @reloadscript) through npc/scripts_custom.conf
#   db/*.yml             db/import-style YAML, named like rAthena's
#   db/pre-re/*.yml,     database files (item_db.yml, mob_db.yml, ...);
#   db/re/*.yml          the pre-re/ or re/ ones only in that mode
# Problems are logged with the file's path, which the app looks for.
enabled_mods() {
	if [ ! -f mods/enabled.txt ]; then
		return 0
	fi
	sed -e 's/\r$//' -e 's/#.*//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' mods/enabled.txt |
		while IFS= read -r name; do
			case "${name}" in
				'') ;;
				.* | *[!A-Za-z0-9._-]*)
					echo "[Warning]: mods/${name}/ skipped: a mod's folder name may only use letters, digits, . _ and -" >&2
					;;
				*)
					if [ -d "mods/${name}" ]; then
						echo "${name}"
					fi
					;;
			esac
		done | LC_ALL=C sort -u
}

apply_mods() {
	mode=re
	if [ "${PRERE:-yes}" = "yes" ]; then
		mode=pre-re
	fi

	# Start from rAthena's own import files, so a mod switched off since
	# the last start leaves nothing behind.
	for path in db/import-tmpl/*.yml; do
		cp "${path}" "db/import/$(basename "${path}")"
	done
	if ! grep -qx 'import: npc/mods.conf' npc/scripts_custom.conf; then
		printf '\nimport: npc/mods.conf\n' >> npc/scripts_custom.conf
	fi
	: > npc/mods.conf

	imports=$(mktemp)
	for name in $(enabled_mods); do
		echo "Mod: ${name}"
		if [ -d "mods/${name}/npc" ]; then
			find "mods/${name}/npc" -type f -iname '*.txt' | LC_ALL=C sort | sed 's/^/npc: /' >> npc/mods.conf
		fi
		for path in "mods/${name}"/db/*.yml "mods/${name}/db/${mode}"/*.yml; do
			if [ ! -f "${path}" ]; then
				continue
			fi
			file=$(basename "${path}")
			if [ ! -f "db/import-tmpl/${file}" ]; then
				echo "[Warning]: ${path} skipped: it isn't named after one of rAthena's databases in db/import" >&2
				continue
			fi
			printf '%s %s\n' "${file}" "${path}" >> "${imports}"
		done
	done

	# Each database's import file lists the mods' files under its Footer,
	# which rAthena loads after the file's own entries, in this order.
	for file in $(cut -d ' ' -f 1 "${imports}" | sort -u); do
		{
			printf '\nFooter:\n  Imports:\n'
			awk -v f="${file}" '$1 == f { sub(/^[^ ]+ /, ""); gsub(/\047/, "\047\047"); printf "    - Path: \047%s\047\n", $0 }' "${imports}"
		} >> "db/import/${file}"
	done
	rm -f "${imports}"
}

# The files enabled mods add to one database, from its import file's Footer.
mod_files() {
	sed -n "s/^    - Path: '\(.*\)'\$/\1/p" "db/import/$1" 2>/dev/null | sed "s/''/'/g"
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

# The portal shows item names (on its admin inventory page) from this
# table. It is refilled on every start from the item database this image
# was built with, pre-renewal or renewal, plus db/import/item_db.yml and the
# enabled mods' item_db.yml, so it follows rAthena updates and custom
# items. Assumes rAthena's own YAML
# layout: "  - Id: 501" with "    Name: Red Potion" under it.
load_item_names() {
	mode=re
	if [ "${PRERE:-yes}" = "yes" ]; then
		mode=pre-re
	fi
	# Only the portal uses these names, so a failure here doesn't stop the
	# servers from starting.
	if ! {
		echo "CREATE TABLE IF NOT EXISTS wrapper_item_names (
			id int unsigned NOT NULL PRIMARY KEY,
			name varchar(100) NOT NULL);"
		echo "START TRANSACTION; DELETE FROM wrapper_item_names;"
		# Later files win, so db/import and then the mods come last.
		{
			cat "db/${mode}"/item_db_*.yml db/import/item_db.yml
			mod_files item_db.yml | while IFS= read -r path; do cat "${path}"; done
		} 2>/dev/null | awk '
			{ sub(/\r$/, "") }
			/^  - Id:/ { id = $3 + 0; next }
			/^    Name:/ && id > 0 {
				name = substr($0, index($0, ":") + 1)
				sub(/^ +/, "", name)
				if (name ~ /^"/) { name = substr(name, 2); sub(/".*$/, "", name) }
				else { sub(/[ \t]+#.*$/, "", name); sub(/ +$/, "", name) }
				gsub(/\\/, "\\\\", name); gsub(/\047/, "\047\047", name)
				if (n % 500 == 0) printf "%sREPLACE INTO wrapper_item_names VALUES ", (n ? ";\n" : "")
				else printf ","
				printf "(%d,\047%s\047)", id, name
				n++; id = 0
			}
			END { if (n) print ";" }'
		echo "COMMIT;"
	} | mysql_cmd; then
		echo "Couldn't load item names; the portal will show item numbers" >&2
		return 0
	fi
	echo "Item names loaded: $(mysql_cmd -N -e "SELECT COUNT(*) FROM wrapper_item_names")"
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
	load_item_names
	echo "Database ready"
}

write_imports
apply_mods

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
