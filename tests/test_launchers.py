"""Exercise Git installs without network access or a user's saved credentials."""

import hashlib
import json
import os
import queue
import shutil
import subprocess
import threading
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
        self.cache = self.root / "bin" / version / TARGET / f"bulbul{SUFFIX}"
        self.env = {
            key: value for key, value in os.environ.items()
            if not key.startswith("BULBUL_")
        }
        self.env.update({
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
            "try { $binary = Get-BulbulBinary } catch {\n"
            "  [Console]::Error.WriteLine($_.Exception.Message); exit 1\n"
            "}\n"
            "& $binary @args\nexit $LASTEXITCODE\n",
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
            self.command(*args, mock=mock), env=self.env, cwd=self.root,
            capture_output=True, text=True, encoding="utf-8", timeout=45,
        )

    def requests(self):
        return self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []


class LauncherTests(unittest.TestCase):
    def test_first_download_then_offline_cache_and_clean_json(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                first = install.run("list", "species_catalog")
                self.assertEqual(first.returncode, 0, first.stderr)
                self.assertIn("species_catalog", first.stdout)
                json.loads(first.stdout)
                self.assertIn("preparing version", first.stderr)
                self.assertEqual(len(install.requests()), 2)
                for url in install.requests():
                    self.assertIn(f"/{RUNTIME['repository']}/releases/download/v{VERSION}/", url)
                cached = install.run("--version", mock=False)
                self.assertEqual(cached.returncode, 0, cached.stderr)
                self.assertEqual(cached.stdout.strip(), f"bulbul {VERSION}")
                self.assertEqual(cached.stderr, "")
                self.assertEqual(len(install.requests()), 2)

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
                self.assertEqual(hashlib.sha256(install.cache.read_bytes()).digest(), hashlib.sha256(BINARY.read_bytes()).digest())

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
                self.assertEqual(install.run("unknown-command", mock=False).returncode, 2)
                arguments.write_text("not json", encoding="utf-8")
                result = install.run("call", "species_list_create", "--arguments-file", str(arguments), mock=False)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                json.loads(result.stderr)

    def test_real_wrapper_stdio_mcp(self):
        for flavor in FLAVORS:
            with self.subTest(flavor=flavor):
                install = Install(flavor)
                self.assertEqual(install.run("--version").returncode, 0)
                process = subprocess.Popen(
                    install.command("mcp", mock=False), cwd=install.root, env=install.env,
                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    text=True, encoding="utf-8", bufsize=1,
                )
                output = queue.Queue()
                def read_lines():
                    for line in process.stdout:
                        output.put(line)
                reader = threading.Thread(target=read_lines, daemon=True)
                reader.start()
                def send(value):
                    process.stdin.write(json.dumps(value) + "\n")
                    process.stdin.flush()
                try:
                    send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                        "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": {"name": "launcher-test", "version": "1"},
                    }})
                    response = json.loads(output.get(timeout=15))
                    self.assertEqual(response["result"]["serverInfo"]["name"], "bulbul-birding")
                    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
                    send({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
                    response = json.loads(output.get(timeout=15))
                    self.assertEqual(len(response["result"]["tools"]), 25)
                finally:
                    process.stdin.close()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                    stderr = process.stderr.read()
                    process.stdout.close()
                    process.stderr.close()
                self.assertEqual(process.returncode, 0, stderr)
                self.assertNotIn("preparing version", stderr)


if __name__ == "__main__":
    unittest.main()
