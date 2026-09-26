"""Offline tests for Pixie mind + harness wiring. No llama-server."""
from __future__ import annotations

import importlib.machinery
import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "bin"
sys.path.insert(0, str(BIN))

import pixie_mind as mind  # noqa: E402


def _load_pixie():
    path = BIN / "pixie"
    loader = importlib.machinery.SourceFileLoader("pixie_under_test", str(path))
    spec = importlib.util.spec_from_loader("pixie_under_test", loader)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["pixie_under_test"] = mod
    spec.loader.exec_module(mod)
    return mod


class TestSpells(unittest.TestCase):
    def test_kit_spells_include_v_and_play(self):
        spells = mind.load_spells(ROOT / "config" / "pixie" / "spells.toml")
        self.assertIn("V", spells)
        self.assertIn("play", spells)
        self.assertTrue(spells["V"].get("confirm"))
        self.assertEqual(spells["play"].get("bin"), "siren")

    def test_resolve_play_rewrites_to_siren(self):
        argv = mind.rewrite_bash("play lofi", mind.load_spells(ROOT / "config" / "pixie" / "spells.toml"))
        self.assertEqual(Path(argv[0]).name, "siren")
        self.assertEqual(argv[1:], ["play", "lofi"])

    def test_unknown_spell(self):
        r = mind.resolve_spell("nope", [], {"spark": {"bin": "true"}})
        self.assertIn("error", r)

    def test_house_runs_true(self):
        r = mind.house("spark", [], {"spark": {"bin": "true", "confirm": False}})
        self.assertEqual(r.get("exit_code"), 0)

    def test_house_path_has_home_bin(self):
        self.assertIn(str(Path.home() / "bin"), mind.house_path())

    def test_spells_line(self):
        line = mind.spells_line({"V": {}, "siren": {}})
        self.assertIn("V", line)
        self.assertIn("siren", line)


class TestMemory(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="pixie-mem-"))
        self.path = self.tmp / "memory.md"

    def test_seed_and_remember(self):
        p = mind.seed_memory(self.path)
        text = p.read_text(encoding="utf-8")
        self.assertIn("Pixie memory", text)
        r = mind.remember("tea at dusk", self.path)
        self.assertTrue(r["wrote"])
        body = self.path.read_text(encoding="utf-8")
        self.assertIn("tea at dusk", body)
        again = mind.remember("tea at dusk", self.path)
        self.assertFalse(again["wrote"])

    def test_empty_fact(self):
        self.assertIn("error", mind.remember("   ", self.path))

    def test_inject_cap(self):
        mind.seed_memory(self.path)
        self.path.write_text("x" * 5000, encoding="utf-8")
        inj = mind.load_memory_inject(self.path, cap=100)
        self.assertLessEqual(len(inj), 120)
        self.assertTrue(inj.endswith("…"))


class TestContext(unittest.TestCase):
    def test_last_tool_entry(self):
        chat = [
            ("user", "hi"),
            ("tool", "Calling tools", "stdout: ok"),
            ("assistant", "done"),
        ]
        last = mind.last_tool_entry(chat)
        self.assertIsNotNone(last)
        self.assertEqual(last[0], "tool")
        self.assertEqual(last[2], "stdout: ok")

    def test_drop_oldest_tools_first(self):
        msgs = [
            {"role": "system", "content": "s"},
            {"role": "user", "content": "u1"},
            {"role": "tool", "content": "t1"},
            {"role": "assistant", "content": "a1"},
            {"role": "tool", "content": "t2"},
            {"role": "user", "content": "u2"},
        ]
        dropped = mind.drop_oldest(msgs, 0.5)
        self.assertTrue(dropped)
        self.assertFalse(any(m.get("role") == "tool" for m in msgs))
        self.assertEqual(msgs[0]["role"], "system")


class TestHarness(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pixie = _load_pixie()

    def test_runes_house_remember_and_build_rounds(self):
        chat = next(r for r in self.pixie._RUNES if r["key"] == "chat")
        names = [n for n, _ in chat["tools"]]
        self.assertEqual(names[0], "house")
        self.assertIn("remember", names)
        build = next(r for r in self.pixie._RUNES if r["key"] == "build")
        self.assertEqual(build.get("rounds"), 10)
        self.assertIn("house", [n for n, _ in build["tools"]])
        plan = next(r for r in self.pixie._RUNES if r["key"] == "plan")
        self.assertNotIn("house", [n for n, _ in plan["tools"]])

    def test_norm_house_splits_spell_string(self):
        got = self.pixie._norm_args("house", {"spell": "siren play lofi"})
        self.assertEqual(got["spell"], "siren")
        self.assertEqual(got["args"], ["play", "lofi"])

    def test_chat_messages_injects_memory_and_last_tool(self):
        rune = next(r for r in self.pixie._RUNES if r["key"] == "chat")
        chat = [
            ("user", "play something"),
            ("tool", "Calling tools", "Routing: house -> siren"),
            ("assistant", "playing"),
            ("user", "what did you run?"),
        ]
        msgs = self.pixie.chat_messages(chat, rune)
        self.assertEqual(msgs[0]["role"], "system")
        self.assertIn("House spells", msgs[0]["content"])
        self.assertIn("House memory", msgs[0]["content"])
        roles = [m["role"] for m in msgs]
        self.assertIn("tool", roles)
        self.assertEqual(roles.count("tool"), 1)

    def test_bash_true(self):
        r = self.pixie.run_bash_command("true")
        self.assertEqual(r.get("exit_code"), 0)

    def test_unknown_house(self):
        r = self.pixie.run_tool("house", {"spell": "definitely-not-a-spell"}, confirm=lambda *_: True)
        self.assertIn("error", r)

    def test_cast_alias(self):
        self.assertEqual(self.pixie._TOOL_ALIASES.get("cast"), "house")
        self.assertEqual(self.pixie._TOOL_ALIASES.get("memory"), "remember")

    def test_deep_plan_runes(self):
        deep = next(r for r in self.pixie._RUNES if r["key"] == "deep")
        self.assertEqual(deep.get("rounds"), 10)
        names = [n for n, _ in deep["tools"]]
        self.assertIn("wiki", names)
        self.assertIn("search_house", names)
        plan = next(r for r in self.pixie._RUNES if r["key"] == "plan")
        self.assertEqual(plan.get("rounds"), 10)
        pnames = [n for n, _ in plan["tools"]]
        self.assertIn("write_plan", pnames)
        self.assertIn("search_house", pnames)
        chat = next(r for r in self.pixie._RUNES if r["key"] == "chat")
        cnames = [n for n, _ in chat["tools"]]
        self.assertIn("web_research", cnames)
        self.assertIn("search_house", cnames)
        self.assertIn("wiki", cnames)

    def test_chrome_pack_full_width(self):
        os.environ["PIXIE_UNICODE"] = "1"
        import fae_termart as art
        lines = self.pixie.chrome_pack(
            80, 24, "Pixie ✦ Chat · x",
            [], ["✦ hi"], "1 Chat  2 Deep",
            band=["▸ y  Approve → Build"],
        )
        self.assertEqual(len(lines), 24)
        for ln in lines:
            self.assertEqual(art.vis_len(ln), 80, art.strip_ansi(ln)[:60])

    def test_cli_chat_flags(self):
        self.assertTrue(callable(self.pixie._print_chats))
        self.assertTrue(callable(self.pixie._print_chat))

    def test_harness_tool_then_session(self):
        tmp = Path(tempfile.mkdtemp(prefix="pixie-h-"))
        r = self.pixie.run_bash_command("true")
        mind.append_session("fake", "user", "run true", rune="build", sessions_dir=tmp)
        mind.append_session(
            "fake", "tool", rune="build", tool="execute_bash",
            args={"command": "true"}, result=r,
            raw='{"name":"execute_bash","arguments":{"command":"true"}}',
            sessions_dir=tmp,
        )
        mind.append_session("fake", "assistant", "ok", rune="build", sessions_dir=tmp)
        recs = mind.read_session("fake", tmp)
        self.assertEqual([x["role"] for x in recs], ["user", "tool", "assistant"])
        self.assertEqual(recs[1]["tool"], "execute_bash")
        self.assertIn("execute_bash", recs[1].get("raw", ""))


class TestSession(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="pixie-sess-"))

    def test_append_roundtrip(self):
        mind.append_session("t1", "user", "hi", rune="chat", sessions_dir=self.tmp)
        mind.append_session(
            "t1", "tool", rune="chat", tool="execute_bash",
            args={"command": "true"}, result={"exit_code": 0},
            raw='{"name":"execute_bash"}', sessions_dir=self.tmp,
        )
        mind.append_session("t1", "assistant", "done", rune="chat", sessions_dir=self.tmp)
        recs = mind.read_session("t1", self.tmp)
        self.assertEqual([r["role"] for r in recs], ["user", "tool", "assistant"])
        self.assertIn("ts", recs[0])
        self.assertEqual(recs[0]["rune"], "chat")
        text = mind.format_chat("t1", self.tmp)
        self.assertIn("you: hi", text)
        self.assertIn("tool execute_bash", text)
        self.assertIn("pixie: done", text)

    def test_list_and_latest(self):
        mind.append_session("a", "user", "first", sessions_dir=self.tmp)
        mind.append_session("b", "user", "second", sessions_dir=self.tmp)
        rows = mind.list_sessions(self.tmp)
        self.assertEqual(rows[0]["id"], "b")
        self.assertEqual(mind.latest_session_id(self.tmp), "b")
        listing = mind.format_chat_list(self.tmp)
        self.assertIn("second", listing)

    def test_cap_result(self):
        mind.append_session(
            "c", "tool", tool="read_local_file",
            result={"content": "x" * 5000}, sessions_dir=self.tmp,
        )
        rec = mind.read_session("c", self.tmp)[0]
        self.assertTrue(rec["result"].get("_truncated"))

    def test_reads_legacy_thin_lines(self):
        p = self.tmp / "old.jsonl"
        p.write_text('{"role": "user", "text": "Hi pixie"}\n', encoding="utf-8")
        text = mind.format_chat("old", self.tmp)
        self.assertIn("Hi pixie", text)

    def test_chat_from_session_rebuilds_tui_tuples(self):
        mind.append_session("t1", "user", "hi", rune="build", sessions_dir=self.tmp)
        mind.append_session(
            "t1", "tool", rune="build", tool="execute_bash",
            args={"command": "true"}, result={"exit_code": 0, "stdout": "ok"},
            sessions_dir=self.tmp,
        )
        mind.append_session("t1", "assistant", "done", rune="build", sessions_dir=self.tmp)
        chat, rune = mind.chat_from_session("t1", self.tmp)
        self.assertEqual(rune, "build")
        self.assertEqual(chat[0], ("user", "hi"))
        self.assertEqual(chat[1][0], "tool")
        self.assertIn("execute_bash", chat[1][1])
        self.assertIn("ok", chat[1][2])
        self.assertEqual(chat[2][0], "assistant")
        self.assertEqual(chat[2][1], "done")

    def test_resolve_session_id(self):
        mind.append_session("keep", "user", "x", sessions_dir=self.tmp)
        self.assertEqual(mind.resolve_session_id("keep", self.tmp), "keep")
        self.assertEqual(mind.resolve_session_id("keep.jsonl", self.tmp), "keep")
        self.assertIsNone(mind.resolve_session_id("missing", self.tmp))
        self.assertEqual(mind.resolve_session_id(None, self.tmp), "keep")

class TestWideTools(unittest.TestCase):
    def test_json_in_prose(self):
        blob = 'I will use execute_bash now.\n{"name": "execute_bash", "arguments": {"command": "ls ~/bin"}}\nDone planning.'
        calls = mind.extract_tool_calls(blob)
        self.assertEqual(calls[0][0]["name"], "execute_bash")
        self.assertEqual(calls[0][0]["arguments"]["command"], "ls ~/bin")

    def test_arguments_as_string(self):
        blob = '{"name": "execute_bash", "arguments": "{\\"command\\": \\"true\\"}"}'
        calls = mind.extract_tool_calls(blob)
        self.assertEqual(calls[0][0]["arguments"]["command"], "true")

    def test_xml_tool_call(self):
        blob = '<tool_call>{"name": "house", "arguments": {"spell": "menagerie"}}</tool_call>'
        calls = mind.extract_tool_calls(blob)
        self.assertEqual(calls[0][0]["name"], "house")
        self.assertEqual(calls[0][0]["arguments"]["spell"], "menagerie")

    def test_invoke_xml(self):
        blob = '<invoke name="read_local_file"><parameter name="path">~/faeOS/bin/pixie</parameter></invoke>'
        calls = mind.extract_tool_calls(blob)
        self.assertEqual(calls[0][0]["name"], "read_local_file")
        self.assertIn("pixie", calls[0][0]["arguments"]["path"])

    def test_narrated_plan_is_flagged(self):
        blob = (
            "I'll use execute_bash to list the contents of ~/faeOS/ "
            "and look for logs."
        )
        self.assertTrue(mind.looks_like_narrated_tools(blob))
        self.assertEqual(mind.extract_tool_calls(blob), [])

    def test_term_width_cap_none(self):
        sys.path.insert(0, str(BIN))
        import fae_termart as art
        capped = art.term_width(cap=96)
        open_w = art.term_width(cap=None)
        self.assertGreaterEqual(open_w, capped)


class TestLegacyResume(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="pixie-sess-"))

    def test_legacy_chat_from_session(self):
        p = self.tmp / "old.jsonl"
        p.write_text(
            '{"role": "user", "text": "Hi pixie"}\n'
            '{"role": "assistant", "text": "hello"}\n',
            encoding="utf-8",
        )
        chat, rune = mind.chat_from_session("old", self.tmp)
        self.assertEqual(rune, "chat")
        self.assertEqual(chat[0][1], "Hi pixie")
        self.assertEqual(chat[1][1], "hello")


class TestThinkAndSpecs(unittest.TestCase):
    def test_strip_think(self):
        raw = "<think>\nsecret chain\n</think>\n\nHello there"
        self.assertEqual(mind.strip_think(raw), "Hello there")
        self.assertEqual(mind.strip_think("</think>\nHi"), "Hi")

    def test_invented_specs(self):
        self.assertTrue(mind.looks_like_invented_specs("use a 19mm socket at 25 Nm"))
        self.assertFalse(mind.looks_like_invented_specs("I do not know the bolt size."))

    def test_tool_was_empty(self):
        self.assertTrue(mind.tool_was_empty({"count": 0, "results": []}))
        self.assertTrue(mind.tool_was_empty({"exit_code": 1, "stdout": "", "stderr": ""}))
        self.assertFalse(mind.tool_was_empty({"exit_code": 0, "stdout": "ok"}))

    def test_plan_menu_choices(self):
        acts = [a for _k, _l, a in mind.plan_menu_choices()]
        self.assertEqual(acts, ["approve", "edit", "drop"])


class TestResearchPlan(unittest.TestCase):
    def test_search_house_finds_pixie_plan(self):
        r = mind.search_house("write_plan")
        self.assertGreaterEqual(r.get("count", 0), 0)
        r2 = mind.search_house("Pixie")
        self.assertGreater(r2.get("count", 0), 0)

    def test_write_plan_roundtrip(self):
        tmp_id = "testplan"
        r = mind.write_plan(
            "upgrade pixie",
            steps=["search", "write"],
            files=["bin/pixie"],
            risks=["ctx"],
            verify="tests",
            session_id=tmp_id,
        )
        self.assertTrue(r.get("wrote"))
        body = mind.load_plan(tmp_id)
        self.assertIn("upgrade pixie", body)
        self.assertIn("bin/pixie", body)
        card = mind.plan_summary(tmp_id)
        self.assertIn("Goal", card)
        self.assertTrue(mind.drop_plan(tmp_id))
        self.assertEqual(mind.load_plan(tmp_id), "")

    def test_note_and_list_sources(self):
        sid = "testsrc"
        mind.note_source(sid, "https://example.org/a", "A")
        mind.note_source(sid, "https://example.org/a", "A again")
        srcs = mind.listed_sources(sid)
        self.assertEqual(len(srcs), 1)


if __name__ == "__main__":
    unittest.main()
