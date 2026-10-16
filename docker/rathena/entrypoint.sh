#!/bin/sh
# Usage: entrypoint.sh init|login|char|map|web|config
#
# Writes rAthena's conf/import overrides from the environment, then either
# prepares the database (init) or runs one server in the foreground.
# config writes the files and stops, which the tests use.
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

# RATHENA_DIR lets the tests run this on a copy outside the image.
cd "${RATHENA_DIR:-/rathena}"

mode_name() {
	if [ "$1" = "yes" ]; then echo Pre-Renewal; else echo Renewal; fi
}

# The game mode is compiled into the servers, and PRERE picks the
# characters database and the mods' files to match. An image built for the
# other mode (a mode switch whose rebuild failed or never ran) would play
# one mode's characters with the other's rules, so stop. Images built
# before BUILD_PRERE existed have no file and aren't checked.
if [ -f BUILD_PRERE ] && [ "$(cat BUILD_PRERE)" != "${PRERE:-yes}" ]; then
	echo "This server was built for $(mode_name "$(cat BUILD_PRERE)"), but the settings ask for $(mode_name "${PRERE:-yes}") (PRERE=${PRERE:-yes})." >&2
	echo "Rebuild the server so it matches (Rebuild in the desktop app, or docker compose build db-init), or switch the mode back." >&2
	exit 66
fi

# The passwords .env.example comes with are public. The desktop app makes
# random ones for a new install, but a .env copied by hand, or made by the
# app next to an existing database, can still have them. Only warned
# about: refusing would stop a server that has run on them for months, and
# the database passwords can't simply be changed in .env (MariaDB keeps
# the ones it was created with).
warn_placeholder_passwords() {
	placeholders=""
	[ "${DB_PASSWORD}" = "change-me" ] && placeholders="${placeholders} DB_PASSWORD"
	[ "${DB_ROOT_PASSWORD:-}" = "change-me-too" ] && placeholders="${placeholders} DB_ROOT_PASSWORD"
	[ "${INTER_PASSWORD}" = "changeme123" ] && placeholders="${placeholders} INTER_PASSWORD"
	[ -n "${placeholders}" ] || return 0
	for key in ${placeholders}; do
		echo "WARNING: ${key} is still the example value from .env.example, which anyone can look up." >&2
	done
	case "${placeholders}" in *INTER_PASSWORD*)
		echo "WARNING: With INTER_PASSWORD known, someone on your network can connect a fake char server while the real one is down or restarting. Set a new random INTER_PASSWORD in .env and restart the whole stack; db-init updates the account on every start." >&2
		;;
	esac
	case "${placeholders}" in *DB_*)
		echo "WARNING: The database port isn't published, so the database passwords matter less. Changing them in .env alone stops the server from starting: MariaDB keeps the passwords it was created with." >&2
		;;
	esac
}

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

	# rAthena logs every item that moves, down to each potion drunk and
	# arrow fired, into tables nothing trims, and every backup copies them.
	# Keep the logs that answer "where did this item go" between players
	# (trades, vending, storage, mail, MVP prizes, GM commands and so on)
	# and drop (P) player drops and pickups, (L) monster loot and (C)
	# consumed items. Set before the overrides folder's file, so a server
	# can turn them back on there.
	{ echo "enable_logs: 0xFFFFFF73"; user_import log_conf.txt; } > conf/import/log_conf.txt

	# Passwords are stored as MD5; accounts come from the portal, not _M/_F.
	{ user_import login_conf.txt; cat <<CONF
use_MD5_passwords: yes
new_account: no
CONF
	} > conf/import/login_conf.txt

	# New characters all start in the same place instead of one of rAthena's
	# five copies (iz_int01-04 lead to izlude_a-d, new_2-1 to new_5-1 on
	# Pre-Renewal): a LAN server is too small for channels, and some NPCs are
	# only on the main izlude. Set before the overrides folder's file, so a
	# server can still choose its own start points there.
	#
	# No PIN code window before character select: accounts already have a
	# portal password, and a forgotten PIN can only be reset in the database.
	{
		echo "start_point: iz_int,18,26"
		echo "start_point_pre: new_1-1,53,111"
		user_import char_conf.txt
		cat <<CONF
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

# The stack's own scripts in npc/wrapper/ (docker/rathena/npc/ in
# rag-wrapper), such as the desktop app's in-game announcements, load
# after rAthena's own, before the custom NPCs and mods.
apply_wrapper_npcs() {
	if ! grep -qx 'import: npc/wrapper.conf' npc/scripts_custom.conf; then
		printf '\nimport: npc/wrapper.conf\n' >> npc/scripts_custom.conf
	fi
	: > npc/wrapper.conf
	for path in npc/wrapper/*.txt; do
		if [ -f "${path}" ]; then
			echo "npc: ${path}" >> npc/wrapper.conf
		fi
	done
}

# rAthena ships custom NPCs (npc/custom/) that npc/scripts_custom.conf
# lists switched off. conf/user/custom_npcs.txt, which the desktop app's
# Mods screen writes, names the ones to load, one script per line. A line
# starting "re: " or "pre-re: " loads only in that game mode, for scripts
# that need the other mode's maps or items. They load before the mods.
apply_custom_npcs() {
	if ! grep -qx 'import: npc/custom_npcs.conf' npc/scripts_custom.conf; then
		printf '\nimport: npc/custom_npcs.conf\n' >> npc/scripts_custom.conf
	fi
	: > npc/custom_npcs.conf
	loaded=
	if [ -f conf/user/custom_npcs.txt ]; then
		loaded=$(sed -e 's/\r$//' -e 's/#.*//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' conf/user/custom_npcs.txt |
			while IFS= read -r line; do
				path=${line#*: }
				case "${line}" in
					'') continue ;;
					"re: "*) [ "${PRERE:-yes}" = "yes" ] && continue ;;
					"pre-re: "*) [ "${PRERE:-yes}" = "yes" ] || continue ;;
					*": "*)
						echo "[Warning]: conf/user/custom_npcs.txt: '${line}' skipped: only re: and pre-re: can come before a script" >&2
						continue
						;;
				esac
				case "${path}" in
					npc/*.txt) ;;
					*)
						echo "[Warning]: conf/user/custom_npcs.txt: '${path}' skipped: only rAthena's npc/ scripts (.txt) can be named here" >&2
						continue
						;;
				esac
				case "${path}" in
					*..* | *[!A-Za-z0-9._/-]*)
						echo "[Warning]: conf/user/custom_npcs.txt: '${path}' skipped: only letters, digits, . _ - and / work in a script's path" >&2
						continue
						;;
				esac
				if [ ! -f "${path}" ]; then
					echo "[Warning]: conf/user/custom_npcs.txt: ${path} skipped: this server's rAthena has no such script" >&2
					continue
				fi
				echo "${path}"
			done | awk '!seen[$0]++')
	fi
	for path in ${loaded}; do
		echo "npc: ${path}" >> npc/custom_npcs.conf
	done
	# The desktop app looks for this line to tell an image that loads them
	# from one built before.
	echo "Custom NPCs on: $(echo ${loaded})"
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
		applied_at datetime NOT NULL DEFAULT CURRENT_TIMESTAMP);
		ALTER TABLE wrapper_sql_upgrades ADD COLUMN IF NOT EXISTS done tinyint(1) NOT NULL DEFAULT 1"

	# A file is recorded with done=0 before it runs and set to done=1 once
	# it has. The mysql client stops at a failing statement but keeps what
	# came before it, and rAthena's tables are mostly MyISAM, so nothing
	# rolls back: running the file again would fail on its first change.
	unfinished=$("${cmd}" -N -e "SELECT file FROM wrapper_sql_upgrades WHERE done=0 ORDER BY file")
	if [ -n "${unfinished}" ]; then
		upgrade_failed "${cmd}" "$(echo "${unfinished}" | head -n 1)"
	fi

	# Assigned first, so a failed query stops the script.
	count=$("${cmd}" -N -e "SELECT COUNT(*) FROM wrapper_sql_upgrades")
	applied=$("${cmd}" -N -e "SELECT file FROM wrapper_sql_upgrades")
	files=$(ls sql-files/upgrades/upgrade_*.sql | sort)

	if [ "${count}" = "0" ]; then
		# One statement, so an interrupted first start records all of them
		# or none.
		values=
		for path in ${files}; do
			values="${values}${values:+,}('$(basename "${path}")')"
		done
		if [ -n "${values}" ]; then
			"${cmd}" -e "INSERT INTO wrapper_sql_upgrades (file) VALUES ${values}"
		fi
		return 0
	fi

	for path in ${files}; do
		file=$(basename "${path}")
		if echo "${applied}" | grep -qxF "${file}"; then
			continue
		fi
		echo "Applying upgrade ${file}"
		"${cmd}" -e "INSERT INTO wrapper_sql_upgrades (file, done) VALUES ('${file}', 0)"
		if ! "${cmd}" < "${path}"; then
			upgrade_failed "${cmd}" "${file}"
		fi
		"${cmd}" -e "UPDATE wrapper_sql_upgrades SET done=1, applied_at=CURRENT_TIMESTAMP WHERE file='${file}'"
	done
}

# Stops the start-up after an upgrade file failed partway, now or on an
# earlier start, without running it again.
upgrade_failed() {
	case "$1" in
		char_cmd) db=${CHAR_DB_NAME} ;;
		*) db=${DB_NAME} ;;
	esac
	echo "The database upgrade $2 didn't finish in the ${db} database, so the servers can't start." >&2
	echo "Part of it may have been applied, so it isn't run again. Restore the backup made before the server update." >&2
	echo "If you finished the upgrade by hand instead, mark it done: UPDATE ${db}.wrapper_sql_upgrades SET done=1 WHERE file='$2';" >&2
	exit 67
}

# Upgrades a database has recorded that are dated after this image's
# newest one came from a newer rAthena commit, after which the server went
# back to this older one. Its code may not work with what they changed, so
# the servers don't start and the database is left as it is. Older files
# missing here (upstream renaming one) don't count. Prints the newer files.
newer_upgrades() {
	db=$1
	if [ -z "$(root_cmd -N -e "SELECT 1 FROM information_schema.tables
		WHERE table_schema='${db}' AND table_name='wrapper_sql_upgrades'")" ]; then
		return 0
	fi
	newest=$(basename "$(ls sql-files/upgrades/upgrade_*.sql | sort | tail -n 1)")
	root_cmd -N -e "SELECT file FROM \`${db}\`.wrapper_sql_upgrades
		WHERE file > '${newest}' ORDER BY file"
}

check_not_newer() {
	newer="$(newer_upgrades "${DB_NAME}")$(newer_upgrades "${CHAR_DB_NAME}")"
	if [ -n "${newer}" ]; then
		echo "The database was upgraded by a newer server version than this one:" >&2
		echo "${newer}" | sort -u | sed 's/^/  /' >&2
		echo "This version can't run on it. Go forward to the newer version again, or restore the backup made before the server update." >&2
		exit 65
	fi
}

# The portal's item and monster pages (and the item names on its admin
# pages) read tables lookup_db.py fills from the item and monster
# databases, drop rate settings and spawns this map server loads. It runs
# beside the map server each time the map server starts, so mods and
# settings show once a restart has loaded them, and the portal keeps the
# previous tables until the new ones are complete. Only the portal uses
# them, so a failure here leaves the map server running.
load_lookup() {
	mode=re
	if [ "${PRERE:-yes}" = "yes" ]; then
		mode=pre-re
	fi
	sql=$(mktemp)
	if lookup_db.py "${mode}" > "${sql}" && char_cmd < "${sql}"; then
		:
	else
		echo "[Warning]: couldn't fill the portal's item and monster tables" >&2
	fi
	rm -f "${sql}"
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
			INSERT IGNORE INTO \`${CHAR_DB_NAME}\`.wrapper_sql_upgrades (file, applied_at)
				SELECT file, applied_at FROM \`${DB_NAME}\`.wrapper_sql_upgrades"
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

# Group 99 is Admin in conf/groups.yml. The account is only created once:
# an existing one keeps its password and group, since they may have been
# changed on the portal since. The desktop app reads the lines that start
# "Admin account kept:" and "Admin account not created:" from db-init's
# log to tell the person, so keep those starts as they are.
create_admin() {
	[ -n "${ADMIN_USER}" ] && [ -n "${ADMIN_PASSWORD}" ] || return 0
	# userid compares without case, as the login server's lookup does.
	existing=$(mysql_cmd -N -e "SELECT account_id, group_id, user_pass = MD5('${ADMIN_PASSWORD}') FROM login WHERE userid='${ADMIN_USER}' LIMIT 1")
	if [ -z "${existing}" ]; then
		mysql_cmd -e "INSERT INTO login (userid, user_pass, sex, email, group_id) VALUES ('${ADMIN_USER}', MD5('${ADMIN_PASSWORD}'), 'M', 'a@a.com', 99)"
		echo "Created admin account ${ADMIN_USER}"
		return 0
	fi
	# shellcheck disable=SC2086 # splits the three columns
	set -- ${existing}
	if [ "$1" = "1" ]; then
		echo "Admin account not created: ${ADMIN_USER} is the name of the server-to-server login (INTER_USER). Pick another admin name." >&2
	elif [ "$3" != "1" ]; then
		echo "Admin account kept: ${ADMIN_USER} already exists, so its password wasn't changed to ADMIN_PASSWORD. Change it on the portal's account page." >&2
	elif [ "$2" != "99" ]; then
		echo "Admin account kept: ${ADMIN_USER} already exists as a player account (group $2), so it wasn't made an admin." >&2
	else
		echo "Admin account ${ADMIN_USER} already exists"
	fi
}

init_db() {
	if [ -z "$(mysql_cmd -N -e "SHOW TABLES LIKE 'login'")" ]; then
		echo "Empty database: importing rAthena schema"
		import_schema mysql_cmd
	else
		echo "Schema already present: skipping import"
	fi
	check_not_newer
	# Recorded before anything moves, so both databases start from it.
	apply_upgrades mysql_cmd

	init_char_db
	apply_upgrades char_cmd

	# The desktop app's in-game announcements, which the image's
	# npc/wrapper/announcements.txt shows and marks shown. A month of
	# them is kept.
	char_cmd -e "CREATE TABLE IF NOT EXISTS wrapper_announcements (
		id int unsigned NOT NULL AUTO_INCREMENT PRIMARY KEY,
		message varchar(255) NOT NULL,
		created_at datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
		shown_at datetime NULL,
		skipped tinyint(1) NOT NULL DEFAULT 0);
		DELETE FROM wrapper_announcements WHERE created_at < NOW() - INTERVAL 30 DAY"

	# The desktop app's Events screen writes EXP and drop events here;
	# npc/wrapper/rate_events.txt applies them and sets started once
	# announced. Times are Unix seconds, rates percent of the server's own.
	char_cmd -e "CREATE TABLE IF NOT EXISTS wrapper_rate_events (
		id int unsigned NOT NULL AUTO_INCREMENT PRIMARY KEY,
		title varchar(100) NOT NULL,
		starts_at bigint NOT NULL,
		ends_at bigint NOT NULL,
		exp_rate int unsigned NOT NULL DEFAULT 100,
		drop_rate int unsigned NOT NULL DEFAULT 100,
		start_text varchar(255) NOT NULL DEFAULT '',
		end_text varchar(255) NOT NULL DEFAULT '',
		started tinyint(1) NOT NULL DEFAULT 0,
		KEY starts_at (starts_at))"

	# Account 1 is the server-to-server login; replace the stock s1/p1.
	mysql_cmd -e "UPDATE login SET userid='${INTER_USER}', user_pass=MD5('${INTER_PASSWORD}') WHERE account_id=1"

	create_admin
	echo "Database ready"
}

write_imports
apply_wrapper_npcs
apply_custom_npcs
apply_mods

case "${1:-}" in
	init|config) warn_placeholder_passwords ;;
esac

case "${1:-}" in
	init)  init_db ;;
	login) exec ./login-server ;;
	char)  exec ./char-server ;;
	map)
		load_lookup &
		exec ./map-server
		;;
	web)   exec ./web-server ;;
	config) ;;
	*)
		echo "usage: $0 init|login|char|map|web|config" >&2
		exit 64
		;;
esac
