"""Install packed npm artifacts and invoke their native bin with no runtime PATH."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tarfile

ROOT = Path(__file__).resolve().parents[1]
manifest = json.loads((ROOT / "package.json").read_text())
version = manifest["version"]
artifacts = [ROOT / "artifacts" / name for name in (
    f"twcanon-{version}.tgz",
    f"twcanon-linux-x64-gnu-{version}.tgz",
)]
checks = []
with tarfile.open(artifacts[1], 'r:gz') as archive:
    original_bytes = archive.extractfile('package/bin/twcanon.exe').read()


def command(args, cwd, expected=0, env=None):
    result = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True, timeout=120)
    if result.returncode != expected:
        raise RuntimeError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
    return result


managers = os.environ.get("TEST_MANAGERS", "npm").split(",")
for manager, scripts in [(manager, scripts) for manager in managers
                         for scripts in ((True, False) if manager == "npm" else (False,))]:
    print(f"Installing with {manager}, postinstall={scripts}", flush=True)
    with tempfile.TemporaryDirectory(prefix="tw-npm-") as directory:
        root = Path(directory)
        native_name = "twcanon-linux-x64-gnu"
        native_spec = "file:" + str(artifacts[1])
        (root / "package.json").write_text(json.dumps({
            "name": "installation-probe", "private": True,
            "dependencies": {manifest["name"]: "file:" + str(artifacts[0]),
                             **({native_name: native_spec} if manager != "pnpm" else {})},
        }))
        if manager == "pnpm":
            # Keep the unpublished optional package inside pnpm's virtual store.
            (root / "pnpm-workspace.yaml").write_text(
                "overrides:\n  " + json.dumps(native_name) + ": " + json.dumps(native_spec) + "\n")
        args = [manager, "install", "--offline"]
        if manager == "npm":
            args += ["--no-audit", "--no-fund", "--no-package-lock"]
        if not scripts:
            args.append("--ignore-scripts")
        environment = dict(os.environ)
        cache = root / "bun-cache"
        if manager == "bun":
            environment["BUN_INSTALL_CACHE_DIR"] = str(cache)
        command(args, root, env=environment)
        if manager == "pnpm":
            assert not (root / "node_modules" / native_name).exists(), "pnpm probe must keep the optional package isolated"
        installed = root / "node_modules/twcanon/bin/twcanon.exe"
        cached_links = []
        placeholder = (ROOT / "bin/twcanon.exe").read_bytes()
        if manager == "bun":
            # Bun's mutable links/ installation can be the actual package root.
            # Check distinct cache directory entries sharing its original inode.
            cached_links = [file for file in cache.rglob("twcanon.exe")
                            if file.samefile(installed) and file.resolve() != installed.resolve()]
            assert cached_links, "Bun probe must exercise a placeholder hardlinked to its cache"
            assert installed.read_bytes() == placeholder
        if not scripts:
            command(["node", "node_modules/twcanon/install.mjs"], root, env=environment)
        for cached in cached_links:
            assert cached.read_bytes() == placeholder, "Installer changed the Bun package cache"
            assert not cached.samefile(installed), "Installed binary still shares the cache inode"
        if manager == "bun":
            consumer = root / "second-project"
            consumer.mkdir()
            (consumer / "package.json").write_text((root / "package.json").read_text())
            command(args + ["--linker", "hoisted"], consumer, env=environment)
            second = consumer / "node_modules/twcanon/bin/twcanon.exe"
            assert second.read_bytes() == placeholder, "New project received a modified cache entry"
            command(["node", "node_modules/twcanon/install.mjs"], consumer, env=environment)
            assert second.read_bytes() == installed.read_bytes()
            assert not second.samefile(installed), "Projects must have independent native executables"
            for cached in cached_links:
                assert cached.read_bytes() == placeholder, "Second install changed the Bun cache"
        assert installed.read_bytes() == original_bytes, f"Installed executable differs: {manager}, postinstall={scripts}"
        environment = dict(os.environ, PATH="/path-without-node", CI="1")
        binary = str(root / "node_modules/.bin/twcanon")
        assert command([binary, "--version"], root, env=environment).stdout.strip() == version
        (root / "src").mkdir()
        (root / "src/index.css").write_text("@tailwind utilities;")
        source = root / "src/a.tsx"
        source.write_text('cn("flex-grow");\n')
        result = command([binary, "--check"], root, expected=1, env=environment)
        assert "flex-grow" in result.stdout and "grow" in result.stdout
        command([binary], root, env=environment)
        assert source.read_text() == 'cn("grow");\n'
        command([binary, "--check"], root, env=environment)
        checks.append({"manager": manager, "postinstall": scripts, "manual_install": not scripts,
                       "native_without_node": True, "check_fix_check": True,
                       **({"isolated_optional_dependency": True} if manager == "pnpm" else {}),
                       **({"isolated_cache": True, "cache_hardlinks_unchanged": len(cached_links),
                           "second_project_cache_reuse": True, "second_project_linker": "hoisted",
                           "independent_executables": True}
                          if manager == "bun" else {})})

report = {
    "platform": "linux-x64-gnu",
    "manager_versions": {manager: command([manager, "--version"], ROOT).stdout.strip() for manager in managers},
    "node_version": command(["node", "--version"], ROOT).stdout.strip(),
    "hashes": {str(file.relative_to(ROOT)): hashlib.sha256(file.read_bytes()).hexdigest()
               for file in artifacts},
    "checks": checks,
}
(ROOT / "build").mkdir(exist_ok=True)
(ROOT / "build/npm-installation.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
