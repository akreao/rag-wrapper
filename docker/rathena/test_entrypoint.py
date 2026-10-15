"""Tests for entrypoint.sh's config writing and mod loading, on a small copy
of rAthena's folders: python3 -m unittest docker/rathena/test_entrypoint.py

Runs the script with `config`, which writes the files and stops, so no
database or rAthena build is needed. The database steps (init) are covered
by CI's stack smoke test.
"""

import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import merge_mods  # noqa: E402

ENTRYPOINT = os.path.join(HERE, "entrypoint.sh")

HEADER = "Header:\n  Type: X\n  Version: 1\n"

# rAthena's files that entrypoint.sh reads, cut down to what it looks at.
STOCK = {
    "conf/import-tmpl/battle_conf.txt": "// battle_conf template\n",
    "conf/import-tmpl/char_conf.txt": "// char_conf template\n",
    "conf/import-tmpl/inter_conf.txt": "// inter_conf template\n",
    "conf/import-tmpl/login_conf.txt": "// login_conf template\n",
    "conf/import-tmpl/map_conf.txt": "// map_conf template\n",
    "conf/import-tmpl/log_conf.txt": "// log_conf template\n",
    "conf/import-tmpl/packet_conf.txt": "// packet_conf template\n",
    "conf/import-tmpl/groups.yml": HEADER,
    "db/import-tmpl/item_db.yml": HEADER,
    "db/import-tmpl/mob_db.yml": HEADER,
    "db/import-tmpl/mob_skill_db.txt": "// mob_skill_db template\n",
    "npc/custom/barters.yml": HEADER,
    "npc/scripts_custom.conf": "// Place your scripts here!\n",
}

ENV = {
    "DB_PASSWORD": "dbpass",
    "INTER_USER": "wrapper_inter",
    "INTER_PASSWORD": "interpass",
    "PUBLIC_IP": "192.168.1.20",
}


class EntrypointTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        for path, text in STOCK.items():
            self.write(path, text)
        for folder in ("conf/import", "conf/user", "db/import", "mods"):
            os.makedirs(self.path(folder), exist_ok=True)
        # The image has merge_mods.py on PATH next to the entrypoint.
        bin_dir = self.path("bin")
        os.makedirs(bin_dir)
        with open(os.path.join(bin_dir, "merge_mods.py"), "w", encoding="utf-8") as f:
            f.write(f'#!/bin/sh\nexec "{sys.executable}" "{os.path.join(HERE, "merge_mods.py")}" "$@"\n')
        os.chmod(os.path.join(bin_dir, "merge_mods.py"), 0o755)

    def tearDown(self):
        self.tmp.cleanup()

    def path(self, rel):
        return os.path.join(self.root, rel)

    def write(self, rel, text, newline=None):
        os.makedirs(os.path.dirname(self.path(rel)), exist_ok=True)
        with open(self.path(rel), "w", encoding="utf-8", newline=newline) as f:
            f.write(textwrap.dedent(text))

    def read(self, rel):
        with open(self.path(rel), encoding="utf-8") as f:
            return f.read()

    def run_entrypoint(self, command="config", code=0, **env):
        full = {
            "PATH": self.path("bin") + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"),
            "RATHENA_DIR": self.root,
            **ENV,
            **env,
        }
        result = subprocess.run(["sh", ENTRYPOINT, command], env=full, capture_output=True, text=True)
        self.assertEqual(result.returncode, code, result.stderr)
        return result

    def mod(self, name, rel, text=None):
        self.write(f"mods/{name}/{rel}", HEADER + "Body:\n  - Id: 1\n" if text is None else text)

    def footer(self, rel):
        """The files a database's import file loads after its own entries."""
        text = self.read(rel)
        if "\nFooter:\n" not in text:
            return []
        return [line.split("Path: ", 1)[1].strip("'") for line in text.split("\nFooter:\n", 1)[1].splitlines() if "Path:" in line]

    # Configs

    def test_stack_values_follow_the_overrides_folder(self):
        self.write("conf/user/char_conf.txt", "server_name: Mine\nchar_ip: 10.0.0.1\n")
        self.run_entrypoint()

        # The overrides file is imported first, so the stack's lines after it win.
        self.assertEqual(
            self.read("conf/import/char_conf.txt").splitlines(),
            [
                "import: conf/user/char_conf.txt",
                "pincode_enabled: no",
                "userid: wrapper_inter",
                "passwd: interpass",
                "login_ip: login",
                "char_ip: 192.168.1.20",
            ],
        )
        # No overrides file: no import line.
        self.assertEqual(
            self.read("conf/import/map_conf.txt").splitlines(),
            ["userid: wrapper_inter", "passwd: interpass", "char_ip: char", "map_ip: 192.168.1.20"],
        )
        login = self.read("conf/import/login_conf.txt")
        self.assertIn("use_MD5_passwords: yes\n", login)
        self.assertIn("new_account: no\n", login)

    def test_battle_conf_lets_the_overrides_change_the_command_symbol(self):
        self.run_entrypoint()
        self.assertEqual(self.read("conf/import/battle_conf.txt"), "charcommand_symbol: *\n")

        self.write("conf/user/battle_conf.txt", "charcommand_symbol: @\n")
        self.run_entrypoint()
        self.assertEqual(
            self.read("conf/import/battle_conf.txt").splitlines(),
            ["charcommand_symbol: *", "import: conf/user/battle_conf.txt"],
        )

    def test_other_files_come_from_the_template_or_the_overrides(self):
        self.write("conf/user/log_conf.txt", "log_chat: 1\n")
        self.write("conf/user/groups.yml", HEADER + "Body:\n  - Id: 99\n")
        self.run_entrypoint()

        self.assertEqual(self.read("conf/import/packet_conf.txt"), STOCK["conf/import-tmpl/packet_conf.txt"])
        self.assertEqual(self.read("conf/import/log_conf.txt"), "import: conf/user/log_conf.txt\n")
        # YAML has no import line, so the overrides folder's file is copied.
        self.assertEqual(self.read("conf/import/groups.yml"), HEADER + "Body:\n  - Id: 99\n")

        # Taking a file out of the overrides folder goes back to the template.
        os.remove(self.path("conf/user/log_conf.txt"))
        self.run_entrypoint()
        self.assertEqual(self.read("conf/import/log_conf.txt"), STOCK["conf/import-tmpl/log_conf.txt"])

    def test_accounts_and_characters_have_a_database_per_mode(self):
        self.write("conf/user/inter_conf.txt", "char_server_db: other\n")
        self.run_entrypoint()
        inter = self.read("conf/import/inter_conf.txt").splitlines()
        self.assertEqual(inter[0], "import: conf/user/inter_conf.txt")
        self.assertIn("login_server_db: ragnarok", inter)
        self.assertIn("ipban_db_db: ragnarok", inter)
        for db in ("char_server", "map_server", "web_server", "log_db"):
            self.assertIn(f"{db}_db: ragnarok_prere", inter)
            self.assertIn(f"{db}_pw: dbpass", inter)
            self.assertIn(f"{db}_ip: db", inter)

        self.run_entrypoint(PRERE="no")
        inter = self.read("conf/import/inter_conf.txt").splitlines()
        self.assertIn("login_server_db: ragnarok", inter)
        self.assertIn("char_server_db: ragnarok_re", inter)

    def test_refuses_characters_that_would_break_sql_or_configs(self):
        for key, value in [("INTER_PASSWORD", "it's"), ("ADMIN_USER", "a b"), ("INTER_USER", "x;y")]:
            with self.subTest(key=key):
                result = self.run_entrypoint(code=64, **{key: value})
                self.assertIn("may only use letters, digits and _", result.stderr)
        self.assertFalse(os.path.exists(self.path("conf/import/char_conf.txt")))

    def test_needs_the_passwords(self):
        env = {**os.environ, "RATHENA_DIR": self.root, "INTER_USER": "a", "INTER_PASSWORD": "b"}
        result = subprocess.run(["sh", ENTRYPOINT, "config"], env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DB_PASSWORD", result.stderr)

    def test_unknown_command(self):
        result = self.run_entrypoint("serve", code=64)
        self.assertIn("usage:", result.stderr)

    # Mods

    def test_enabled_txt(self):
        for name in ("a", "b", ".hidden"):
            self.mod(name, "npc/x.txt", "-\tscript\tX\t-1,{}\n")
        self.write(
            "mods/enabled.txt",
            "# load order\r\n  b  \r\na # the second\r\nb\r\nmissing\r\n../a\r\n.hidden\r\n\r\n",
            newline="",
        )
        result = self.run_entrypoint()

        self.assertIn("Mods enabled: b a \n", result.stdout)
        self.assertEqual(self.read("npc/mods.conf").splitlines(), ["npc: mods/b/npc/x.txt", "npc: mods/a/npc/x.txt"])
        self.assertIn("mods/../a/ skipped", result.stderr)
        self.assertIn("mods/.hidden/ skipped", result.stderr)

    def test_no_enabled_txt_loads_nothing(self):
        self.mod("a", "npc/x.txt", "")
        result = self.run_entrypoint()
        self.assertIn("Mods enabled: \n", result.stdout)
        self.assertEqual(self.read("npc/mods.conf"), "")

    def test_scripts_load_in_path_order(self):
        for rel in ("npc/z.txt", "npc/sub/b.txt", "npc/A.txt", "npc/notes.md", "npc/Upper.TXT"):
            self.mod("a", rel, "")
        self.write("mods/enabled.txt", "a\n")
        self.run_entrypoint()
        self.run_entrypoint()

        self.assertEqual(
            self.read("npc/mods.conf").splitlines(),
            ["npc: mods/a/npc/A.txt", "npc: mods/a/npc/Upper.TXT", "npc: mods/a/npc/sub/b.txt", "npc: mods/a/npc/z.txt"],
        )
        # Restarts don't add the import again.
        self.assertEqual(self.read("npc/scripts_custom.conf").count("import: npc/mods.conf"), 1)

    def test_database_files_load_after_db_import_in_mod_order(self):
        self.mod("a", "db/item_db.yml")
        self.mod("b", "db/item_db.yml")
        self.mod("b", "db/pre-re/mob_db.yml")
        self.mod("b", "db/re/mob_db.yml")
        self.mod("b", "db/barters.yml")
        self.mod("b", "db/my_notes.yml")
        self.write("mods/enabled.txt", "b\na\n")
        result = self.run_entrypoint()

        item_db = self.read("db/import/item_db.yml")
        self.assertTrue(item_db.startswith(HEADER), "the import file keeps rAthena's own header")
        self.assertEqual(self.footer("db/import/item_db.yml"), ["mods/b/db/item_db.yml", "mods/a/db/item_db.yml"])
        self.assertEqual(self.footer("db/import/mob_db.yml"), ["mods/b/db/pre-re/mob_db.yml"])
        self.assertEqual(self.footer("npc/custom/barters.yml"), ["mods/b/db/barters.yml"])
        self.assertIn("[Warning]: mods/b/db/my_notes.yml skipped", result.stderr)

        self.run_entrypoint(PRERE="no")
        self.assertEqual(self.footer("db/import/mob_db.yml"), ["mods/b/db/re/mob_db.yml"])

    def test_the_footer_matches_merge_mods_check(self):
        # The desktop app shows clashes from merge_mods.py --check before a
        # restart, so it must find the same files, in the same order.
        self.mod("b", "db/item_db.yml")
        self.mod("b", "db/pre-re/mob_db.yml")
        self.mod("b", "db/barters.yml")
        self.mod("a", "db/item_db.yml")
        self.mod("a", "db/mob_db.yml")
        self.mod("a", "db/unknown.yml")
        self.write("mods/enabled.txt", "b\na\n")
        self.run_entrypoint()

        loaded = []
        for rel in ("db/import/item_db.yml", "db/import/mob_db.yml", "npc/custom/barters.yml"):
            loaded += [(os.path.basename(rel), path) for path in self.footer(rel)]
        cwd = os.getcwd()
        os.chdir(self.root)
        try:
            checked = merge_mods.enabled("mods", "pre-re")
        finally:
            os.chdir(cwd)
        self.assertEqual(sorted(loaded, key=checked.index), checked)
        self.assertEqual(sorted(loaded), sorted(checked))

    def test_merged_jobs_load_after_the_mods(self):
        for name, job in (("a", "Swordman"), ("b", "Mage")):
            self.mod(name, "db/item_db.yml", HEADER + f"Body:\n  - Id: 1201\n    Jobs:\n      {job}: true\n")
        self.write("mods/enabled.txt", "a\nb\n")
        self.run_entrypoint()

        self.assertEqual(
            self.footer("db/import/item_db.yml"),
            ["mods/a/db/item_db.yml", "mods/b/db/item_db.yml", "db/import/mods-merged/item_db.yml"],
        )
        merged = self.read("db/import/mods-merged/item_db.yml")
        self.assertIn("Swordman: true", merged)
        self.assertIn("Mage: true", merged)

    def test_text_databases_get_the_mod_rows_appended(self):
        self.mod("a", "db/mob_skill_db.txt", "1002,Poring@NPC_EMOTION\r\n")
        self.mod("a", "db/unknown_db.txt", "1\n")
        self.write("mods/enabled.txt", "a\n")
        result = self.run_entrypoint()

        self.assertEqual(
            self.read("db/import/mob_skill_db.txt"),
            STOCK["db/import-tmpl/mob_skill_db.txt"] + "\n// mods/a/db/mob_skill_db.txt\n1002,Poring@NPC_EMOTION\n",
        )
        self.assertIn("[Warning]: mods/a/db/unknown_db.txt skipped", result.stderr)

    def test_switching_a_mod_off_leaves_nothing_behind(self):
        self.mod("a", "npc/x.txt", "")
        self.mod("a", "db/item_db.yml")
        self.mod("a", "db/barters.yml")
        self.mod("a", "db/mob_skill_db.txt", "1002,x\n")
        self.write("mods/enabled.txt", "a\n")
        self.run_entrypoint()
        self.assertNotEqual(self.read("db/import/item_db.yml"), HEADER)

        self.write("mods/enabled.txt", "# a\n")
        self.run_entrypoint()
        self.assertEqual(self.read("npc/mods.conf"), "")
        self.assertEqual(self.read("db/import/item_db.yml"), STOCK["db/import-tmpl/item_db.yml"])
        self.assertEqual(self.read("db/import/mob_skill_db.txt"), STOCK["db/import-tmpl/mob_skill_db.txt"])
        self.assertEqual(self.read("npc/custom/barters.yml"), STOCK["npc/custom/barters.yml"])


if __name__ == "__main__":
    unittest.main()
