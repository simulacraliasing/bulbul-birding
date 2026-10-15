"""Exercise repository maintenance without a Bulbul service checkout or runtime."""

import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
TASK_TMP = ROOT / ".tmp/maintenance-tests"
TASK_TMP.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location("export_tools", ROOT / "scripts/export_birding_tools.py")
export_tools = importlib.util.module_from_spec(spec)
spec.loader.exec_module(export_tools)


class MaintenanceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=TASK_TMP)
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def run_script(self, script, *args):
        return subprocess.run(
            [sys.executable, str(script), *map(str, args)],
            cwd=self.root, capture_output=True, text=True, encoding="utf-8",
        )

    def test_versions_sync_only_the_application_and_reject_invalid_versions(self):
        for relative in ["VERSION", "assets/runtime.json", "cli/Cargo.toml", "cli/Cargo.lock", "scripts/manage_version.py"]:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        script = self.root / "scripts/manage_version.py"
        self.assertEqual(self.run_script(script, "check").returncode, 0)
        original_lock = tomllib.loads((self.root / "cli/Cargo.lock").read_text())
        (self.root / "VERSION").write_text("1.2.3\n", encoding="utf-8")
        self.assertEqual(self.run_script(script, "check").returncode, 1)
        self.assertEqual(self.run_script(script, "sync").returncode, 0)
        self.assertEqual(self.run_script(script, "check").returncode, 0)
        self.assertEqual(json.loads((self.root / "assets/runtime.json").read_text())["version"], "1.2.3")
        self.assertEqual(tomllib.loads((self.root / "cli/Cargo.toml").read_text())["package"]["version"], "1.2.3")
        updated_lock = tomllib.loads((self.root / "cli/Cargo.lock").read_text())
        for original, updated in zip(original_lock["package"], updated_lock["package"], strict=True):
            if original["name"] == "bulbul-birding":
                self.assertEqual(updated["version"], "1.2.3")
            else:
                self.assertEqual(original, updated)
        self.assertEqual(self.run_script(script, "set", "1.2.4-rc.1").returncode, 0)
        self.assertEqual(self.run_script(script, "check").returncode, 0)
        self.assertEqual(self.run_script(script, "set", "not-a-version").returncode, 2)
        self.assertEqual((self.root / "VERSION").read_text().strip(), "1.2.4-rc.1")

    def test_schema_export_handles_references_and_cli_only_arguments(self):
        contract = {"paths": {}, "components": {"schemas": {"Input": {
            "type": "object", "properties": {"taxon_ids": {"type": "array", "items": {"type": "integer"}}},
            "required": ["taxon_ids"],
        }}}}
        for _, method, path, *_ in export_tools.OPERATIONS:
            contract["paths"].setdefault(f"/api/v1{path}", {})[method.lower()] = {}
        contract["paths"]["/api/v1/me/species-lists/{list_id}/items"]["put"] = {
            "parameters": [{"in": "path", "name": "list_id", "required": True, "schema": {"type": "integer"}}],
            "requestBody": {"content": {"application/json": {"schema": {"$ref": "#/components/schemas/Input"}}}},
        }
        for path in ["/bird-identification/identify", "/bird-identification/audio/identify"]:
            contract["paths"][f"/api/v1{path}"]["post"] = {
                "parameters": [{"in": "query", "name": "cloud_calibration", "schema": {"type": "boolean"}}] if path.endswith("/identify") and "/audio/" not in path else [],
                "requestBody": {"content": {"multipart/form-data": {"schema": {"type": "object", "properties": {
                    "file": {"type": "string", "format": "binary"}, "geographic_calibration": {"type": "string"},
                }, "required": ["file"]}}}},
            }
        source, output = self.root / "contract.json", self.root / "tools.json"
        source.write_text(json.dumps(contract), encoding="utf-8")
        script = ROOT / "scripts/export_birding_tools.py"
        result = self.run_script(script, "--openapi", source, "--output", output)
        self.assertEqual(result.returncode, 0, result.stderr)
        tools = {tool["name"]: tool for tool in json.loads(output.read_text(encoding="utf-8"))}
        self.assertEqual(len(tools), 23)
        self.assertEqual(tools["identify_audio"]["max_file_bytes"], 50 * 1024 * 1024)
        self.assertEqual(tools["identify_audio"]["input_schema"]["properties"]["file"]["type"], "string")
        self.assertNotIn("geographic_calibration", tools["identify_photo"]["input_schema"]["properties"])
        self.assertTrue(tools["identify_photo"]["input_schema"]["properties"]["cloud_calibration"]["const"])
        tool = tools["species_list_replace_items"]
        self.assertEqual(tool["path_keys"], ["list_id"])
        self.assertEqual(tool["body_keys"], ["taxon_ids"])
        self.assertEqual(tool["input_schema"]["required"], ["list_id", "taxon_ids"])
        self.assertEqual(tool["input_schema"]["properties"]["confirm"]["default"], False)
        self.assertEqual(tools["species_predict"]["input_schema"]["properties"]["limit"]["maximum"], 100)
        self.assertEqual(tools["location_search"]["input_schema"]["required"], ["query"])
        self.assertEqual(self.run_script(script, "--openapi", source, "--output", output, "--check").returncode, 0)
        original = output.read_bytes()
        output.write_text("[]\n", encoding="utf-8")
        self.assertEqual(self.run_script(script, "--openapi", source, "--output", output, "--check").returncode, 2)
        output.write_bytes(original)
        contract["paths"].pop("/api/v1/species/catalog")
        source.write_text(json.dumps(contract), encoding="utf-8")
        self.assertEqual(self.run_script(script, "--openapi", source, "--output", output).returncode, 2)
        self.assertEqual(output.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
