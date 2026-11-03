"""Regression coverage for the public workspace reset command (no live harness)."""

import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BINDINGS = {
    "native_session_id": "deleted-session",
    "native_session_harness": "pi",
    "native_session_machine": "machine",
    "native_session_fingerprint": "old-fingerprint",
}


class ResetContextTest(unittest.TestCase):
    def test_reset_clears_runtime_and_machine_bindings_but_preserves_configuration(self):
        schema_source = (ROOT / "crates/storage/src/schema.rs").read_text()
        schema = schema_source.split('const BASELINE_SCHEMA: &str = r#"', 1)[1].split('"#;', 1)[0]
        with tempfile.TemporaryDirectory(prefix="meerkat-reset-test-") as directory:
            home = Path(directory)
            database = home / "runtime.db"
            connection = sqlite3.connect(database)
            connection.executescript(schema)
            connection.executescript(schema_source.split('const OBSERVATION_SCHEMA: &str = r#"',1)[1].split('"#;',1)[0])
            connection.execute("PRAGMA user_version=2")
            connection.execute(
                "INSERT INTO machines(machine_id,display_name,credential_hash,paired_at,os,version) "
                "VALUES('machine','Test','test-only',0,'test','test')"
            )
            session = {**BINDINGS, "logical_session_id": "logical", "custom": {"keep": True}}
            for workspace in ("target", "other"):
                (home / "workspaces" / workspace / "agents" / "agent").mkdir(parents=True)
                connection.execute(
                    "INSERT INTO workspaces(workspace_id,name,provider_id,provider_config_json,desired_state,created_at,updated_at) "
                    "VALUES(?,?,'test','{}','stopped',0,0)", (workspace, workspace)
                )
                connection.execute(
                    "INSERT INTO agents(workspace_id,agent_id,name,created_at,updated_at,machine_id,harness_id,model,thinking,env_json,launch_arguments_json,session_config_json) "
                    "VALUES(?,'agent','Agent',0,0,'machine','pi','test-model','medium',?,?,?)",
                    (workspace, json.dumps({"PATH": "/test/bin"}), '["--test"]', json.dumps(session)),
                )
                connection.execute(
                    "UPDATE agents SET pause_reason='missing session',failure_fingerprint='failure',failure_streak=3 WHERE workspace_id=?",
                    (workspace,),
                )
            connection.commit()
            avatar = {"resource_id": 42, "file_name": "member.jpg", "media_type": "image/jpeg", "size_bytes": 6, "sha256": "test"}
            connection.execute(
                "INSERT INTO resource_metadata VALUES('target',42,'member.jpg','image/jpeg',6,'test','42',0)"
            )
            connection.execute(
                "INSERT INTO people(workspace_id,person_id,team_role,provider_instance_id,external_id,avatar_json,created_at,updated_at) "
                "VALUES('target','human','lead','test','human',?,0,0)", (json.dumps(avatar),)
            )
            connection.commit()
            avatar_file = home / "resources/target/42"
            avatar_file.parent.mkdir(parents=True)
            avatar_file.write_bytes(b"avatar")
            before = connection.execute("SELECT * FROM agents ORDER BY workspace_id").fetchall()
            columns = [item[1] for item in connection.execute("PRAGMA table_info(agents)")]
            session_index = columns.index("session_config_json")
            connection.close()
            memory_files = {
                "MEMORY.md": "- Project details: topics/project.md\n",
                "topics/project.md": "Old project facts.\n",
                ".private-note.md": "Old hidden note.\n",
            }
            for workspace in ("target", "other"):
                agent_home = home / "workspaces" / workspace / "agents/agent"
                (agent_home / "AGENTS.md").write_text("Configured personality.\n")
                for relative, content in memory_files.items():
                    path = agent_home / "memory" / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(content)
            native_file = home / "workspaces/target/agents/agent/.meerkat/pi/session.jsonl"
            native_file.parent.mkdir(parents=True)
            native_file.write_text("old native session")
            state_path = home / "workspaces/.machine/state.json"
            state_path.parent.mkdir()
            state_path.write_text(json.dumps({
                "sessions": {"target/agent/logical": {"id": "old"}, "other/agent/logical": {"id": "keep"}},
                "accepted_assignments": {}, "pending_events": {},
            }))
            result = subprocess.run([
                str(ROOT / "scripts/reset-agent-context.sh"), "--home", str(home),
                "--workspace", "target", "--no-restart", "--yes",
            ], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            connection = sqlite3.connect(database)
            after = connection.execute("SELECT * FROM agents ORDER BY workspace_id").fetchall()
            self.assertEqual(before[0], after[0], "other workspace must remain unchanged")
            expected = list(before[1])
            expected[session_index] = json.dumps({"logical_session_id": "logical", "custom": {"keep": True}}, separators=(",", ":"))
            for field, value in (("pause_reason", None), ("failure_fingerprint", None), ("failure_streak", 0)):
                expected[columns.index(field)] = value
            self.assertEqual(tuple(expected), after[1], "only bindings and execution failures may change")
            self.assertEqual(json.loads(state_path.read_text())["sessions"], {"other/agent/logical": {"id": "keep"}})
            self.assertFalse(native_file.exists())
            self.assertEqual(avatar_file.read_bytes(), b"avatar")
            self.assertEqual(connection.execute("SELECT count(*) FROM resource_metadata WHERE workspace_id='target' AND resource_id=42").fetchone()[0], 1)
            backups = list((home / "backups/zero-context").iterdir())
            self.assertEqual(len(backups), 1)
            self.assertTrue((backups[0] / "agents/agent/.meerkat/pi/session.jsonl").exists())
            for relative, content in memory_files.items():
                self.assertFalse((home / "workspaces/target/agents/agent/memory" / relative).exists())
                self.assertEqual((backups[0] / "agents/agent/memory" / relative).read_text(), content)
                self.assertEqual((home / "workspaces/other/agents/agent/memory" / relative).read_text(), content)
            for workspace in ("target", "other"):
                self.assertEqual(
                    (home / "workspaces" / workspace / "agents/agent/AGENTS.md").read_text(),
                    "Configured personality.\n",
                )
            connection.close()
            # Reject a redirected memory directory before moving any history.
            memory = home / "workspaces/target/agents/agent/memory"
            memory.rmdir()
            memory.symlink_to(home / "workspaces/other/agents/agent/memory", target_is_directory=True)
            rejected = subprocess.run([
                str(ROOT / "scripts/reset-agent-context.sh"), "--home", str(home),
                "--workspace", "target", "--no-restart", "--yes",
            ], capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("symbolic link", rejected.stderr)
            self.assertTrue(memory.is_symlink())
            self.assertEqual((memory / "MEMORY.md").read_text(), memory_files["MEMORY.md"])


if __name__ == "__main__":
    unittest.main()
