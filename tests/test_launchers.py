"""Exercise Git installs without network access or a user's saved credentials."""

import hashlib
import json
import os
import shutil
import subprocess
import unittest
import uuid
from pathlib import Path


SKILL = Path(__file__).resolve().parents[1]
TEMP_ROOT = Path(os.environ.get("BULBUL_TEST_TMP", str(SKILL / ".tmp" / "launchers"))).resolve()
BINARY = Path(os.environ["BULBUL_TEST_BINARY"]).resolve()
RUNTIME = json.loads((SKILL / "assets/runtime.json").read_text(encoding="utf-8"))
VERSION = RUNTIME["version"]
WINDOWS = os.name == "nt"
if WINDOWS:
    BASH = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
    TARGET = "x86_64-pc-windows-msvc"
    SUFFIX = ".exe"
else:
    BASH = Path(shutil.which("bash") or "/bin/bash")
    import platform

    TARGET = {
        ("Darwin", "arm64"): "aarch64-apple-darwin",
        ("Darwin", "x86_64"): "x86_64-apple-darwin",
        ("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
        ("Linux", "aarch64"): "aarch64-unknown-linux-gnu",
    }[(platform.system(), platform.machine())]
    SUFFIX = ""
FLAVORS = ("bash", "powershell") if WINDOWS else ("bash",)


class Install:
    def __init__(self, flavor, version=VERSION):
        self.flavor = flavor
        self.version = version
        self.root = TEMP_ROOT / f"skill with spaces {uuid.uuid4().hex}"
        for name in ("scripts", "assets", "release", "mock"):
            (self.root / name).mkdir(parents=True)
        for name in ("bulbul.sh", "bulbul.ps1"):
            shutil.copyfile(SKILL / "scripts" / name, self.root / "scripts" / name)
        shutil.copyfile(SKILL / "assets/tools.jsonl", self.root / "assets/tools.jsonl")
        (self.root / "assets/runtime.json").write_text(
            json.dumps({**RUNTIME, "version": version}, indent=2), encoding="utf-8"
        )
        self.asset = f"bulbul-{version}-{TARGET}{SUFFIX}"
        self.download = self.root / "release" / self.asset
        shutil.copyfile(BINARY, self.download)
        self.checksum = self.root / "release" / f"{self.asset}.sha256"
        self.checksum.write_text(
            f"{hashlib.sha256(self.download.read_bytes()).hexdigest()}  {self.asset}\n",
            encoding="utf-8",
        )
        self.log = self.root / "requests.txt"
        self.cache_root = self.root / "runtime-cache"
        self.cache = self.cache_root / version / TARGET / f"bulbul{SUFFIX}"
        self.cwd = self.root
        self.env = {
            key: value for key, value in os.environ.items()
            if not key.startswith("BULBUL_")
        }
        self.env.update({
            "BULBUL_CACHE_DIR": self.cache_root.as_posix(),
            "BULBUL_TEST_RELEASE_DIR": (self.root / "release").as_posix(),
            "BULBUL_TEST_REQUEST_LOG": self.log.as_posix(),
        })
        mock_path = (self.root / "mock").as_posix()
        if WINDOWS:
            mock_path = f"/{mock_path[0].lower()}{mock_path[2:]}"
        self.env["BULBUL_TEST_MOCK_PATH"] = mock_path
        mock = self.root / "mock/curl"
        mock.write_text(
            '#!/usr/bin/env bash\nset -eu\n'
            'printf "%s\\n" "$*" >> "$BULBUL_TEST_REQUEST_LOG"\n'
            'while [[ $# -gt 0 ]]; do\n'
            '  if [[ "$1" == --output ]]; then output="$2"; shift; fi\n'
            '  url="$1"; shift\n'
            'done\n'
            'cp "$BULBUL_TEST_RELEASE_DIR/${url##*/}" "$output"\n',
            encoding="utf-8",
        )
        mock.chmod(0o755)
        self.env["PATH"] = str(self.root / "mock") + os.pathsep + self.env["PATH"]
        self.driver = self.root / "mock-download.ps1"
        self.driver.write_text(
            "Import-Module Microsoft.PowerShell.Utility\n"
            ". (Join-Path $PSScriptRoot 'scripts/bulbul.ps1')\n"
            "function Invoke-WebRequest {\n"
            "  param($Uri, $OutFile, [switch]$UseBasicParsing, $TimeoutSec)\n"
            "  [IO.File]::AppendAllText($env:BULBUL_TEST_REQUEST_LOG, $Uri + [Environment]::NewLine)\n"
            "  $name = ([uri]$Uri).Segments[-1]\n"
            "  Copy-Item -LiteralPath (Join-Path $env:BULBUL_TEST_RELEASE_DIR $name) -Destination $OutFile\n"
            "}\n"
            "exit (Invoke-Bulbul -CliArgs @($args))\n",
            encoding="utf-8-sig",
        )

    def command(self, *args, mock=True):
        if self.flavor == "bash":
            if mock:
                return [str(BASH), "-c", 'export PATH="$BULBUL_TEST_MOCK_PATH:$PATH"; exec bash "$@"',
                        "launcher-test", (self.root / "scripts/bulbul.sh").as_posix(), *args]
            return [str(BASH), (self.root / "scripts/bulbul.sh").as_posix(), *args]
        script = self.driver if mock else self.root / "scripts/bulbul.ps1"
        return ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script), *args]

    def run(self, *args, mock=True):
        return subprocess.run(
            self.command(*args, mock=mock), env=self.env, cwd=self.cwd,
            capture_output=True, text=True, encoding="utf-8", timeout=45,
        )

    def requests(self):
        return self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []


class LauncherTests(unittest.TestCase):
    def test_first_download_then_offline_cache_and_clean_json(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                first = install.run("--version")
                self.assertEqual(first.returncode, 0, first.stderr)
                self.assertEqual(first.stdout.strip(), f"bulbul {VERSION}")
                self.assertIn("preparing version", first.stderr)
                self.assertEqual(len(install.requests()), 2)
                for url in install.requests():
                    self.assertIn(f"/{RUNTIME['repository']}/releases/download/v{VERSION}/", url)
                cached = install.run("--version", mock=False)
                self.assertEqual(cached.returncode, 0, cached.stderr)
                self.assertEqual(cached.stdout.strip(), f"bulbul {VERSION}")
                self.assertEqual(cached.stderr, "")
                self.assertEqual(len(install.requests()), 2)

    def test_schema_discovery_needs_no_binary_network_credentials_or_writable_cache(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                blocked = install.root / "blocked-cache"
                blocked.write_text("not a directory", encoding="utf-8")
                install.env["BULBUL_CACHE_DIR"] = str(blocked)
                install.env["BULBUL_API_URL"] = "private-invalid-url"
                install.env["BULBUL_SESSION_TOKEN"] = "private-token"
                names = ["identify_photo", "species_wiki", "location_search"]
                distributed = [*install.root.joinpath("scripts").iterdir(), *install.root.joinpath("assets").iterdir()]
                before = {str(file): file.read_bytes() for file in distributed}
                result = install.run("list", *names)
                self.assertEqual(result.returncode, 0, result.stderr)
                native = subprocess.run([str(BINARY), "list", *names], env=install.env, capture_output=True, check=True)
                self.assertEqual(json.loads(result.stdout), json.loads(native.stdout))
                all_tools = install.run("list")
                self.assertEqual(all_tools.returncode, 0, all_tools.stderr)
                self.assertEqual(len(json.loads(all_tools.stdout)), 23)
                for unknown, code in [("private-unknown", 2), ("not_a_tool", 1)]:
                    result = install.run("list", "species_wiki", unknown)
                    self.assertEqual(result.returncode, code, result.stderr)
                    self.assertEqual(result.stdout, "")
                    self.assertNotIn(unknown, result.stderr)
                    json.loads(result.stderr)
                self.assertEqual(install.requests(), [])
                self.assertFalse(install.cache_root.exists())
                self.assertEqual(before, {str(file): file.read_bytes() for file in distributed})

    def test_default_cache_uses_git_project_and_preserves_skill_files(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                project = install.root.with_name("project " + uuid.uuid4().hex)
                project.mkdir()
                (project / ".git").write_text("gitdir: test-marker", encoding="utf-8")
                install.cwd = project
                install.env.pop("BULBUL_CACHE_DIR")
                install.log = project / "requests.txt"
                install.env["BULBUL_TEST_REQUEST_LOG"] = install.log.as_posix()
                distributed = [*install.root.joinpath("scripts").iterdir(), *install.root.joinpath("assets").iterdir()]
                before = {str(file): file.read_bytes() for file in distributed}
                if not WINDOWS:
                    install.root.chmod(0o555)
                    self.addCleanup(install.root.chmod, 0o755)
                result = install.run("--version")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue((project / ".tmp/bulbul" / VERSION / TARGET / f"bulbul{SUFFIX}").is_file())
                self.assertFalse((install.root / "bin").exists())
                self.assertEqual(before, {str(file): file.read_bytes() for file in distributed})
                self.assertEqual(install.run("--version", mock=False).returncode, 0)
                self.assertEqual(len(install.requests()), 2)

    def test_cache_error_explains_recovery_before_any_download(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                blocked = install.root / "blocked-cache"
                blocked.write_text("not a directory", encoding="utf-8")
                install.env["BULBUL_CACHE_DIR"] = str(blocked)
                result = install.run("--version")
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn("BULBUL_CACHE_DIR", result.stderr)
                self.assertEqual(install.requests(), [])

    def test_old_verified_cache_is_a_read_only_fallback(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                old = install.root / "bin" / VERSION / TARGET / f"bulbul{SUFFIX}"
                old.parent.mkdir(parents=True)
                shutil.copyfile(BINARY, old)
                old.chmod(0o755)
                shutil.copyfile(install.checksum, old.with_suffix(old.suffix + ".sha256"))
                before = old.stat().st_mtime_ns
                result = install.run("--version")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(install.requests(), [])
                self.assertFalse(install.cache_root.exists())
                self.assertEqual(old.stat().st_mtime_ns, before)

    def test_checksum_mismatch_is_rejected_before_execution(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                install.checksum.write_text(f"{'0' * 64}  {install.asset}\n", encoding="utf-8")
                result = install.run("--version")
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn("checksum verification failed", result.stderr)
                self.assertFalse(install.cache.exists())

    def test_matching_checksum_with_wrong_version_is_rejected(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor, "9.9.9")
                result = install.run("--version")
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn("unexpected version", result.stderr)
                self.assertFalse(install.cache.exists())

    def test_damaged_cache_is_replaced(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                self.assertEqual(install.run("--version").returncode, 0)
                install.cache.write_bytes(b"corrupted download")
                result = install.run("--version")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(install.requests()), 4)
                self.assertEqual(hashlib.sha256(install.cache.read_bytes()).digest(), hashlib.sha256(install.download.read_bytes()).digest())

    def test_arguments_file_with_spaces_and_exit_codes(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                self.assertEqual(install.run("--version").returncode, 0)
                arguments = install.root / "arguments with spaces.json"
                arguments.write_text(json.dumps({"name": "预习清单", "taxon_ids": [1]}), encoding="utf-8")
                result = install.run("call", "species_list_create", "--arguments-file", str(arguments), mock=False)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout)["status"], "confirmation_required")
                self.assertEqual(json.loads(result.stdout)["body"]["name"], "预习清单")
                for command in ("unknown-command", "mcp"):
                    unsupported = install.run(command, mock=False)
                    self.assertEqual(unsupported.returncode, 2)
                    self.assertEqual(unsupported.stdout, "")
                    json.loads(unsupported.stderr)
                arguments.write_text("not json", encoding="utf-8")
                result = install.run("call", "species_list_create", "--arguments-file", str(arguments), mock=False)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                json.loads(result.stderr)



if __name__ == "__main__":
    unittest.main()
