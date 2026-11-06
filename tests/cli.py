"""Standalone CLI regression checks; optional differential oracle via CLI_ORACLE."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("TWCANON_BINARY", HERE / "native/target/release/twcanon"))
ORACLE = Path(os.environ["CLI_ORACLE"]) if "CLI_ORACLE" in os.environ else None
JS = ["fnm", "exec", "--using", "24.21.0", "node", "--regexp-interpret-all", str(ORACLE / "scripts/tailwind-canonical.mjs") if ORACLE else ""]
SOURCE = '\r\n'.join([
    'const emoji = "😀"; const a = <b className="flex-grow" />;',
    'const b = <b class={"flex-shrink-0"} />;',
    'const c = <b className={`flex-grow`} />;',
    'cn("flex-grow", `flex-shrink-0`, "flex-\\x67row");',
    'twMerge?.("flex-grow"); mergeClassName("flex-grow");',
    'cn(("flex-grow"), `flex-grow ${emoji}`, { "flex-grow": true });',
    'other("flex-grow"); obj.cn("flex-grow");',
    'const d = <b className="flex&#45;grow" />;',
    'const e = <b className={"flex-\\u0067row"} />;',
])


class Cli(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tw-cli-check-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "src").mkdir()
        (self.root / "node_modules").mkdir()
        self.put("node_modules/tailwindcss/package.json", '{"style":"utilities.css"}')
        self.put("node_modules/tailwindcss/utilities.css", "@tailwind utilities;")

    def put(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode())
        return path

    def run_cli(self, *args, js=False, ci=False):
        environment = dict(os.environ)
        environment.pop("CI", None)
        if ci:
            environment["CI"] = "1"
        result = subprocess.run((JS if js else [str(BINARY)]) + list(args), cwd=self.root,
                                env=environment, capture_output=True, text=True, timeout=90)
        return result

    def test_fixes_escapes_unicode_and_line_numbers(self):
        self.put("src/index.css", '@import "tailwindcss/utilities.css";')
        file = self.put("src/example.tsx", SOURCE)
        native = self.run_cli("--check")
        self.assertEqual(native.returncode, 1, native.stderr)
        self.assertIn("Encontradas 9 ocorrência(s)", native.stdout)
        self.assertEqual(file.read_bytes(), SOURCE.encode())
        native = self.run_cli()
        self.assertEqual(native.returncode, 0, native.stderr)
        fixed = file.read_bytes()
        expected = '\r\n'.join([
            'const emoji = "😀"; const a = <b className="grow" />;',
            'const b = <b class={"shrink-0"} />;',
            'const c = <b className={`grow`} />;',
            'cn("grow", `shrink-0`, "grow");',
            'twMerge?.("grow"); mergeClassName("grow");',
            'cn(("flex-grow"), `flex-grow ${emoji}`, { "flex-grow": true });',
            'other("flex-grow"); obj.cn("flex-grow");',
            'const d = <b className="flex&#45;grow" />;',
            'const e = <b className={"grow"} />;',
        ])
        self.assertEqual(fixed, expected.encode())
        file.write_bytes(SOURCE.encode())
        if ORACLE:
            js = self.run_cli("--serial", js=True)
            self.assertEqual((native.returncode, native.stdout), (js.returncode, js.stdout), js.stderr)
        else:
            self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(file.read_bytes(), fixed)
        self.assertEqual(self.run_cli("--check").returncode, 0)
        self.assertIn(b"\r\n", fixed)

    def test_syntax_error_preserves_whole_batch_and_cache(self):
        self.put("src/index.css", "@tailwind utilities;")
        first = self.put("src/a.tsx", 'cn("flex-grow");')
        broken = self.put("src/z.tsx", 'cn("grow"); const broken = (')
        result = self.run_cli()
        self.assertEqual(result.returncode, 2)
        self.assertIn("Sintaxe inválida em src/z.tsx:", result.stderr)
        self.assertEqual(first.read_text(), 'cn("flex-grow");')
        self.assertFalse((self.root / ".cache").exists())
        broken.write_text('cn("grow");')
        self.assertEqual(self.run_cli().returncode, 0)
        cache = self.root / ".cache/twcanon.json"
        previous = cache.read_bytes()
        broken.write_text('cn("grow"); const broken = (')
        self.assertEqual(self.run_cli().returncode, 2)
        self.assertEqual(cache.read_bytes(), previous)

    def test_content_hash_invalidates_unchanged_source_and_mtime(self):
        self.put("src/index.css", '@import "./theme.css"; @tailwind utilities;')
        theme = self.put("src/theme.css", "@theme { --color-brand: #654321; }")
        file = self.put("src/example.tsx", 'cn("bg-[#123456]");')
        self.assertEqual(self.run_cli("--check").returncode, 0)
        self.assertEqual(self.run_cli("--check").returncode, 0)
        stat = theme.stat()
        theme.write_text("@theme { --color-brand: #123456; }")
        os.utime(theme, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        result = self.run_cli("--check")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("bg-[#123456] -> bg-brand", result.stdout)
        self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(file.read_text(), 'cn("bg-brand");')
        theme.unlink()
        self.assertEqual(self.run_cli("--check").returncode, 2)

    def test_filters_targets_duplicates_and_symlinks(self):
        self.put("src/index.css", "@tailwind utilities;")
        for name in ["first.tsx", "second.tsx", "outside.tsx", "skip.d.ts", "skip.gen.ts", "lib/tailwind-merge.ts", "types/icon.ts"]:
            self.put("src/" + name, 'cn("flex-grow");')
        outside = self.put("external.tsx", 'cn("flex-grow");')
        (self.root / "src/link.tsx").symlink_to(outside)
        result = self.run_cli("--check", "src/first.tsx", "src/second.tsx", "src/first.tsx")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("2 ocorrência(s) em 2 arquivo(s)", result.stdout)
        self.assertNotIn("outside", result.stdout)
        result = self.run_cli("--check")
        self.assertIn("3 ocorrência(s) em 3 arquivo(s)", result.stdout)
        self.assertNotIn("skip", result.stdout)
        self.assertNotIn("link.tsx", result.stdout)
        self.assertEqual(self.run_cli("--check", "src/missing.ts").returncode, 0)

    @unittest.skipUnless(os.name == "posix", "requires directory symlinks")
    def test_absolute_targets_accept_root_alias_but_skip_source_links(self):
        self.put("src/index.css", "@tailwind utilities;")
        file = self.put("src/example.tsx", 'cn("flex-grow");')
        nested = self.put("src/nested/example.tsx", 'cn("flex-grow");')
        with tempfile.TemporaryDirectory(prefix="tw-cli-root-alias-") as directory:
            alias = Path(directory) / "project"
            alias.symlink_to(self.root, target_is_directory=True)
            outside_directory = Path(directory) / "outside"
            outside_directory.mkdir()
            outside = outside_directory / "example.tsx"
            outside.write_text('cn("flex-grow");')
            (self.root / "src/link.tsx").symlink_to(outside)
            (self.root / "src/linked").symlink_to(outside_directory, target_is_directory=True)
            (self.root / "src/rootlink").symlink_to(self.root, target_is_directory=True)
            for name in ["src/rootlink/src/example.tsx", "src/rootlink/src"]:
                for args in [("--check", str(alias / name)), (str(alias / name),)]:
                    result = self.run_cli(*args, ci=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(file.read_text(), 'cn("flex-grow");')
                    self.assertEqual(nested.read_text(), 'cn("flex-grow");')
            target = str(alias / "src/example.tsx")
            result = self.run_cli("--check", target, ci=True)
            self.assertEqual(result.returncode, 1, result.stderr)
            result = self.run_cli(target, ci=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(file.read_text(), 'cn("grow");')
            self.assertEqual(nested.read_text(), 'cn("flex-grow");')
            result = self.run_cli(str(alias / "src/nested"), ci=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(nested.read_text(), 'cn("grow");')
            for name in ["src/link.tsx", "src/linked/example.tsx"]:
                result = self.run_cli(str(alias / name), ci=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(outside.read_text(), 'cn("flex-grow");')

    def test_plain_batch_and_invalid_flags_do_not_require_css(self):
        self.put("src/plain.ts", "export const value = 1;")
        self.put("src/lib/tailwind-merge.ts", 'cn("flex-grow");')
        self.assertEqual(self.run_cli("--check").returncode, 0)
        self.assertFalse((self.root / ".cache").exists())
        for flag in ["--fix", "--chek"]:
            result = self.run_cli(flag)
            self.assertEqual(result.returncode, 2)
            self.assertIn("Argumento desconhecido", result.stderr)

    def test_jsx_quotes_backslashes_and_bom(self):
        self.put("src/index.css", "@tailwind utilities;")
        source = '\ufeffconst emoji = "😀";\r\n' + r'''const view = <b className='content-["hello"] flex-grow'/>; const slash = <b className="content-['a\b'] flex-grow"/>;'''
        file = self.put("src/example.tsx", source)
        native = self.run_cli()
        self.assertEqual(native.returncode, 0, native.stderr)
        fixed = file.read_bytes()
        file.write_bytes(source.encode())
        if ORACLE:
            js = self.run_cli("--serial", js=True)
            self.assertEqual((native.returncode, native.stdout), (js.returncode, js.stdout), js.stderr)
        else:
            self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(fixed, file.read_bytes())
        self.assertTrue(fixed.startswith(b"\xef\xbb\xbf"))

    def test_broken_cache_is_optional_and_ci_disables_cache(self):
        self.put("src/index.css", "@tailwind utilities;")
        file = self.put("src/example.tsx", 'cn("flex-grow");')
        cache = self.put(".cache/twcanon.json", "{")
        self.assertEqual(self.run_cli("--check").returncode, 1)
        self.assertEqual(json.loads(cache.read_text())["version"], 1)
        cache.unlink()
        cache.mkdir()
        self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(file.read_text(), 'cn("grow");')
        shutil.rmtree(self.root / ".cache")
        self.assertEqual(self.run_cli("--check", ci=True).returncode, 0)
        self.assertFalse((self.root / ".cache").exists())

    def test_package_entry_and_symlink_retarget_invalidate_cache(self):
        shutil.rmtree(self.root / "node_modules")
        package = self.root / "node_modules/theme-package"
        self.put("node_modules/theme-package/package.json", '{"style":"a.css"}')
        self.put("node_modules/theme-package/a.css", "@theme { --color-brand: #654321; }")
        self.put("node_modules/theme-package/b.css", "@theme { --color-brand: #123456; }")
        self.put("src/index.css", '@import "theme-package"; @tailwind utilities;')
        self.put("src/example.tsx", 'cn("bg-[#123456]");')
        self.assertEqual(self.run_cli("--check").returncode, 0)
        (package / "package.json").write_text('{"style":"b.css"}')
        self.assertEqual(self.run_cli("--check").returncode, 1)
        (package / "package.json").write_text('{"style":"a.css"}')
        self.assertEqual(self.run_cli("--check").returncode, 0)
        # An unchanged manifest and stylesheet can still resolve elsewhere.
        first = self.root / "first-package"
        package.rename(first)
        package.symlink_to(first, target_is_directory=True)
        self.assertEqual(self.run_cli("--check").returncode, 0)
        self.put("second-package/package.json", '{"style":"a.css"}')
        second = self.put("second-package/a.css", "@theme { --color-brand: #123456; }").parent
        package.unlink()
        package.symlink_to(second, target_is_directory=True)
        self.assertEqual(self.run_cli("--check").returncode, 1)

    def test_readonly_source_stops_batch_before_writing(self):
        self.put("src/index.css", "@tailwind utilities;")
        first = self.put("src/a.tsx", 'cn("flex-grow");')
        readonly = self.put("src/z.tsx", 'cn("flex-grow");')
        readonly.chmod(0o444)
        self.addCleanup(readonly.chmod, 0o644)
        result = self.run_cli()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("Arquivo somente leitura:", result.stderr)
        self.assertIn("z.tsx", result.stderr)
        self.assertEqual(first.read_text(), 'cn("flex-grow");')
        self.assertEqual(readonly.read_text(), 'cn("flex-grow");')
        self.assertFalse((self.root / ".cache").exists())

    def test_surrogate_error_identifies_source_and_line(self):
        self.put("src/index.css", "@tailwind utilities;")
        first = self.put("src/a.tsx", 'cn("flex-grow");')
        self.put("src/z.tsx", 'const a="😀";\r\nconst b=1;\u2028cn("flex-grow \\uD800");')
        result = self.run_cli()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("src/z.tsx:L3:", result.stderr)
        self.assertIn("surrogate UTF-16 isolado", result.stderr)
        self.assertEqual(first.read_text(), 'cn("flex-grow");')

    @unittest.skipIf(os.name != "posix" or os.geteuid() == 0, "requires Unix permissions")
    def test_partial_io_failure_reports_written_files(self):
        self.put("src/index.css", "@tailwind utilities;")
        first = self.put("src/a.tsx", 'cn("flex-grow");')
        blocked = self.put("src/b/z.tsx", 'cn("flex-grow");')
        blocked.parent.chmod(0o555)
        self.addCleanup(blocked.parent.chmod, 0o755)
        result = self.run_cli()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("Arquivos já alterados neste lote:\n  src/a.tsx", result.stderr)
        self.assertEqual(first.read_text(), 'cn("grow");')
        self.assertEqual(blocked.read_text(), 'cn("flex-grow");')
        self.assertFalse((self.root / ".cache").exists())

    @unittest.skipUnless(os.name == "posix", "requires a directory symlink")
    def test_symlink_source_root_has_explicit_fix_error(self):
        self.put("src/index.css", "@tailwind utilities;")
        file = self.put("src/a.tsx", 'cn("flex-grow");')
        directory = self.root / "actual"
        (self.root / "src").rename(directory)
        (self.root / "src").symlink_to(directory, target_is_directory=True)
        self.assertEqual(self.run_cli("--check").returncode, 1)
        result = self.run_cli()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("caminho com symlink ou não canônico", result.stderr)
        self.assertNotIn("mudou durante", result.stderr)
        self.assertEqual(file.read_text(), 'cn("flex-grow");')

    def test_prefixed_theme_arbitrary_values(self):
        self.put("src/index.css", '@theme prefix(tw) { --spacing: .25rem; --color-white: #fff; } @tailwind utilities;')
        source = 'cn("tw:p-[1rem] tw:bg-[#fff] tw:w-[16px]");'
        file = self.put("src/example.tsx", source)
        native = self.run_cli()
        fixed = file.read_bytes()
        self.assertEqual(native.returncode, 0, native.stderr)
        self.assertEqual(fixed, b'cn("tw:w-4 tw:bg-white tw:p-4");')
        file.write_text(source)
        if ORACLE:
            js = self.run_cli("--serial", js=True)
            self.assertEqual((native.returncode, native.stdout), (js.returncode, js.stdout), js.stderr)
        else:
            self.assertEqual(self.run_cli().returncode, 0)
        self.assertEqual(fixed, file.read_bytes())


if __name__ == "__main__":
    unittest.main(verbosity=2)
