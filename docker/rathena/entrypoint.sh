#!/bin/sh
# Usage: entrypoint.sh init|login|char|map|web
#
# Writes rAthena's conf/import overrides from the environment, then either
# prepares the database (init) or runs one server in the foreground.
set -eu

: "${DB_HOST:=db}"
: "${DB_PORT:=3306}"
: "${DB_NAME:=ragnarok}"
# Accounts are in DB_NAME, shared by both game modes. Characters and
# everything else in the game are in a database of their own for each
# mode, so switching between Pre-Renewal and Renewal swaps the characters
# and switching back brings the old ones back.
if [ "${PRERE:-yes}" = "yes" ]; then
	: "${CHAR_DB_NAME:=${DB_NAME}_prere}"
	OTHER_CHAR_DB_NAME=${DB_NAME}_re
else
	: "${CHAR_DB_NAME:=${DB_NAME}_re}"
	OTHER_CHAR_DB_NAME=${DB_NAME}_prere
fi
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

	# Commands on other players start with * rather than rAthena's #, which
	# the 20250716 client keeps for itself. Set before the overrides
	# folder's file, so a server can still choose its own symbol there.
	{ echo "charcommand_symbol: *"; user_import battle_conf.txt; } > conf/import/battle_conf.txt

	{
		user_import inter_conf.txt
		for db in login_server ipban_db char_server map_server web_server log_db; do
			echo "${db}_ip: ${DB_HOST}"
			echo "${db}_port: ${DB_PORT}"
			echo "${db}_id: ${DB_USER}"
			echo "${db}_pw: ${DB_PASSWORD}"
			case "${db}" in
				login_server | ipban_db) echo "${db}_db: ${DB_NAME}" ;;
				*) echo "${db}_db: ${CHAR_DB_NAME}" ;;
			esac
		done
	} > conf/import/inter_conf.txt

	# Passwords are stored as MD5; accounts come from the portal, not _M/_F.
	{ user_import login_conf.txt; cat <<CONF
use_MD5_passwords: yes
new_account: no
CONF
	} > conf/import/login_conf.txt

	# No PIN code window before character select: accounts already have a
	# portal password, and a forgotten PIN can only be reset in the database.
	{ user_import char_conf.txt; cat <<CONF
pincode_enabled: no
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
# mods/enabled.txt names the ones that are on, in load order, so where
# two change the same database entry the later one wins. In each mod:
#   npc/**/*.txt         scripts, loaded after rAthena's own (and kept by
#                        @reloadscript) through npc/scripts_custom.conf
#   db/*.yml             db/import-style YAML, named like rAthena's
#   db/pre-re/*.yml,     database files (item_db.yml, mob_db.yml, ...);
#   db/re/*.yml          the pre-re/ or re/ ones only in that mode
# Problems are logged with the file's path, which the app looks for, and
# clashes between mods with both paths.
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
		done | awk '!seen[$0]++'
}

# The import file a mod's database file is added to. Most are rAthena's
# db/import/ files; barters live with the NPCs, under npc/custom/.
import_target() {
	case "$1" in
		barters.yml) echo npc/custom/barters.yml ;;
		*) if [ -f "db/import-tmpl/$1" ]; then echo "db/import/$1"; fi ;;
	esac
}

apply_mods() {
	mode=re
	if [ "${PRERE:-yes}" = "yes" ]; then
		mode=pre-re
	fi

	# Start from rAthena's own import files, so a mod switched off since
	# the last start leaves nothing behind.
	for path in db/import-tmpl/*.yml db/import-tmpl/*.txt; do
		cp "${path}" "db/import/$(basename "${path}")"
	done
	# npc/custom/barters.yml has no template, so the first start keeps a copy.
	if [ ! -f npc/custom/barters.yml.orig ]; then
		cp npc/custom/barters.yml npc/custom/barters.yml.orig
	fi
	cp npc/custom/barters.yml.orig npc/custom/barters.yml
	if ! grep -qx 'import: npc/mods.conf' npc/scripts_custom.conf; then
		printf '\nimport: npc/mods.conf\n' >> npc/scripts_custom.conf
	fi
	: > npc/mods.conf

	# The desktop app looks for this line to tell an image that loads mods
	# from one built before mods existed.
	echo "Mods enabled: $(enabled_mods | tr '\n' ' ')"
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
			if [ -z "$(import_target "${file}")" ]; then
				echo "[Warning]: ${path} skipped: it isn't named after one of rAthena's databases in db/import (or barters.yml)" >&2
				continue
			fi
			printf '%s %s\n' "${file}" "${path}" >> "${imports}"
		done
		# The few databases rAthena still reads as text (mob_skill_db.txt,
		# ...) have no imports, so a mod's rows are added to the end of the
		# db/import/ file, after a comment naming the mod's file.
		for path in "mods/${name}"/db/*.txt "mods/${name}/db/${mode}"/*.txt; do
			if [ ! -f "${path}" ]; then
				continue
			fi
			file=$(basename "${path}")
			if [ ! -f "db/import-tmpl/${file}" ]; then
				echo "[Warning]: ${path} skipped: it isn't named after one of rAthena's databases in db/import" >&2
				continue
			fi
			{
				printf '\n// %s\n' "${path}"
				sed 's/\r$//' "${path}"
			} >> "db/import/${file}"
		done
	done

	# rAthena applies each mod's entries as changes to what's loaded, so
	# most lists (a monster's Drops, an item group's items) combine on
	# their own. merge_mods.py warns where two mods set the same value
	# (the later mod's is used) and writes db/import/mods-merged/ files,
	# loaded last, for the fields rAthena would otherwise reset (an item's
	# Jobs). A failure here leaves the mods loading as they are.
	if ! merge_mods.py "${imports}" db/import/mods-merged; then
		echo "[Warning]: couldn't check how the mods' database files combine" >&2
	fi

	# Each database's import file lists the mods' files under its Footer,
	# which rAthena loads after the file's own entries, in this order.
	for file in $(cut -d ' ' -f 1 "${imports}" | sort -u); do
		{
			printf '\nFooter:\n  Imports:\n'
			awk -v f="${file}" '$1 == f { sub(/^[^ ]+ /, ""); gsub(/\047/, "\047\047"); printf "    - Path: \047%s\047\n", $0 }' "${imports}"
		} >> "$(import_target "${file}")"
	done
	rm -f "${imports}"
}

# The files enabled mods add to one database, from its import file's Footer.
mod_files() {
	sed -n "s/^    - Path: '\(.*\)'\$/\1/p" "db/import/$1" 2>/dev/null | sed "s/''/'/g"
}

sql_in() {
	db=$1
	shift
	mysql --host="${DB_HOST}" --port="${DB_PORT}" --user="${DB_USER}" \
		--password="${DB_PASSWORD}" "${db}" "$@"
}

# The accounts database.
mysql_cmd() {
	sql_in "${DB_NAME}" "$@"
}

# This game mode's characters database.
char_cmd() {
	sql_in "${CHAR_DB_NAME}" "$@"
}

root_cmd() {
	mysql --host="${DB_HOST}" --port="${DB_PORT}" --user=root \
		--password="${DB_ROOT_PASSWORD:?DB_ROOT_PASSWORD must be set}" "$@"
}

# rAthena ships schema changes as sql-files/upgrades/upgrade_YYYYMMDD*.sql.
# main.sql already includes every upgrade up to the commit it came from, so
# the first time the tracking table is empty all current files are recorded
# as applied. After that, files that arrive with a newer RATHENA_COMMIT are
# run once each, in date order. Some upstream files are not safe to re-run,
# which is why they are tracked rather than replayed.
apply_upgrades() {
	cmd=$1
	"${cmd}" -e "CREATE TABLE IF NOT EXISTS wrapper_sql_upgrades (
		file varchar(64) NOT NULL PRIMARY KEY,
		applied_at datetime NOT NULL DEFAULT CURRENT_TIMESTAMP)"

	baseline=
	if [ "$("${cmd}" -N -e "SELECT COUNT(*) FROM wrapper_sql_upgrades")" = "0" ]; then
		baseline=yes
	fi

	for path in $(ls sql-files/upgrades/upgrade_*.sql | sort); do
		file=$(basename "${path}")
		if [ -n "$("${cmd}" -N -e "SELECT 1 FROM wrapper_sql_upgrades WHERE file='${file}'")" ]; then
			continue
		fi
		if [ -z "${baseline}" ]; then
			echo "Applying upgrade ${file}"
			"${cmd}" < "${path}"
		fi
		"${cmd}" -e "INSERT INTO wrapper_sql_upgrades (file) VALUES ('${file}')"
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
	} | char_cmd; then
		echo "Couldn't load item names; the portal will show item numbers" >&2
		return 0
	fi
	echo "Item names loaded: $(char_cmd -N -e "SELECT COUNT(*) FROM wrapper_item_names")"
}

# The tables the login server uses. They stay in the accounts database;
# every other table goes in the characters database of each game mode.
ACCOUNT_TABLES="login ipbanlist global_acc_reg_num global_acc_reg_str"

is_account_table() {
	case " ${ACCOUNT_TABLES} " in
		*" $1 "*) return 0 ;;
	esac
	return 1
}

import_schema() {
	for f in main.sql logs.sql web.sql roulette_default_data.sql; do
		echo "  ${f}"
		"$1" < "sql-files/${f}"
	done
}

has_char_table() {
	[ -n "$(sql_in "$1" -N -e "SHOW TABLES LIKE 'char'" 2>/dev/null)" ]
}

# Makes this mode's characters database. Until there is one, the accounts
# database holds everything, as it did before the modes had databases of
# their own: the first mode to get one takes those characters with it.
# A mode that comes later starts with no characters.
init_char_db() {
	root_cmd -e "CREATE DATABASE IF NOT EXISTS \`${CHAR_DB_NAME}\`;
		GRANT ALL PRIVILEGES ON \`${CHAR_DB_NAME}\`.* TO '${DB_USER}'@'%'"

	if has_char_table "${CHAR_DB_NAME}"; then
		echo "Characters database ${CHAR_DB_NAME} already present"
	elif has_char_table "${OTHER_CHAR_DB_NAME}"; then
		echo "New characters database ${CHAR_DB_NAME}: importing rAthena schema"
		import_schema char_cmd
	else
		echo "Moving the characters to ${CHAR_DB_NAME}"
		moves=
		for table in $(mysql_cmd -N -e "SHOW TABLES"); do
			case "${table}" in
				wrapper_sql_upgrades) continue ;;
			esac
			if ! is_account_table "${table}"; then
				moves="${moves}${moves:+, }\`${DB_NAME}\`.\`${table}\` TO \`${CHAR_DB_NAME}\`.\`${table}\`"
			fi
		done
		# One statement, so the tables move together or not at all. The
		# upgrades already applied to them come along too.
		root_cmd -e "RENAME TABLE ${moves};
			CREATE TABLE IF NOT EXISTS \`${CHAR_DB_NAME}\`.wrapper_sql_upgrades LIKE \`${DB_NAME}\`.wrapper_sql_upgrades;
			INSERT IGNORE INTO \`${CHAR_DB_NAME}\`.wrapper_sql_upgrades SELECT * FROM \`${DB_NAME}\`.wrapper_sql_upgrades"
	fi

	# rAthena's upgrade files can change any table, so both databases keep
	# every table, the ones they don't use empty.
	for table in $(char_cmd -N -e "SHOW TABLES"); do
		case "${table}" in wrapper_*) continue ;; esac
		mysql_cmd -e "CREATE TABLE IF NOT EXISTS \`${table}\` LIKE \`${CHAR_DB_NAME}\`.\`${table}\`"
	done
	for table in ${ACCOUNT_TABLES}; do
		char_cmd -e "CREATE TABLE IF NOT EXISTS \`${table}\` LIKE \`${DB_NAME}\`.\`${table}\`"
	done
}

init_db() {
	if [ -z "$(mysql_cmd -N -e "SHOW TABLES LIKE 'login'")" ]; then
		echo "Empty database: importing rAthena schema"
		import_schema mysql_cmd
	else
		echo "Schema already present: skipping import"
	fi
	# Recorded before anything moves, so both databases start from it.
	apply_upgrades mysql_cmd

	init_char_db
	apply_upgrades char_cmd

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
