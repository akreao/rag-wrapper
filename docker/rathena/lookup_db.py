#!/usr/bin/env python3
"""Writes the portal's item and monster lookup tables, as SQL on stdout.

Reads what the map server loads at the same start: the item and monster
databases of this game mode with db/import and the enabled mods' files
(followed through each file's Footer imports, as rAthena does), the drop
rate settings of conf/battle_athena.conf with the overrides folder's
battle_conf.txt, and the monster spawns in the NPC files the map server
loads (npc/<mode>/scripts_main.conf and what it imports, which includes
rAthena's custom NPCs and the mods' scripts).

The tables are filled under new names and swapped in with one RENAME, so
the portal keeps answering from the old ones until the new ones are whole:

  wrapper_items        one row per item
  wrapper_mobs         one row per monster
  wrapper_mob_drops    a monster's drops, with the rate on this server
  wrapper_mob_spawns   where a monster spawns, one row per spawn line
  wrapper_lookup_info  when the tables were filled, and the server's rates

Usage: lookup_db.py pre-re|re   (run from the rAthena folder)

Problems are printed to stderr; the tables are written with what loaded.
"""

import math
import os
import re
import sys
import time

import yaml

# Every value is read as a string, as rAthena does, so a name like "No" or
# "1e5" stays a name.
try:
    Loader = yaml.CBaseLoader
except AttributeError:
    Loader = yaml.BaseLoader

# From rAthena's mob.hpp.
MAX_MOB_DROP = 10
MAX_MVP_DROP = 3

TABLES = ("wrapper_items", "wrapper_mobs", "wrapper_mob_drops", "wrapper_mob_spawns", "wrapper_lookup_info")


def warn(text):
    print(f"[Warning]: {text}", file=sys.stderr)


def read_text(path):
    with open(path, "rb") as f:
        return f.read().decode("utf-8", errors="replace")


# --- YAML databases ---

def yaml_files(path, mode, seen=None):
    """The database files rAthena loads starting from path, in order, as
    (path, document) pairs. A document whose Header says Clear is yielded
    as (path, None) first, to empty what came before."""
    seen = set() if seen is None else seen
    if path in seen:
        return
    seen.add(path)
    try:
        doc = yaml.load(read_text(path), Loader=Loader) or {}
    except (OSError, yaml.YAMLError) as e:
        warn(f"{path} skipped: {e}")
        return
    if not isinstance(doc, dict):
        return
    header = doc.get("Header")
    if isinstance(header, dict) and is_true(header.get("Clear")):
        yield path, None
    yield path, doc
    footer = doc.get("Footer")
    imports = footer.get("Imports") if isinstance(footer, dict) else None
    for entry in imports if isinstance(imports, list) else ():
        if not isinstance(entry, dict) or not isinstance(entry.get("Path"), str):
            continue
        wanted = entry.get("Mode")
        if isinstance(wanted, str) and wanted.lower() != ("prerenewal" if mode == "pre-re" else "renewal"):
            continue
        yield from yaml_files(entry["Path"], mode, seen)


def body(doc):
    entries = doc.get("Body")
    return [e for e in entries if isinstance(e, dict)] if isinstance(entries, list) else []


def is_true(value):
    return isinstance(value, str) and value.strip().lower() == "true"


def to_int(value, default=0):
    if not isinstance(value, str):
        return default
    try:
        return int(value.strip(), 0)
    except ValueError:
        try:
            return int(float(value))
        except ValueError:
            return default


def text(value):
    return value if isinstance(value, str) else ""


def flags(value):
    """The names set to true in a map such as Locations or Jobs."""
    if not isinstance(value, dict):
        return []
    return [k for k, v in value.items() if is_true(v)]


def source(path):
    """Where an entry was added: a mod's name, "import" for db/import, or
    "" for rAthena's own databases."""
    parts = path.replace("\\", "/").split("/")
    # merge_mods.py's copies are under db/import/mods-merged/mods/<name>/.
    if "mods" in parts[:-1]:
        return parts[parts.index("mods") + 1]
    if "import" in parts:
        return "import"
    return ""


def load_entries(start, mode, key="Id"):
    """Entries by key, with later files' fields replacing earlier ones, as
    rAthena applies an import entry with a known key as a partial update.
    Drops and MvpDrops are combined as rAthena does (see merge_drops)."""
    entries = {}
    for path, doc in yaml_files(start, mode):
        if doc is None:
            entries.clear()
            continue
        for entry in body(doc):
            k = to_int(entry.get(key), -1)
            if k <= 0:
                continue
            old = entries.get(k)
            if old is None:
                old = entries[k] = {"_source": source(path)}
            for field, value in entry.items():
                if field in ("Drops", "MvpDrops"):
                    old.setdefault("_" + field, []).append(value)
                else:
                    old[field] = value
    return entries


def merge_drops(lists, limit, items_by_aegis, where):
    """A monster's drops after every file that set them, as rAthena's
    parseDropNode builds them: a drop with an Index replaces that slot (or
    adds the next one), one without is added while there's room."""
    drops = []
    for added in lists:
        for d in added if isinstance(added, list) else ():
            if not isinstance(d, dict):
                continue
            item = items_by_aegis.get(text(d.get("Item")).lower())
            if item is None:
                warn(f"{where}: unknown item {text(d.get('Item'))!r} in its drops")
                continue
            drop = {
                "item": item,
                "rate": to_int(d.get("Rate"), 1),
                "steal_protected": is_true(d.get("StealProtected")),
            }
            if "Index" in d:
                index = to_int(d.get("Index"), -1)
                if index < 0 or index >= limit or index > len(drops):
                    continue
                if index == len(drops):
                    drops.append(drop)
                else:
                    drops[index] = drop
            elif len(drops) < limit:
                drops.append(drop)
    return drops


# --- Battle config ---

def read_conf(path, values, seen=None):
    """key: value settings of a rAthena config file and what it imports."""
    seen = set() if seen is None else seen
    if path in seen:
        return values
    seen.add(path)
    try:
        lines = read_text(path).splitlines()
    except OSError:
        return values
    for line in lines:
        line = line.split("//", 1)[0]
        m = re.match(r"\s*([^:\s]+)\s*:\s*(\S+)", line)
        if not m:
            continue
        key, value = m.group(1), m.group(2)
        if key.lower() == "import":
            read_conf(value, values, seen)
        else:
            values[key.lower()] = value
    return values


def config_switch(value):
    v = value.lower()
    if v in ("on", "yes"):
        return 1
    if v in ("off", "no"):
        return 0
    try:
        return int(v, 0)
    except ValueError:
        return 0


class Rates:
    """The battle config settings that change drop rates and exp, with
    rAthena's defaults from battle.cpp."""

    def __init__(self, values):
        self.values = values

    def get(self, key, default):
        return config_switch(self.values[key]) if key in self.values else default

    def drop(self, base, kind, mob_kind, ratio):
        """The rate (out of 10000) a drop has on this server, from rAthena's
        mob_load_drop_rates and mob_drop_adjust. kind is the item's drop
        category, mob_kind "", "_boss" or "_mvp"; ratio is the item's
        mob_item_ratio.yml value, if any."""
        if kind == "mvp":
            adjust = self.get("item_rate_mvp", 100)
            lo, hi = self.get("item_drop_mvp_min", 1), self.get("item_drop_mvp_max", 10000)
        else:
            if self.get("drop_rateincrease", 0) and base < 5000:
                base += 1
            if kind == "treasure":
                adjust = self.get("item_rate_treasure", 100)
            else:
                adjust = self.get(f"item_rate_{kind}{mob_kind}", 100)
            lo, hi = self.get(f"item_drop_{kind}_min", 1), self.get(f"item_drop_{kind}_max", 10000)
        if ratio is not None:
            adjust = ratio
        if adjust == 0:
            return 0
        rate = float(base)
        if self.get("item_logarithmic_drops", 0) and adjust > 0 and adjust != 100 and base > 0:
            rate = rate * math.pow(5.0 - math.log10(rate), math.log(adjust / 100.0) / math.log(5.0)) + 0.5
        else:
            rate = rate * adjust / 100
        return int(min(max(rate, lo), hi))

    def exp(self, value, key):
        return int(min(max(value * self.get(key, 100) / 100.0, 0), 2**63 - 1))


def drop_kind(item_type):
    """The drop rate setting an item type uses, from mob_load_drop_rates."""
    t = item_type.lower()
    if t == "healing":
        return "heal"
    if t in ("usable", "cash"):
        return "use"
    if t in ("weapon", "armor", "petarmor"):
        return "equip"
    if t == "card":
        return "card"
    return "common"


# --- NPC files ---

def npc_files(conf, seen=None, files=None):
    """The NPC files the map server loads, from its scripts_main.conf, with
    npc:, delnpc: and import: lines as map_reloadnpc_sub reads them."""
    seen = set() if seen is None else seen
    files = [] if files is None else files
    if conf in seen:
        return files
    seen.add(conf)
    try:
        lines = read_text(conf).splitlines()
    except OSError:
        warn(f"{conf} not found")
        return files
    for line in lines:
        line = line.split("//", 1)[0]
        m = re.match(r"([^:]+): (.+)", line)
        if not m:
            continue
        key, value = m.group(1).strip().lower(), m.group(2).strip()
        if key == "npc":
            if value.lower() == "clear":
                files.clear()
            elif value not in files and os.path.isfile(value):
                files.append(value)
        elif key == "delnpc":
            if value.lower() == "all":
                files.clear()
            elif value in files:
                files.remove(value)
        elif key == "import":
            npc_files(value, seen, files)
    return files


COMMENTS = re.compile(r"/\*.*?\*/", re.S)


def spawns(path, mobs_by_aegis, count_rate):
    """The monster and boss_monster lines of an NPC file."""
    try:
        src = COMMENTS.sub("", read_text(path))
    except OSError:
        return
    for line in src.splitlines():
        if "monster\t" not in line.lower():
            continue
        w = line.split("\t")
        if len(w) < 4 or w[1].strip().lower() not in ("monster", "boss_monster"):
            continue
        place = w[0].strip().split(",")
        info = w[3].split("//", 1)[0].strip().split(",")
        if len(info) < 2 or not place[0]:
            continue
        mob = info[0].strip()
        try:
            mob_id = int(mob, 0)
        except ValueError:
            mob_id = mobs_by_aegis.get(mob.lower())
            if mob_id is None:
                continue
        num = to_int(info[1])
        if num < 1 or num > 1000:
            continue
        if num > 1 and count_rate != 100:
            num = max(1, num * count_rate // 100)
        coords = [to_int(c) for c in place[1:5]] + [0] * 4
        yield {
            "mob": mob_id,
            "map": place[0].strip(),
            "x": coords[0], "y": coords[1], "xs": coords[2], "ys": coords[3],
            "amount": num,
            "delay1": to_int(info[2]) if len(info) > 2 else 5000,
            "delay2": to_int(info[3]) if len(info) > 3 else 0,
            "boss": w[1].strip().lower() == "boss_monster",
        }


# --- SQL ---

def sql(value):
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "1" if value else "0"
    if isinstance(value, int):
        return str(value)
    s = str(value)
    for a, b in (("\\", "\\\\"), ("'", "\\'"), ("\0", "\\0"), ("\n", "\\n"), ("\r", "\\r"), ("\x1a", "\\Z")):
        s = s.replace(a, b)
    return "'" + s + "'"


def inserts(out, table, columns, rows):
    batch = []
    for row in rows:
        batch.append("(" + ",".join(sql(v) for v in row) + ")")
        if len(batch) == 500:
            out.write(f"INSERT INTO {table}_new ({','.join(columns)}) VALUES {','.join(batch)};\n")
            batch = []
    if batch:
        out.write(f"INSERT INTO {table}_new ({','.join(columns)}) VALUES {','.join(batch)};\n")


SCHEMA = {
    "wrapper_items": """
        id int unsigned NOT NULL PRIMARY KEY,
        aegis varchar(100) NOT NULL,
        name varchar(100) NOT NULL,
        type varchar(20) NOT NULL,
        subtype varchar(20) NOT NULL,
        buy int unsigned NOT NULL,
        sell int unsigned NOT NULL,
        weight int unsigned NOT NULL,
        attack int unsigned NOT NULL,
        magic_attack int unsigned NOT NULL,
        defense int unsigned NOT NULL,
        `range` int unsigned NOT NULL,
        slots int unsigned NOT NULL,
        weapon_level int unsigned NOT NULL,
        armor_level int unsigned NOT NULL,
        equip_level_min int unsigned NOT NULL,
        equip_level_max int unsigned NOT NULL,
        refineable tinyint(1) NOT NULL,
        gender varchar(10) NOT NULL,
        jobs text NOT NULL,
        locations text NOT NULL,
        script mediumtext NOT NULL,
        equip_script mediumtext NOT NULL,
        unequip_script mediumtext NOT NULL,
        source varchar(100) NOT NULL,
        KEY name (name)""",
    "wrapper_mobs": """
        id int unsigned NOT NULL PRIMARY KEY,
        aegis varchar(100) NOT NULL,
        name varchar(100) NOT NULL,
        level int unsigned NOT NULL,
        hp bigint unsigned NOT NULL,
        sp bigint unsigned NOT NULL,
        base_exp bigint unsigned NOT NULL,
        job_exp bigint unsigned NOT NULL,
        mvp_exp bigint unsigned NOT NULL,
        attack int unsigned NOT NULL,
        attack2 int unsigned NOT NULL,
        defense int NOT NULL,
        magic_defense int NOT NULL,
        resistance int NOT NULL,
        magic_resistance int NOT NULL,
        str int unsigned NOT NULL,
        agi int unsigned NOT NULL,
        vit int unsigned NOT NULL,
        `int` int unsigned NOT NULL,
        dex int unsigned NOT NULL,
        luk int unsigned NOT NULL,
        attack_range int unsigned NOT NULL,
        size varchar(20) NOT NULL,
        race varchar(20) NOT NULL,
        element varchar(20) NOT NULL,
        element_level int unsigned NOT NULL,
        walk_speed int unsigned NOT NULL,
        attack_delay int unsigned NOT NULL,
        class varchar(20) NOT NULL,
        mvp tinyint(1) NOT NULL,
        modes text NOT NULL,
        source varchar(100) NOT NULL,
        KEY name (name)""",
    "wrapper_mob_drops": """
        mob_id int unsigned NOT NULL,
        slot int unsigned NOT NULL,
        mvp tinyint(1) NOT NULL,
        item_id int unsigned NOT NULL,
        base_rate int unsigned NOT NULL,
        rate int unsigned NOT NULL,
        steal_protected tinyint(1) NOT NULL,
        PRIMARY KEY (mob_id, mvp, slot),
        KEY item_id (item_id)""",
    "wrapper_mob_spawns": """
        id int unsigned NOT NULL PRIMARY KEY,
        mob_id int unsigned NOT NULL,
        map varchar(20) NOT NULL,
        x int NOT NULL,
        y int NOT NULL,
        xs int NOT NULL,
        ys int NOT NULL,
        amount int unsigned NOT NULL,
        delay1 int unsigned NOT NULL,
        delay2 int unsigned NOT NULL,
        boss tinyint(1) NOT NULL,
        file varchar(255) NOT NULL,
        KEY mob_id (mob_id)""",
    "wrapper_lookup_info": """
        name varchar(40) NOT NULL PRIMARY KEY,
        value varchar(255) NOT NULL""",
}


COLUMNS = {
    "wrapper_items": (
        "id", "aegis", "name", "type", "subtype", "buy", "sell", "weight", "attack", "magic_attack", "defense",
        "`range`", "slots", "weapon_level", "armor_level", "equip_level_min", "equip_level_max", "refineable",
        "gender", "jobs", "locations", "script", "equip_script", "unequip_script", "source",
    ),
    "wrapper_mobs": (
        "id", "aegis", "name", "level", "hp", "sp", "base_exp", "job_exp", "mvp_exp", "attack", "attack2",
        "defense", "magic_defense", "resistance", "magic_resistance", "str", "agi", "vit", "`int`", "dex", "luk",
        "attack_range", "size", "race", "element", "element_level", "walk_speed", "attack_delay", "class", "mvp",
        "modes", "source",
    ),
    "wrapper_mob_drops": ("mob_id", "slot", "mvp", "item_id", "base_rate", "rate", "steal_protected"),
    "wrapper_mob_spawns": ("id", "mob_id", "map", "x", "y", "xs", "ys", "amount", "delay1", "delay2", "boss", "file"),
    "wrapper_lookup_info": ("name", "value"),
}


def collect(mode):
    """Every table's rows, by table name, read from the rAthena folder
    that is the current directory."""
    items = load_entries("db/item_db.yml", mode)
    items = {k: v for k, v in items.items() if text(v.get("AegisName")) and text(v.get("Name"))}
    items_by_aegis = {text(v["AegisName"]).lower(): k for k, v in items.items()}

    mobs = load_entries("db/mob_db.yml", mode)
    mobs = {k: v for k, v in mobs.items() if text(v.get("AegisName")) and text(v.get("Name"))}
    mobs_by_aegis = {text(v["AegisName"]).lower(): k for k, v in mobs.items()}

    # mob_item_ratio.yml: an item's own drop rate setting, for every
    # monster or the ones listed.
    ratios = {}
    for _, doc in yaml_files("db/mob_item_ratio.yml", mode):
        if doc is None:
            ratios.clear()
            continue
        for entry in body(doc):
            item = items_by_aegis.get(text(entry.get("Item")).lower())
            if item is None:
                continue
            r = ratios.setdefault(item, {"ratio": None, "mobs": set()})
            if "Ratio" in entry:
                r["ratio"] = to_int(entry.get("Ratio"))
            listed = entry.get("List")
            if isinstance(listed, dict):
                for name, on in listed.items():
                    mob = mobs_by_aegis.get(name.lower())
                    if mob is not None:
                        (r["mobs"].add if is_true(on) else r["mobs"].discard)(mob)
    ratios = {k: v for k, v in ratios.items() if v["ratio"] is not None}

    rates = Rates(read_conf("conf/battle_athena.conf", {}))

    drops = []
    mob_rows = []
    for k, m in sorted(mobs.items()):
        mvp = "Mvp" in flags(m.get("Modes"))
        boss = text(m.get("Class")).lower() == "boss"
        treasure = "Treasure" in flags(m.get("RaceGroups"))
        mob_kind = "_mvp" if mvp else "_boss" if boss else ""
        mob_rows.append(mob_row(k, m, mvp, rates))
        where = f"monster {k} ({text(m.get('AegisName'))})"
        for is_mvp, field, limit in ((True, "_MvpDrops", MAX_MVP_DROP), (False, "_Drops", MAX_MOB_DROP)):
            for slot, d in enumerate(merge_drops(m.get(field, []), limit, items_by_aegis, where)):
                if d["rate"] <= 0:
                    continue
                ratio = ratios.get(d["item"])
                if ratio and ratio["mobs"] and k not in ratio["mobs"]:
                    ratio = None
                if is_mvp:
                    kind = "mvp"
                elif treasure:
                    kind = "treasure"
                else:
                    kind = drop_kind(text(items[d["item"]].get("Type")) or "Etc")
                rate = rates.drop(d["rate"], kind, mob_kind, ratio["ratio"] if ratio else None)
                if rate <= 0:
                    continue
                drops.append((k, slot, is_mvp, d["item"], d["rate"], rate, d["steal_protected"]))

    count_rate = rates.get("mob_count_rate", 100)
    spawn_rows = []
    for path in npc_files(f"npc/{mode}/scripts_main.conf"):
        for s in spawns(path, mobs_by_aegis, count_rate):
            if s["mob"] not in mobs:
                continue
            spawn_rows.append((len(spawn_rows) + 1, s["mob"], s["map"], s["x"], s["y"], s["xs"], s["ys"],
                               s["amount"], s["delay1"], s["delay2"], s["boss"], path))

    info = {"loaded_at": str(int(time.time())), "mode": mode}
    for key in ("base_exp_rate", "job_exp_rate", "item_rate_common", "item_rate_heal", "item_rate_use",
                "item_rate_equip", "item_rate_card", "item_rate_mvp"):
        info[key] = str(rates.get(key, 100))

    return {
        "wrapper_items": [item_row(k, v) for k, v in sorted(items.items())],
        "wrapper_mobs": mob_rows,
        "wrapper_mob_drops": drops,
        "wrapper_mob_spawns": spawn_rows,
        "wrapper_lookup_info": sorted(info.items()),
    }


def write_sql(out, tables):
    for table, columns in SCHEMA.items():
        out.write(f"DROP TABLE IF EXISTS {table}_new;\nCREATE TABLE {table}_new ({columns}) DEFAULT CHARSET=utf8mb4;\n")
    for table, rows in tables.items():
        inserts(out, table, COLUMNS[table], rows)
    # Swapped in together, so the portal never sees half of a refill.
    for table in TABLES:
        out.write(f"CREATE TABLE IF NOT EXISTS {table} LIKE {table}_new;\n")
    out.write("DROP TABLE IF EXISTS " + ", ".join(f"{t}_old" for t in TABLES) + ";\n")
    out.write("RENAME TABLE " + ", ".join(f"{t} TO {t}_old, {t}_new TO {t}" for t in TABLES) + ";\n")
    out.write("DROP TABLE " + ", ".join(f"{t}_old" for t in TABLES) + ";\n")
    # Replaced by wrapper_items.
    out.write("DROP TABLE IF EXISTS wrapper_item_names;\n")


def main(argv):
    if len(argv) != 2 or argv[1] not in ("pre-re", "re"):
        print("usage: lookup_db.py pre-re|re", file=sys.stderr)
        return 64
    tables = collect(argv[1])
    write_sql(sys.stdout, tables)
    print(f"Lookup tables: {len(tables['wrapper_items'])} items, {len(tables['wrapper_mobs'])} monsters, "
          f"{len(tables['wrapper_mob_drops'])} drops, {len(tables['wrapper_mob_spawns'])} spawns", file=sys.stderr)
    return 0


def item_row(k, v):
    item_type = text(v.get("Type")) or "Etc"
    buy, sell = v.get("Buy"), v.get("Sell")
    # itemdb.cpp: a missing price is worked out from the other one.
    if buy is None and sell is not None:
        buy_v, sell_v = to_int(sell) * 2, to_int(sell)
    elif sell is None:
        buy_v, sell_v = to_int(buy), to_int(buy) // 2
    else:
        buy_v, sell_v = to_int(buy), to_int(sell)
    jobs = flags(v.get("Jobs"))
    return (
        k, text(v.get("AegisName")), text(v.get("Name")), item_type, text(v.get("SubType")),
        buy_v, sell_v, to_int(v.get("Weight")), to_int(v.get("Attack")), to_int(v.get("MagicAttack")),
        to_int(v.get("Defense")), to_int(v.get("Range")), to_int(v.get("Slots")),
        to_int(v.get("WeaponLevel"), 1 if item_type == "Weapon" else 0),
        to_int(v.get("ArmorLevel"), 1 if item_type == "Armor" else 0),
        to_int(v.get("EquipLevelMin")), to_int(v.get("EquipLevelMax")), is_true(v.get("Refineable")),
        text(v.get("Gender")) or "Both",
        ", ".join(jobs) if "Jobs" in v else "All",
        ", ".join(flags(v.get("Locations"))),
        text(v.get("Script")).strip(), text(v.get("EquipScript")).strip(), text(v.get("UnEquipScript")).strip(),
        v.get("_source", ""),
    )


def mob_row(k, m, mvp, rates):
    def n(field, default=0):
        return to_int(m.get(field), default)
    return (
        k, text(m.get("AegisName")), text(m.get("Name")), n("Level", 1), n("Hp", 1), n("Sp", 1),
        rates.exp(n("BaseExp"), "base_exp_rate"), rates.exp(n("JobExp"), "job_exp_rate"),
        rates.exp(n("MvpExp"), "mvp_exp_rate"),
        n("Attack"), n("Attack2"), n("Defense"), n("MagicDefense"), n("Resistance"), n("MagicResistance"),
        n("Str", 1), n("Agi", 1), n("Vit", 1), n("Int", 1), n("Dex", 1), n("Luk", 1), n("AttackRange"),
        text(m.get("Size")) or "Small", text(m.get("Race")) or "Formless", text(m.get("Element")) or "Neutral",
        n("ElementLevel", 1), n("WalkSpeed", 150), n("AttackDelay"), text(m.get("Class")) or "Normal", mvp,
        ", ".join(flags(m.get("Modes"))), m.get("_source", ""),
    )


if __name__ == "__main__":
    sys.exit(main(sys.argv))
