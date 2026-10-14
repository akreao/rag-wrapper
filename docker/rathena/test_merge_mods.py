"""Tests for merge_mods.py: python3 -m unittest docker/rathena/test_merge_mods.py"""

import contextlib
import io
import os
import sys
import tempfile
import textwrap
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import merge_mods  # noqa: E402

ITEM_TMPL = """\
Header:
  Type: ITEM_DB
  Version: 3
"""


class MergeModsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cwd = os.getcwd()
        os.chdir(self.tmp.name)
        self.write("db/import-tmpl/item_db.yml", ITEM_TMPL)
        self.imports = []

    def tearDown(self):
        os.chdir(self.cwd)
        self.tmp.cleanup()

    def write(self, path, text):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(textwrap.dedent(text))

    def mod(self, name, file, body, sub="db"):
        path = f"mods/{name}/{sub}/{file}"
        self.write(path, "Header:\n  Type: X\n  Version: 1\nBody:\n" + textwrap.indent(textwrap.dedent(body), "  "))
        self.imports.append((file, path))

    def run_merge(self):
        with open("imports", "w", encoding="utf-8") as f:
            for file, path in self.imports:
                f.write(f"{file} {path}\n")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(merge_mods.main(["merge_mods.py", "imports", "db/import/mods-merged"]), 0)
        with open("imports", encoding="utf-8") as f:
            added = f.read().splitlines()[len(self.imports):]
        return out.getvalue().splitlines(), added

    def test_different_fields_and_appended_drops_dont_clash(self):
        self.mod("a", "mob_db.yml", """\
            - Id: 1002
              Hp: 100
              Drops:
                - Item: Apple
                  Rate: 5000
            """)
        self.mod("b", "mob_db.yml", """\
            - Id: 1002
              Sp: 10
              Drops:
                - Item: Jellopy
                  Rate: 5000
            """)
        warnings, added = self.run_merge()
        self.assertEqual(warnings, [])
        self.assertEqual(added, [])

    def test_a_drop_two_mods_add_is_kept_once(self):
        self.mod("a", "mob_db.yml", """\
            - Id: 1002
              Drops:
                - Item: Apple
                  Rate: 5000
                - Item: Jellopy
                  Rate: 100
                  StealProtected: true
                - Item: Jellopy
                  Rate: 100
              MvpDrops:
                - Item: Apple
                  Rate: 10
            """)
        self.mod("b", "mob_db.yml", """\
            - Id: 1002
              Drops:
                - Item: jellopy
                  Rate: 100
                  StealProtected: true
                - Item: Apple
                  Rate: 5000
                  Index: 0
            """)
        warnings, added = self.run_merge()
        # Mod a's second Jellopy isn't steal protected, so that one clashes.
        self.assertEqual(len(warnings), 1, warnings)
        self.assertIn("Drops jellopy: mods/a/db/mob_db.yml line 12 adds {Item: Jellopy, Rate: 100}, mods/b/", warnings[0])
        copy = "db/import/mods-merged/mods/a/db/mob_db.yml"
        with open("imports", encoding="utf-8") as f:
            self.assertEqual(f.read().splitlines(), [f"mob_db.yml {copy}", "mob_db.yml mods/b/db/mob_db.yml"])
        with open(copy, encoding="utf-8") as f:
            lines = f.read().split("\n")
        with open("mods/a/db/mob_db.yml", encoding="utf-8") as f:
            original = f.read().split("\n")
        self.assertEqual(len(lines), len(original))
        # Both of mod a's Jellopy drops are left out, nothing else.
        changed = [i + 1 for i, (x, y) in enumerate(zip(original, lines)) if x != y]
        self.assertEqual(changed, [9, 10, 11, 12, 13])
        self.assertEqual(lines[8], "#      - Item: Jellopy")
        doc = merge_mods.yaml.safe_load("\n".join(lines))
        self.assertEqual([d["Item"] for d in doc["Body"][0]["Drops"]], ["Apple"])
        self.assertEqual(doc["Body"][0]["MvpDrops"], [{"Item": "Apple", "Rate": 10}])

    def test_the_same_drop_at_a_different_rate_clashes(self):
        self.mod("a", "mob_db.yml", "- Id: 1002\n  Drops:\n    - Item: Apple\n      Rate: 5000\n")
        self.mod("b", "mob_db.yml", "- Id: 1002\n  Drops:\n    - Item: Apple\n      Rate: 100\n")
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        w = warnings[0]
        self.assertTrue(w.startswith("[Warning]: Mod clash in mob_db.yml, Id 1002, Drops Apple: mods/a/db/mob_db.yml line 7 adds "), w)
        self.assertTrue(w.endswith("the one from mods/b/ is kept (later in load order)"), w)
        self.assertTrue(os.path.exists("db/import/mods-merged/mods/a/db/mob_db.yml"))

    def test_same_value_set_differently_clashes_and_later_wins(self):
        self.mod("a", "item_db.yml", """\
            - Id: 501
              Buy: 100
              Flags:
                BuyingStore: true
            """)
        self.mod("b", "item_db.yml", """\
            - Id: 501
              Buy: 100
            """)
        self.mod("c", "item_db.yml", """\
            - Id: 501
              Buy: 250
              Flags:
                BuyingStore: True
            """, sub="db/pre-re")
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        w = warnings[0]
        self.assertTrue(w.startswith("[Warning]: Mod clash in item_db.yml, Id 501, Buy: "), w)
        self.assertIn("mods/a/db/item_db.yml line 6 sets 100", w)
        self.assertIn("mods/b/db/item_db.yml line 6 sets 100", w)
        self.assertIn("mods/c/db/pre-re/item_db.yml line 6 sets 250", w)
        self.assertTrue(w.endswith("the value from mods/c/ is used (later in load order)"), w)

    def test_keyed_list_entries_clash_only_on_the_same_key(self):
        self.mod("a", "mob_db.yml", """\
            - Id: 1002
              Drops:
                - Item: Apple
                  Rate: 5000
                  Index: 1
            """)
        self.mod("b", "mob_db.yml", """\
            - Id: 1002
              Drops:
                - Index: 1
                  Item: Jellopy
                  Rate: 5000
                - Index: 2
                  Item: Apple
                  Rate: 10
            """)
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        self.assertIn("Id 1002, Drops[Index 1].Item:", warnings[0])
        self.assertIn("sets Apple", warnings[0])
        self.assertIn("sets Jellopy", warnings[0])

    def test_one_mod_setting_a_value_twice_isnt_a_clash(self):
        self.mod("a", "item_db.yml", "- Id: 501\n  Buy: 1\n")
        self.mod("a", "item_db.yml", "- Id: 501\n  Buy: 2\n", sub="db/pre-re")
        warnings, _ = self.run_merge()
        self.assertEqual(warnings, [])

    def test_item_jobs_from_two_mods_are_combined(self):
        self.mod("a", "item_db.yml", """\
            - Id: 1201
              Jobs:
                Novice: true
                Thief: true
            """)
        self.mod("b", "item_db.yml", """\
            - Id: 1201
              Jobs:
                Swordman: true
                Thief: false
            - Id: 1202
              Jobs:
                Mage: true
            """)
        warnings, added = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        self.assertIn("Id 1201, Jobs.Thief:", warnings[0])
        self.assertEqual(added, ["item_db.yml db/import/mods-merged/item_db.yml"])
        with open("db/import/mods-merged/item_db.yml", encoding="utf-8") as f:
            merged = f.read()
        self.assertIn("Header:\n  Type: ITEM_DB\n  Version: 3\nBody:\n  # a, b\n  - Id: 1201\n    Jobs:\n", merged)
        self.assertIn("      Novice: true\n      Thief: false\n      Swordman: true\n", merged)
        self.assertNotIn("1202", merged)
        merged_doc = merge_mods.yaml.safe_load(merged)
        self.assertEqual(merged_doc["Body"], [{"Id": 1201, "Jobs": {"Novice": True, "Thief": False, "Swordman": True}}])

    def test_old_fixups_are_removed(self):
        self.write("db/import/mods-merged/item_db.yml", "old")
        self.mod("a", "item_db.yml", "- Id: 501\n  Buy: 1\n")
        _, added = self.run_merge()
        self.assertEqual(added, [])
        self.assertEqual(os.listdir("db/import/mods-merged"), [])

    def test_whole_lists_compare_as_one_value(self):
        self.mod("a", "abra_db.yml", """\
            - Skill: SM_BASH
              Probability:
                - Level: 1
                  Probability: 500
            """)
        self.mod("b", "abra_db.yml", """\
            - Skill: sm_bash
              Probability:
                - Level: 2
                  Probability: 500
            """)
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        self.assertIn("abra_db.yml, Skill sm_bash, Probability:", warnings[0])

    def test_per_level_lists_match_by_level(self):
        self.mod("a", "skill_db.yml", """\
            - Id: 5
              Range:
                - Level: 1
                  Size: 3
            """)
        self.mod("b", "skill_db.yml", """\
            - Id: 5
              Range:
                - Level: 2
                  Size: 9
            """)
        warnings, _ = self.run_merge()
        self.assertEqual(warnings, [])

    def test_combos_and_job_stats_have_several_keys(self):
        self.mod("a", "item_combos.yml", """\
            - Combos:
                - Combo:
                    - Sword
                    - Shield
              Script: bonus bStr,1;
            """)
        self.mod("b", "item_combos.yml", """\
            - Combos:
                - Combo:
                    - Shield
                    - Sword
                - Combo:
                    - Hat
                    - Sword
              Script: bonus bStr,2;
            """)
        self.mod("a", "job_stats.yml", """\
            - Jobs:
                Novice: true
                Swordman: true
              MaxWeight: 100
            """)
        self.mod("b", "job_stats.yml", """\
            - Jobs:
                Swordman: true
              MaxWeight: 200
            """)
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 2, warnings)
        self.assertIn("item_combos.yml, combo shield+sword, Script:", warnings[0])
        self.assertIn("job_stats.yml, job Swordman, MaxWeight:", warnings[1])

    def test_constants_keep_the_first_value(self):
        self.mod("a", "const.yml", "- Name: MY_CONST\n  Value: 1\n")
        self.mod("b", "const.yml", "- Name: MY_CONST\n  Value: 2\n")
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        self.assertTrue(warnings[0].endswith("the value from mods/a/ is used (earlier in load order)"), warnings[0])

    def test_header_clear_is_warned_about(self):
        self.write("mods/a/db/mob_db.yml", "Header:\n  Type: MOB_DB\n  Version: 4\n  Clear: true\nBody:\n  - Id: 1\n")
        self.imports.append(("mob_db.yml", "mods/a/db/mob_db.yml"))
        warnings, _ = self.run_merge()
        self.assertEqual(len(warnings), 1, warnings)
        self.assertTrue(warnings[0].startswith("[Warning]: mods/a/db/mob_db.yml clears every mob_db.yml entry"), warnings[0])

    def test_broken_files_are_left_to_rathena(self):
        self.write("mods/a/db/item_db.yml", "Body: [\n")
        self.imports.append(("item_db.yml", "mods/a/db/item_db.yml"))
        self.mod("b", "item_db.yml", "- Id: 501\n  Buy: 1\n")
        warnings, _ = self.run_merge()
        self.assertEqual(warnings, [])


if __name__ == "__main__":
    unittest.main()
