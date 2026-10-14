#!/usr/bin/env python3
"""Checks how the enabled mods' database files combine, before rAthena loads them.

rAthena loads each mod's db/*.yml as an import, in enabled.txt order, and
applies an entry with an existing key as a partial update. For almost every
list-shaped field that already combines: a monster's Drops without an Index
are added to the ones it has, and keyed lists (a skill tree's skills, an
item group's items) change only the sub-entries a file names. Two things
don't combine on their own, and this script handles them:

- Two mods setting the same single value (an item's Buy price, the drop in
  one Drops Index). rAthena keeps the later mod's, as before; this prints a
  "Mod clash" warning naming both files, which the desktop app's Mods
  screen shows under each of the mods.
- Fields rAthena clears before applying an import's value. An item's Jobs
  is the one that loses changes: each import resets the job mask to the
  jobs it lists, so a second mod's Jobs drops the first one's. When more
  than one mod sets Jobs on the same item, this writes a file that sets the
  combined jobs, loaded after the mods. The other cleared lists
  (RULES' "whole" ones) are rare and each mod sets them as a whole; two
  mods setting one differently is a clash.
- The same drop added by more than one mod. A monster's Drops (and
  MvpDrops) without an Index are added to its list, so two mods adding the
  same item would give it twice, using two of its 10 drop slots and
  rolling for it twice. Only the last mod's copy is kept: the earlier
  mods' files load from copies in OUT_DIR with that drop commented out,
  line for line, so errors still point at the right line. A different
  Rate is a clash.

Usage: merge_mods.py IMPORTS OUT_DIR

IMPORTS lists the mods' files in load order, "<database file> <path>" per
line, as entrypoint.sh builds it. Files written to OUT_DIR are added to
IMPORTS, after the mods, so they load last, and the copies replace their
originals in IMPORTS. Problems are printed as
rAthena-style [Warning] lines with the files' paths.
"""

import os
import shutil
import sys

import yaml

try:
    Loader = yaml.CSafeLoader
except AttributeError:
    Loader = yaml.SafeLoader

# How each database's entries are keyed, and how its list fields combine
# when imports change an entry that exists, from rAthena's loaders.
#
# "key" is the field (or fields, the first present wins) that names an
# entry; "Jobs*" means every key of the Jobs map, "Combos*" every combo's
# item set. "lists" maps a field path (mapping fields joined by ".") to how
# rAthena treats a sequence there:
#   a field name, or names   sub-entries matched by that field and changed
#                            in place; one without it is added ("append")
#   "append"                 each sub-entry is added to the list
#   "whole"                  rAthena clears the list first, so the whole
#                            list is one value
#   "position"               matched by position (or Level, when given)
# A sequence not listed here whose sub-entries have a Level is matched by
# Level (rAthena's per-level fields); any other is "whole".
# "replace_maps" are maps rAthena clears before applying (item Jobs).
# "one_copy" are appended lists where a sub-entry with the same value of
# the given field, from more than one mod, is kept only once (the last).
RULES = {
    "item_db.yml": {"key": ["Id"], "replace_maps": {"Jobs"}},
    "item_group_db.yml": {"key": ["Group"], "lists": {"SubGroups": ["SubGroup"], "SubGroups.List": ["Index"]}},
    "item_combos.yml": {"key": ["Combos*"]},
    "item_randomopt_db.yml": {"key": ["Id"]},
    "item_randomopt_group.yml": {"key": ["Id"], "lists": {"Slots": ["Slot"], "Slots.Options": "whole", "Random": "whole"}},
    "laphine_synthesis.yml": {"key": ["Item"], "lists": {"Requirements": ["Item"]}},
    "laphine_upgrade.yml": {"key": ["Item"], "lists": {"TargetItems": "append"}},
    "item_reform.yml": {"key": ["Item"], "lists": {"BaseItems": ["BaseItem"], "BaseItems.Materials": ["Material"]}},
    "item_enchant.yml": {
        "key": ["Id"],
        "lists": {
            "Reset.Materials": ["Material"],
            "Order": "whole",
            "Slots": ["Slot"],
            "Slots.Materials": ["Material"],
            "Slots.EnchantgradeBonus": ["Enchantgrade"],
            "Slots.Enchants": ["Enchantgrade"],
            "Slots.Enchants.Items": ["Item"],
            "Slots.PerfectEnchants": ["Item"],
            "Slots.PerfectEnchants.Materials": ["Material"],
            "Slots.Upgrades": ["Enchant"],
            "Slots.Upgrades.Materials": ["Material"],
        },
    },
    "item_packages.yml": {"key": ["Item"], "lists": {"Groups": ["Group"], "Groups.Items": ["Item"]}},
    "item_cash.yml": {"key": ["Tab"], "lists": {"Items": ["Item"]}},
    "attendance.yml": {"key": ["Start"], "lists": {"Rewards": ["Day"]}},
    "reputation.yml": {"key": ["Id"]},
    "reputation_group.yml": {"key": ["Id"], "lists": {"ReputeList": "append"}},
    "level_penalty.yml": {"key": ["Type"], "lists": {"LevelDifferences": ["Difference"]}},
    "skill_tree.yml": {"key": ["Job"], "lists": {"Tree": ["Name"], "Tree.Requires": ["Name"]}},
    "job_stats.yml": {"key": ["Jobs*"]},
    "statpoint.yml": {"key": ["Level"]},
    "captcha_db.yml": {"key": ["Id"]},
    "mob_db.yml": {"key": ["Id"], "lists": {"Drops": ["Index"], "MvpDrops": ["Index"]}, "one_copy": {"Drops": "Item", "MvpDrops": "Item"}},
    "mob_avail.yml": {"key": ["Mob"]},
    "mob_summon.yml": {"key": ["Group"], "lists": {"Summon": ["Mob"]}},
    "mob_chat_db.yml": {"key": ["Id"]},
    "mob_item_ratio.yml": {"key": ["Item"]},
    "map_drops.yml": {
        "key": ["Map"],
        "lists": {"GlobalDrops": ["Index"], "SpecificDrops": ["Monster"], "SpecificDrops.Drops": ["Index"]},
    },
    "skill_db.yml": {"key": ["Id"], "lists": {"Requires.ItemCost": "position"}},
    "spellbook_db.yml": {"key": ["Skill"]},
    "magicmushroom_db.yml": {"key": ["Skill"]},
    "create_arrow_db.yml": {"key": ["Source"], "lists": {"Make": ["Item"]}},
    "abra_db.yml": {"key": ["Skill"], "lists": {"Probability": "whole"}},
    "refine.yml": {"key": ["Group"], "lists": {"Levels": ["Level"], "Levels.RefineLevels": ["Level"], "Levels.RefineLevels.Chances": ["Type"]}},
    "enchantgrade.yml": {
        "key": ["Type"],
        "lists": {"Levels": ["Level"], "Levels.Grades": ["Grade"], "Levels.Grades.Chances": ["Refine"], "Levels.Grades.Options": ["Option"]},
    },
    "size_fix.yml": {"key": ["Weapon"]},
    "attr_fix.yml": {"key": ["Level"]},
    "status.yml": {"key": ["Status"]},
    "exp_homun.yml": {"key": ["Level"]},
    "homunculus_db.yml": {"key": ["Class"], "lists": {"Status": ["Type"], "SkillTree": ["Skill"], "SkillTree.Required": ["Skill"]}},
    "mercenary_db.yml": {"key": ["Id"], "lists": {"Skills": ["Name"]}},
    "elemental_db.yml": {"key": ["Id"]},
    "pet_db.yml": {"key": ["Mob"], "lists": {"Evolution": ["Target"], "Evolution.ItemRequirements": ["Item"]}},
    "stylist.yml": {"key": ["Look"], "lists": {"Options": ["Index"]}},
    "achievement_db.yml": {"key": ["Id"], "lists": {"Targets": ["Id"]}},
    "achievement_level_db.yml": {"key": ["Level"]},
    "guild_skill_tree.yml": {"key": ["Id"], "lists": {"Required": ["Id"]}},
    "castle_db.yml": {"key": ["Id"]},
    "battleground_db.yml": {"key": ["Id"], "lists": {"Locations": "append"}},
    "instance_db.yml": {"key": ["Id"]},
    "quest_db.yml": {"key": ["Id"], "lists": {"Targets": ["Mob", "Id"], "Drops": ["Mob"]}},
    "exp_guild.yml": {"key": ["Level"]},
    # A constant that's already defined keeps its first value.
    "const.yml": {"key": ["Name"], "first_wins": True},
}

# rAthena's own import files, relative to /rathena.
IMPORT_TMPL = "db/import-tmpl"

# Longest value quoted in a warning.
MAX_VALUE = 40


def warn(text):
    print(f"[Warning]: {text}", flush=True)


def child(node, name):
    """The value node under `name` in a mapping node."""
    if isinstance(node, yaml.MappingNode):
        for k, v in node.value:
            if k.value == name:
                return v
    return None


def scalar(node):
    return node.value.strip() if isinstance(node, yaml.ScalarNode) else None


def norm(value):
    """A key as rAthena matches it: names ignore case."""
    return value.lower()


def text(node):
    """A value as one short line, to compare and quote."""
    if isinstance(node, yaml.ScalarNode):
        out = " ".join(node.value.split())
    else:
        out = flow(node)
    return out if len(out) <= MAX_VALUE else out[: MAX_VALUE - 3] + "..."


def flow(node):
    """A list or map in YAML's one-line flow style."""
    if isinstance(node, yaml.SequenceNode):
        return "[" + ", ".join(flow(e) for e in node.value) + "]"
    if isinstance(node, yaml.MappingNode):
        return "{" + ", ".join(f"{k.value}: {flow(v)}" for k, v in node.value) + "}"
    return " ".join(node.value.split())


def same(a, b):
    if isinstance(a, yaml.ScalarNode) and isinstance(b, yaml.ScalarNode):
        x, y = " ".join(a.value.split()), " ".join(b.value.split())
        return x == y or x.lower() in ("true", "false") and x.lower() == y.lower()
    return flow(a) == flow(b)


def entry_keys(rule, entry):
    """The keys of one Body entry: usually one, but a job_stats entry is
    every job it lists and an item_combos entry every combo."""
    for field in rule["key"]:
        if field == "Jobs*":
            jobs = child(entry, "Jobs")
            if isinstance(jobs, yaml.MappingNode):
                return [f"job {k.value}" for k, _ in jobs.value]
        elif field == "Combos*":
            combos = child(entry, "Combos")
            keys = []
            if isinstance(combos, yaml.SequenceNode):
                for c in combos.value:
                    items = child(c, "Combo")
                    if isinstance(items, yaml.SequenceNode):
                        names = sorted(norm(i.value) for i in items.value if isinstance(i, yaml.ScalarNode))
                        keys.append("combo " + "+".join(names))
            return keys
        else:
            value = scalar(child(entry, field))
            if value:
                return [f"{field} {norm(value)}"]
    return []


def entry_fields(rule):
    """Fields that name the entry rather than set something on it."""
    skip = set()
    for field in rule["key"]:
        skip.add(field.rstrip("*"))
    return skip


def walk(rule, node, path, label, out):
    """Records every value `node` sets, as (path, label) -> node, where
    path is the rule path (mapping fields joined by ".") and label is the
    readable place, with sub-entry keys. Appended sub-entries set nothing
    another mod can clash with."""
    if isinstance(node, yaml.MappingNode):
        for k, v in node.value:
            name = k.value
            p = f"{path}.{name}" if path else name
            lbl = f"{label}.{name}" if label else name
            walk(rule, v, p, lbl, out)
    elif isinstance(node, yaml.SequenceNode):
        how = rule.get("lists", {}).get(path)
        if how is None:
            levelled = node.value and all(child(e, "Level") is not None for e in node.value)
            how = ["Level"] if levelled else "whole"
        if how == "whole":
            out[(path, label)] = node
        elif how == "append":
            pass
        else:
            fields = ["Level"] if how == "position" else how
            for i, e in enumerate(node.value):
                key = None
                for f in fields:
                    value = scalar(child(e, f))
                    if value:
                        key = f"{f} {norm(value)}"
                        sub = [f]
                        break
                if key is None and how == "position":
                    key, sub = f"#{i + 1}", []
                if key is None:
                    # Added to the list, like a Drops entry without an Index.
                    continue
                if isinstance(e, yaml.MappingNode):
                    rest = yaml.MappingNode(e.tag, [(k, v) for k, v in e.value if k.value not in sub])
                    walk(rule, rest, path, f"{label}[{key}]", out)
                else:
                    out[(path, f"{label}[{key}]")] = e
    else:
        out[(path, label)] = node


def load(path):
    with open(path, encoding="utf-8") as f:
        return yaml.compose(f, Loader=Loader)


def check(imports):
    """Reads the mods' files and reports clashes. Returns, per database
    file, the entries whose replace-maps more than one mod set, with each
    mod's map in load order, for the fix-up files; and, per mod file, the
    sub-entries a later mod adds again, to leave out."""
    # (file, entry key, path, label) -> [(mod path, node)]
    seen = {}
    maps = {}
    # (file, entry key, list field, value) -> [(mod path, sub-entry node)]
    copies = {}
    removals = {}
    for file, path in imports:
        rule = RULES.get(file)
        try:
            doc = load(path)
        except (OSError, yaml.YAMLError):
            # rAthena reports the file's own errors when it loads it.
            continue
        header = child(doc, "Header")
        if scalar(child(header, "Clear")) in ("true", "True", "yes"):
            warn(f"{path} clears every {file} entry loaded before it, rAthena's own and other mods' too (Header: Clear)")
        body = child(doc, "Body")
        if rule is None or not isinstance(body, yaml.SequenceNode):
            continue
        skip = entry_fields(rule)
        for entry in body.value:
            if not isinstance(entry, yaml.MappingNode):
                continue
            keys = entry_keys(rule, entry)
            fields = yaml.MappingNode(entry.tag, [(k, v) for k, v in entry.value if k.value not in skip])
            values = {}
            walk(rule, fields, "", "", values)
            for key in keys:
                for (p, label), node in values.items():
                    seen.setdefault((file, key, p, label), []).append((path, node))
                for name in rule.get("replace_maps", ()):
                    node = child(entry, name)
                    if node is not None:
                        maps.setdefault(file, {}).setdefault((key, name), []).append((path, node))
                for name, field in rule.get("one_copy", {}).items():
                    added = child(entry, name)
                    for e in added.value if isinstance(added, yaml.SequenceNode) else ():
                        value = scalar(child(e, field))
                        if not value or child(e, "Index") is not None:
                            continue
                        # Every earlier mod's copy goes; the same mod's stay.
                        where = (file, key, name, norm(value))
                        before = copies.get(where, [])
                        copies[where] = [c for c in before if mod_of(c[0]) == mod_of(path)] + [(path, e)]
                        for earlier, node in before:
                            if mod_of(earlier) == mod_of(path):
                                continue
                            removals.setdefault(earlier, []).append(node)
                            if not same(without(node, field), without(e, field)):
                                warn(
                                    f"Mod clash in {file}, {key}, {name} {value}: {earlier} line {node.start_mark.line + 1} "
                                    f"adds {text(node)}, {path} line {e.start_mark.line + 1} adds {text(e)}; "
                                    f"the one from mods/{mod_of(path)}/ is kept (later in load order)"
                                )

    for (file, key, p, label), sets in seen.items():
        rule = RULES[file]
        mods = []
        for path, node in sets:
            if mods and mods[-1][0] == mod_of(path):
                mods[-1] = (mod_of(path), path, node)
            else:
                mods.append((mod_of(path), path, node))
        if len(mods) < 2 or all(same(mods[0][2], m[2]) for m in mods[1:]):
            continue
        used = mods[0] if rule.get("first_wins") else mods[-1]
        sources = ", ".join(f"{path} line {node.start_mark.line + 1} sets {text(node)}" for _, path, node in mods)
        order = "earlier" if rule.get("first_wins") else "later"
        warn(f"Mod clash in {file}, {key}, {label}: {sources}; the value from mods/{used[0]}/ is used ({order} in load order)")

    combined = {f: {k: v for k, v in m.items() if len({mod_of(p) for p, _ in v}) > 1} for f, m in maps.items()}
    return combined, removals


def without(node, field):
    """A mapping node without one field."""
    return yaml.MappingNode(node.tag, [(k, v) for k, v in node.value if k.value != field])


def write_copies(removals, out_dir):
    """Writes a copy of each mod file with the given sub-entries commented
    out, keeping every line where it was. Returns {original: copy}."""
    copies = {}
    for path, nodes in removals.items():
        with open(path, encoding="utf-8") as f:
            lines = f.read().split("\n")
        for node in nodes:
            first, end = node.start_mark.line, node.end_mark
            # The entry ends where the next one starts, or at a line's end.
            last = end.line if lines[end.line][: end.column].strip() else end.line - 1
            # Only a block-style "- Item: ..." entry on lines of its own.
            if node.flow_style or lines[first][: node.start_mark.column].strip() != "-":
                continue
            for i in range(first, last + 1):
                lines[i] = "#" + lines[i]
        target = os.path.join(out_dir, path)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "w", encoding="utf-8") as f:
            f.write("\n".join(lines))
        copies[path] = target
    return copies


def mod_of(path):
    """The mod's name in a mods/<name>/... path."""
    parts = path.split("/")
    return parts[parts.index("mods") + 1] if "mods" in parts else path


def write_fixups(maps, out_dir):
    """Writes, per database, the combined replace-maps of the entries more
    than one mod set them on. Returns the files written."""
    written = []
    for file, entries in sorted(maps.items()):
        if not entries:
            continue
        lines = [
            "# Written by merge_mods.py on every start: where more than one mod",
            "# sets an item's Jobs, rAthena would keep only the last mod's, so",
            "# this sets the jobs they list together (a later mod's wins for the",
            "# same job). Loaded after the mods.",
            *header(file),
            "Body:",
        ]
        for (key, p), sets in entries.items():
            field, value = key.split(" ", 1)
            lines.append(f"  # {', '.join(dict.fromkeys(mod_of(path) for path, _ in sets))}")
            lines.append(f"  - {field}: {value}")
            lines.append(f"    {p}:")
            combined = {}
            for path, node in sets:
                for k, v in node.value if isinstance(node, yaml.MappingNode) else ():
                    combined[k.value] = v.value
            for k, v in combined.items():
                lines.append(f"      {k}: {v}")
        target = os.path.join(out_dir, file)
        with open(target, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
        written.append((file, target))
    return written


def header(file):
    """The Header of rAthena's own import file for this database, whose
    Type and Version the fix-up file needs."""
    doc = load(os.path.join(IMPORT_TMPL, file))
    node = child(doc, "Header")
    return ["Header:"] + [f"  {k.value}: {v.value}" for k, v in node.value if k.value in ("Type", "Version")]


def main(argv):
    if len(argv) != 3:
        print("usage: merge_mods.py IMPORTS OUT_DIR", file=sys.stderr)
        return 64
    imports_path, out_dir = argv[1], argv[2]
    with open(imports_path, encoding="utf-8") as f:
        imports = [tuple(line.rstrip("\n").split(" ", 1)) for line in f if " " in line]
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    combined, removals = check(imports)
    copies = write_copies(removals, out_dir)
    written = write_fixups(combined, out_dir)
    with open(imports_path, "w", encoding="utf-8") as f:
        for file, path in imports:
            f.write(f"{file} {copies.get(path, path)}\n")
        for file, target in written:
            f.write(f"{file} {target}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
