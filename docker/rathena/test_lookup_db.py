"""Tests for lookup_db.py: python3 -m unittest docker/rathena/test_lookup_db.py"""

import contextlib
import io
import os
import sys
import tempfile
import textwrap
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lookup_db  # noqa: E402

# A small rAthena folder laid out like the real one: db/item_db.yml and
# db/mob_db.yml import the mode's files and then db/import, which imports
# the mods' files the way entrypoint.sh writes its Footer.
FILES = {
    "db/item_db.yml": """
        Header:
          Type: ITEM_DB
        Footer:
          Imports:
          - Path: db/pre-re/item_db.yml
            Mode: Prerenewal
          - Path: db/re/item_db.yml
            Mode: Renewal
          - Path: db/import/item_db.yml
    """,
    "db/pre-re/item_db.yml": """
        Body:
          - Id: 501
            AegisName: Red_Potion
            Name: Red Potion
            Type: Healing
            Buy: 50
            Weight: 70
            Script: |
              itemheal rand(45,65),0;
          - Id: 909
            AegisName: Jellopy
            Name: Jellopy
            Sell: 3
            Weight: 10
          - Id: 1101
            AegisName: Sword
            Name: Sword
            Type: Weapon
            SubType: 1hSword
            Buy: 100
            Attack: 25
            Slots: 3
            Jobs:
              Swordman: true
              Knight: true
            Locations:
              Right_Hand: true
            Refineable: true
          - Id: 4001
            AegisName: Poring_Card
            Name: Poring Card
            Type: Card
          - Id: 7000
            AegisName: No
            Name: "Yes"
    """,
    "db/re/item_db.yml": """
        Body:
          - Id: 501
            AegisName: Red_Potion
            Name: Renewal Potion
    """,
    "db/import/item_db.yml": """
        Header:
          Type: ITEM_DB
        Body:
          - Id: 909
            Name: Better Jellopy
        Footer:
          Imports:
            - Path: 'mods/eden/db/item_db.yml'
    """,
    "mods/eden/db/item_db.yml": """
        Body:
          - Id: 40000
            AegisName: Eden_Coin
            Name: Eden Coin
            Weight: 1
    """,
    "db/mob_db.yml": """
        Footer:
          Imports:
          - Path: db/pre-re/mob_db.yml
            Mode: Prerenewal
          - Path: db/import/mob_db.yml
    """,
    "db/pre-re/mob_db.yml": """
        Body:
          - Id: 1002
            AegisName: PORING
            Name: Poring
            Level: 1
            Hp: 55
            BaseExp: 150
            JobExp: 40
            Race: Plant
            Element: Water
            Drops:
              - Item: Jellopy
                Rate: 7000
              - Item: Red_Potion
                Rate: 100
              - Item: Sword
                Rate: 10
              - Item: Poring_Card
                Rate: 1
                StealProtected: true
          - Id: 1038
            AegisName: OSIRIS
            Name: Osiris
            Level: 78
            Class: Boss
            Modes:
              Mvp: true
            MvpExp: 100
            MvpDrops:
              - Item: Red_Potion
                Rate: 4000
            Drops:
              - Item: Sword
                Rate: 100
          - Id: 1324
            AegisName: TREASURE_BOX1
            Name: Treasure Chest
            RaceGroups:
              Treasure: true
            Drops:
              - Item: Jellopy
                Rate: 1000
    """,
    "db/import/mob_db.yml": """
        Body:
          - Id: 1002
            Drops:
              - Item: Eden_Coin
                Rate: 500
              - Item: Red_Potion
                Rate: 0
                Index: 1
              - Item: Not_An_Item
                Rate: 50
        Footer:
          Imports:
            - Path: mods/eden/db/mob_db.yml
    """,
    "mods/eden/db/mob_db.yml": """
        Body:
          - Id: 3000
            AegisName: EDEN_PORING
            Name: Eden Poring
            Level: 5
            Drops:
              - Item: Eden_Coin
                Rate: 10000
    """,
    "db/mob_item_ratio.yml": """
        Body:
          - Item: Jellopy
            Ratio: 50
            List:
              PORING: true
        Footer:
          Imports:
          - Path: db/import/mob_item_ratio.yml
    """,
    "conf/battle_athena.conf": """
        import: conf/battle/drops.conf
        import: conf/battle/exp.conf
        import: conf/import/battle_conf.txt
    """,
    "conf/battle/drops.conf": """
        // Comments are skipped.
        item_rate_common: 100
        item_rate_equip: 100
        item_rate_equip_boss: 100
        item_rate_card: 100
        item_rate_mvp: 100
        item_rate_treasure: 100
        item_drop_card_min: 1
        item_logarithmic_drops: no
    """,
    "conf/battle/exp.conf": """
        base_exp_rate: 100
        mvp_exp_rate: 100
    """,
    "conf/import/battle_conf.txt": """
        import: conf/user/battle_conf.txt
    """,
    "conf/user/battle_conf.txt": """
        base_exp_rate: 300 // three times
        item_rate_equip: 200
        item_rate_card: 50
        item_drop_card_min: 5
        item_rate_treasure: 300
    """,
    "npc/pre-re/scripts_main.conf": """
        import: npc/pre-re/scripts_monsters.conf
        import: npc/scripts_custom.conf
    """,
    "npc/pre-re/scripts_monsters.conf": """
        npc: npc/pre-re/mobs/fields/prontera.txt
        npc: npc/pre-re/mobs/dungeons/pyramid.txt
        npc: npc/pre-re/mobs/removed.txt
        delnpc: npc/pre-re/mobs/removed.txt
        // npc: npc/pre-re/mobs/commented.txt
    """,
    "npc/scripts_custom.conf": """
        import: npc/mods.conf
    """,
    "npc/mods.conf": """
        npc: mods/eden/npc/spawns.txt
    """,
    "npc/pre-re/mobs/fields/prontera.txt": """
        prt_fild08,0,0\tmonster\tPoring\t1002,70,5000,0,0
        prt_fild08,100,100,10,10\tmonster\tPoring\t1002,5
        /*
        prt_fild09,0,0\tmonster\tPoring\t1002,10
        */
        prt_fild10,0,0\tmonster\tNobody\t9999,10
        //prt_fild12,0,0\tmonster\tPoring\t1002,10
        \t// prt_fild13,0,0\tboss_monster\tOsiris\t1038,1
        -\tscript\tSpawner\t-1,{
        \tmonster "prt_fild01",0,0,"Poring",1002,1;
        }
    """,
    "npc/pre-re/mobs/dungeons/pyramid.txt": """
        moc_pryd04,0,0\tboss_monster\tOsiris\t1038,1,3600000,600000
    """,
    "npc/pre-re/mobs/removed.txt": """
        prt_fild11,0,0\tmonster\tPoring\t1002,10
    """,
    "mods/eden/npc/spawns.txt": """
        prontera,0,0\tmonster\tEden Poring\tEDEN_PORING,3
    """,
}


class LookupDbTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cwd = os.getcwd()
        os.chdir(self.tmp.name)
        for rel, text in FILES.items():
            self.write(rel, text)

    def tearDown(self):
        os.chdir(self.cwd)
        self.tmp.cleanup()

    def write(self, rel, text):
        os.makedirs(os.path.dirname(rel), exist_ok=True)
        with open(rel, "w", encoding="utf-8") as f:
            f.write(textwrap.dedent(text).lstrip("\n"))

    def collect(self, mode="pre-re", inputs=""):
        warnings = io.StringIO()
        with contextlib.redirect_stderr(warnings):
            tables = lookup_db.collect(mode, inputs)
        self.warnings = warnings.getvalue()
        return tables

    def rows(self, tables, table):
        return [dict(zip((c.strip("`") for c in lookup_db.COLUMNS[table]), row)) for row in tables[table]]

    def items(self, tables):
        return {r["id"]: r for r in self.rows(tables, "wrapper_items")}

    def drops(self, tables, mob):
        return [(r["item_id"], r["base_rate"], r["rate"], r["mvp"]) for r in self.rows(tables, "wrapper_mob_drops") if r["mob_id"] == mob]

    def test_items_follow_the_mode_imports_and_mods(self):
        items = self.items(self.collect())
        self.assertEqual(items[501]["name"], "Red Potion")
        self.assertEqual(items[501]["script"], "itemheal rand(45,65),0;")
        # db/import changes one field of an item and keeps the rest.
        self.assertEqual(items[909]["name"], "Better Jellopy")
        self.assertEqual(items[909]["type"], "Etc")
        self.assertEqual(items[909]["source"], "")
        self.assertEqual(items[40000]["source"], "eden")
        self.assertEqual(self.items(self.collect("re"))[501]["name"], "Renewal Potion")

    def test_item_fields(self):
        items = self.items(self.collect())
        sword = items[1101]
        self.assertEqual((sword["buy"], sword["sell"]), (100, 50))
        self.assertEqual(sword["jobs"], "Swordman, Knight")
        self.assertEqual(sword["locations"], "Right_Hand")
        self.assertEqual((sword["slots"], sword["weapon_level"], sword["refineable"]), (3, 1, True))
        # Only a sell price: the buy price is twice that.
        self.assertEqual((items[909]["buy"], items[909]["sell"]), (6, 3))
        self.assertEqual(items[4001]["jobs"], "All")
        # Names YAML 1.1 would read as booleans stay names.
        self.assertEqual((items[7000]["aegis"], items[7000]["name"]), ("No", "Yes"))

    def test_drops_combine_like_rathena_and_use_the_server_rates(self):
        tables = self.collect()
        self.assertEqual(self.drops(tables, 1002), [
            # mob_item_ratio.yml sets Jellopy to 50% of its rate for Poring.
            (909, 7000, 3500, False),
            # The Red Potion's slot was set to rate 0 by db/import: gone.
            # item_rate_equip 200.
            (1101, 10, 20, False),
            # item_rate_card 50 with item_drop_card_min 5.
            (4001, 1, 5, False),
            # Added by db/import after its four drops; the unknown item is skipped.
            (40000, 500, 500, False),
        ])
        self.assertIn("Not_An_Item", self.warnings)
        # An MVP's drops: its prize uses item_rate_mvp, its equipment
        # item_rate_equip_mvp (not set, so 100), not item_rate_equip.
        self.assertEqual(self.drops(tables, 1038), [(501, 4000, 4000, True), (1101, 100, 100, False)])
        # Treasure chests use item_rate_treasure; mob_item_ratio is only for Poring.
        self.assertEqual(self.drops(tables, 1324), [(909, 1000, 3000, False)])
        stolen = [r for r in self.rows(tables, "wrapper_mob_drops") if r["item_id"] == 4001]
        self.assertTrue(stolen[0]["steal_protected"])

    def test_monsters_get_the_exp_rates(self):
        mobs = {r["id"]: r for r in self.rows(self.collect(), "wrapper_mobs")}
        self.assertEqual((mobs[1002]["base_exp"], mobs[1002]["job_exp"]), (450, 40))
        self.assertEqual((mobs[1002]["race"], mobs[1002]["element"], mobs[1002]["size"]), ("Plant", "Water", "Small"))
        self.assertTrue(mobs[1038]["mvp"])
        self.assertEqual(mobs[1038]["class"], "Boss")
        self.assertEqual(mobs[3000]["source"], "eden")
        info = dict(self.collect()["wrapper_lookup_info"])
        self.assertEqual((info["base_exp_rate"], info["item_rate_card"], info["mode"]), ("300", "50", "pre-re"))
        self.assertNotIn("inputs", info)
        # entrypoint.sh's fingerprint of the inputs is kept with the tables.
        self.assertEqual(dict(self.collect(inputs="abc123")["wrapper_lookup_info"])["inputs"], "abc123")

    def test_spawns_come_from_the_loaded_npc_files(self):
        spawns = [(r["mob_id"], r["map"], r["amount"], r["delay1"], r["delay2"], r["boss"])
                  for r in self.rows(self.collect(), "wrapper_mob_spawns")]
        self.assertEqual(spawns, [
            (1002, "prt_fild08", 70, 5000, 0, False),
            (1002, "prt_fild08", 5, 5000, 0, False),
            (1038, "moc_pryd04", 1, 3600000, 600000, True),
            # Commented-out lines (prt_fild12 and 13) aren't spawns.
            # A mod's script, naming the monster by AegisName.
            (3000, "prontera", 3, 5000, 0, False),
        ])

    def test_modes_race_groups_and_locations_change_key_by_key(self):
        self.write("mods/eden/db/mob_db.yml", """
            Body:
              - Id: 1038
                Modes:
                  Aggressive: true
              - Id: 1324
                RaceGroups:
                  Goblin: true
              - Id: 1002
                Modes:
                  Detector: true
              - Id: 1002
                Modes:
                  Detector: false
                  Looter: true
        """)
        self.write("mods/eden/db/item_db.yml", """
            Body:
              - Id: 1101
                Locations:
                  Right_Hand: false
                  Both_Hand: true
                Jobs:
                  Thief: true
        """)
        tables = self.collect()
        mobs = {r["id"]: r for r in self.rows(tables, "wrapper_mobs")}
        # Osiris stays an MVP, with its prize priced as one.
        self.assertEqual((mobs[1038]["mvp"], mobs[1038]["modes"]), (True, "Mvp, Aggressive"))
        self.assertEqual(self.drops(tables, 1038)[0], (501, 4000, 4000, True))
        # Still a treasure chest: item_rate_treasure 300.
        self.assertEqual(self.drops(tables, 1324), [(909, 1000, 3000, False)])
        # A mode set to false is taken off again.
        self.assertEqual(mobs[1002]["modes"], "Looter")
        sword = self.items(tables)[1101]
        self.assertEqual(sword["locations"], "Both_Hand")
        # Jobs is replaced whole, as rAthena resets it.
        self.assertEqual(sword["jobs"], "Thief")

    def test_ai_resets_the_modes(self):
        self.write("mods/eden/db/mob_db.yml", """
            Body:
              - Id: 1038
                Ai: 21
                Modes:
                  Detector: true
        """)
        mobs = {r["id"]: r for r in self.rows(self.collect(), "wrapper_mobs")}
        self.assertEqual((mobs[1038]["mvp"], mobs[1038]["modes"]), (False, "Detector"))

    def test_sql_swaps_the_tables_in_at_the_end(self):
        out = io.StringIO()
        lookup_db.write_sql(out, self.collect())
        sql = out.getvalue()
        self.assertIn("INSERT INTO wrapper_items_new", sql)
        self.assertIn("'Better Jellopy'", sql)
        last = sql.strip().splitlines()[-4:]
        self.assertTrue(last[1].startswith("RENAME TABLE wrapper_items TO wrapper_items_old, wrapper_items_new TO wrapper_items"))
        self.assertEqual(lookup_db.sql("it's a \\ test\n"), "'it\\'s a \\\\ test\\n'")

    def test_a_cleared_database_starts_again(self):
        self.write("db/import/mob_db.yml", """
            Header:
              Type: MOB_DB
              Clear: true
            Body:
              - Id: 3001
                AegisName: ONLY
                Name: Only One
        """)
        mobs = [r["id"] for r in self.rows(self.collect(), "wrapper_mobs")]
        self.assertEqual(mobs, [3001])


if __name__ == "__main__":
    unittest.main()
