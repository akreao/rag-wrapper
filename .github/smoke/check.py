#!/usr/bin/env python3
"""Checks a running stack the way a player uses it: registers an account on
the portal and opens its player pages, then logs in to the login server as
the game client does and enters the char server. Also checks the
smoke-test mod and rAthena's custom NPCs loaded, and that an EXP and drop
event changes the map server's rates without a restart.

Run by run.sh once the stack is up. Standard library only.
"""

import http.cookiejar
import os
import re
import socket
import struct
import subprocess
import sys
import time
import urllib.parse
import urllib.request

PORTAL = f"http://127.0.0.1:{os.environ.get('PORTAL_PORT', '8080')}"
LOGIN = ("127.0.0.1", 6900)
PUBLIC_IP = os.environ["PUBLIC_IP"]
ADMIN = (os.environ["ADMIN_USER"], os.environ["ADMIN_PASSWORD"])
PLAYER = ("smoke_player", "Smoke-pass1")

# rAthena's packets for PACKETVER 20170315 and later (both of the app's presets).
CA_LOGIN = 0x64
AC_ACCEPT_LOGIN = 0xAC4
AC_REFUSE_LOGIN = 0x83E
SC_NOTIFY_BAN = 0x81
CH_ENTER = 0x65
HC_REFUSE_ENTER = 0x6C
# AC_ACCEPT_LOGIN after its type and length, then one SERVER per char server.
ACCEPT = struct.Struct("<III4x26sB17s")
SERVER = struct.Struct("<4sH20sHHH128s")


def step(text):
    print(f"--- {text}", flush=True)


def fail(text):
    print(f"FAILED: {text}", file=sys.stderr, flush=True)
    sys.exit(1)


def wait_for(what, check, timeout=300):
    """Retries check() until it returns something other than None."""
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result is not None:
                return result
        except OSError as e:
            last = e
        time.sleep(3)
    fail(f"timed out waiting for {what} ({last})")


def logs(service):
    return subprocess.run(
        ["docker", "compose", "logs", "--no-color", "--no-log-prefix", service],
        capture_output=True, text=True, check=True,
    ).stdout


def problem_blocks(log):
    """rAthena's errors and warnings, each with the lines that follow it
    up to the next tagged line, except [Debug] lines, which name the
    script a failing command ran in."""
    blocks, block = [], None
    for raw in log.splitlines():
        line = re.sub(r"\x1b\[[0-9;]*m", "", raw.split("\r")[-1])
        if line.startswith("[") and not (block and line.startswith("[Debug]")):
            block = None
            if line.startswith(("[Error]", "[Warning]")):
                block = [line]
                blocks.append(block)
        elif block is not None:
            block.append(line)
    return ["\n".join(b) for b in blocks]


def chars_sql(sql):
    """Runs SQL in this mode's characters database as the database's own
    account, as the desktop app's Events screen does."""
    db = "ragnarok_re" if os.environ.get("PRERE") == "no" else "ragnarok_prere"
    subprocess.run(
        ["docker", "compose", "exec", "-T", "db", "sh", "-c",
         'MYSQL_PWD="$MARIADB_PASSWORD" exec mariadb --user="$MARIADB_USER" "$1" -e "$2"', "sh", db, sql],
        check=True,
    )


def compose_state(service):
    out = subprocess.run(
        ["docker", "compose", "ps", "--all", "--format", "{{.State}} {{.ExitCode}}", service],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    return out if out else None


def recv_exact(sock, n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise ConnectionError(f"connection closed after {len(data)} of {n} bytes")
        data += chunk
    return data


def game_login(user, password):
    """CA_LOGIN to the login server. Returns ("accept", fields, servers),
    ("refuse", error code) or ("ban", reason)."""
    with socket.create_connection(LOGIN, timeout=10) as s:
        s.sendall(struct.pack("<HI24s24sB", CA_LOGIN, 0, user.encode(), password.encode(), 0))
        packet = struct.unpack("<H", recv_exact(s, 2))[0]
        if packet == AC_ACCEPT_LOGIN:
            length = struct.unpack("<H", recv_exact(s, 2))[0]
            body = recv_exact(s, length - 4)
            login_id1, aid, login_id2, _, sex, _ = ACCEPT.unpack_from(body)
            servers = []
            for offset in range(ACCEPT.size, len(body), SERVER.size):
                ip, port, name, _, _, _, _ = SERVER.unpack_from(body, offset)
                servers.append((socket.inet_ntoa(ip), port, name.rstrip(b"\0").decode(errors="replace")))
            return "accept", (aid, login_id1, login_id2, sex), servers
        if packet == AC_REFUSE_LOGIN:
            return "refuse", struct.unpack("<I20s", recv_exact(s, 24))[0]
        if packet == SC_NOTIFY_BAN:
            return "ban", recv_exact(s, 1)[0]
        fail(f"login server answered with packet 0x{packet:04x}")


def enter_char_server(server, fields):
    """CH_ENTER with the login server's session. The char server answers
    with the account ID, then the character list, or HC_REFUSE_ENTER."""
    aid, login_id1, login_id2, sex = fields
    with socket.create_connection(server, timeout=10) as s:
        s.sendall(struct.pack("<HIIIHB", CH_ENTER, aid, login_id1, login_id2, 0, sex))
        echoed = struct.unpack("<I", recv_exact(s, 4))[0]
        if echoed != aid:
            fail(f"char server answered with account {echoed}, not {aid}")
        packet = struct.unpack("<H", recv_exact(s, 2))[0]
        if packet == HC_REFUSE_ENTER:
            fail(f"char server refused account {aid}: error {recv_exact(s, 1)[0]}")
        return packet


def register(user, password):
    jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    form = opener.open(f"{PORTAL}/register", timeout=10).read().decode()
    csrf = re.search(r'name="csrf" value="([^"]+)"', form)
    if not csrf:
        fail("the portal's register page has no csrf field")
    data = urllib.parse.urlencode({
        "csrf": csrf.group(1),
        "userid": user,
        "password": password,
        "password2": password,
        "sex": "F",
        "email": "smoke@example.com",
        "birthdate": "2000-01-01",
    }).encode()
    response = opener.open(f"{PORTAL}/register", data=data, timeout=10)
    if not response.url.endswith("/account"):
        error = re.search(r'class="error"[^>]*>([^<]*)', response.read().decode())
        fail(f"the portal didn't register {user}: {error.group(1) if error else response.url}")


def main():
    step("db-init")
    state = wait_for("db-init to finish", lambda: (s := compose_state("db-init")) and s[0] == "exited" and s or None)
    if state[1] != "0":
        fail(f"db-init exited with {state[1]}")
    init = logs("db-init")
    for line in ("Mods enabled: smoke-test", "Created admin account " + ADMIN[0], "Database ready"):
        if line not in init:
            fail(f"db-init's log has no {line!r}")

    step("portal")
    wait_for("the portal", lambda: urllib.request.urlopen(f"{PORTAL}/healthz", timeout=5).status == 200 or None)
    register(*PLAYER)
    # The public player pages read the characters database of the server's
    # mode, and rankings join it to the accounts database.
    for path, title in (("/online", "Who's online"), ("/rankings?by=zeny&class=0", "Rankings"), ("/news", "News")):
        page = urllib.request.urlopen(f"{PORTAL}{path}", timeout=10).read().decode()
        if f"<h1>{title}</h1>" not in page.replace("&#39;", "'"):
            fail(f"the portal's {path} page didn't load")

    step("login server")
    # The login server says "server closed" until the char server has connected.
    def admin_login():
        result = game_login(*ADMIN)
        return None if result == ("ban", 1) else result
    result = wait_for("the char server to connect to the login server", admin_login)
    if result[0] != "accept":
        fail(f"the admin account's login was refused: {result}")
    servers = result[2]
    if [(ip, port) for ip, port, _ in servers] != [(PUBLIC_IP, 6121)]:
        fail(f"the login server lists {servers}, not one char server at {PUBLIC_IP}:6121")
    print(f"admin logged in; char server {servers[0]}")

    result = game_login(ADMIN[0], ADMIN[1] + "x")
    if result != ("refuse", 1):
        fail(f"a wrong password got {result}, not 'incorrect password'")

    result = game_login(*PLAYER)
    if result[0] != "accept":
        fail(f"the portal's account couldn't log in: {result}")
    print(f"{PLAYER[0]} (account {result[1][0]}) logged in")

    step("char server")
    packet = enter_char_server(("127.0.0.1", servers[0][1]), result[1])
    print(f"char server accepted {PLAYER[0]} (packet 0x{packet:04x})")

    step("map server and mod")
    wait_for("the map server to connect to the char server",
             lambda: "Successfully logged on to Char Server" in logs("map") or None)
    log = wait_for("the mod's script", lambda: next((text for text in [logs("map")] if "Smoke test mod loaded" in text), None))
    if "Smoke test mod loaded: Smoke Test Potion" not in log:
        fail("the mod's script ran but the map server doesn't know its item")
    problems = [line for line in log.splitlines() if "mods/" in line and ("[Error]" in line or "[Warning]" in line)]
    if problems:
        fail("the map server reported problems in the mod:\n" + "\n".join(problems))
    print("smoke-test mod loaded")

    step("portal item and monster lookup")
    # The map server's container fills the tables beside the map server.
    def lookup(path):
        return urllib.request.urlopen(f"{PORTAL}{path}", timeout=10).read().decode()
    page = wait_for("the portal's item search to list the mod's item",
                    lambda: next((p for p in [lookup("/items?q=smoke")] if "Smoke Test Potion" in p), None))
    if "Mod: smoke-test" not in page:
        fail("the portal's item search doesn't say the mod added its item")
    page = lookup("/monsters/1002")
    for text in ("Poring", "prt_fild08", 'href="/items/909"'):
        if text not in page:
            fail(f"the portal's Poring page has no {text!r}")
    if "[Warning]: couldn't fill the portal's item and monster tables" in logs("map"):
        fail("the map server's container couldn't fill the lookup tables")
    print("item and monster pages show the server's data")

    step("rAthena's custom NPCs")
    with open("overrides/custom_npcs.txt", encoding="utf-8") as f:
        wanted = [line.strip() for line in f if line.strip()]
    mode = "re" if os.environ.get("PRERE") == "no" else "pre-re"
    expected = [line.split(": ")[-1] for line in wanted if ": " not in line or line.startswith(mode + ": ")]
    loaded = next((line[len("Custom NPCs on:"):].split() for line in log.splitlines() if line.startswith("Custom NPCs on:")), None)
    if loaded != expected:
        fail(f"the map server loaded custom NPCs {loaded}, not {expected}")
    problems = [block for block in problem_blocks(log) if "npc/custom/" in block]
    if problems:
        fail("the map server reported problems in rAthena's custom NPCs:\n" + "\n\n".join(problems))
    print(f"{len(expected)} of the {len(wanted)} custom NPC scripts loaded in {mode}")

    step("EXP and drop event")
    chars_sql("INSERT INTO wrapper_rate_events (title, starts_at, ends_at, exp_rate, drop_rate, start_text, end_text) "
              "VALUES ('Smoke', UNIX_TIMESTAMP() - 1, UNIX_TIMESTAMP() + 3600, 200, 150, 'Smoke event on', 'Smoke event over')")
    # The script checks every 30 seconds.
    log = wait_for("the event to start", lambda: next((t for t in [logs("map")] if "exp 200%, drops 150%" in t), None), timeout=90)
    # The event's lines end with its debug line, after its base_exp_rate
    # line. It starts from the rates before that: the stock 100, or what
    # another script set (rAthena's Floating Rates custom NPC does).
    upto = log[:log.index("exp 200%, drops 150%")]
    block_start = upto.rindex("battle_config flag 'base_exp_rate'")
    def last_set(text, flag):
        found = re.findall(rf"battle_config flag '{flag}' is now set to '(\d+)'", text)
        return int(found[-1]) if found else None
    for flag, percent in (("base_exp_rate", 200), ("job_exp_rate", 200), ("item_rate_common", 150), ("item_rate_card_mvp", 150)):
        own = last_set(upto[:block_start], flag) or 100
        if last_set(upto[block_start:], flag) != own * percent // 100:
            fail(f"the event didn't set {flag} to {percent}% of {own}")
    chars_sql("DELETE FROM wrapper_rate_events")
    wait_for("the event to end", lambda: "exp 100%, drops 100%" in logs("map") or None, timeout=90)
    problems = [block for block in problem_blocks(logs("map")) if "rate_events" in block or "setbattleflag" in block]
    if problems:
        fail("the map server reported problems in the events script:\n" + "\n\n".join(problems))
    print("an event set the rates and put them back")
    print("Smoke test passed")


if __name__ == "__main__":
    main()
