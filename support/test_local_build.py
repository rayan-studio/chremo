# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Regression tests for the local build driver (no Rust compilation)."""

import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import local_build


class LocalBuildTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {"PATH": "", "CARGO_TARGET_DIR": str(self.root / "target")}
        for profile in ("debug", "medium"):
            binary = self.root / "target" / profile / "servoshell.exe"
            binary.parent.mkdir(parents=True)
            binary.touch()
        env_patcher = patch.dict(os.environ, {"CARGO_TARGET_DIR": self.env["CARGO_TARGET_DIR"]})
        env_patcher.start()
        self.addCleanup(env_patcher.stop)
        for name, value in (("ROOT", self.root), ("environment", lambda: self.env)):
            patcher = patch.object(local_build, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.object(local_build.shutil, "which", return_value="tool.exe")
        patcher.start()
        self.addCleanup(patcher.stop)
        output = contextlib.redirect_stdout(io.StringIO())
        output.__enter__()
        self.addCleanup(output.__exit__, None, None, None)

    def test_failed_build_does_not_stage_or_launch_old_binary(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime") as stage:
            run.return_value.returncode = 17
            self.assertEqual(local_build.main(["run", "--build"]), 17)
            self.assertEqual(run.call_count, 1)
            stage.assert_not_called()

    def test_check_never_stages_runtime(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime") as stage:
            run.return_value.returncode = 0
            self.assertEqual(local_build.main(["check", "--offline", "-j", "2"]), 0)
            command = run.call_args.args[0]
            self.assertEqual(command[:2], ["cargo", "check"])
            self.assertIn("--locked", command)
            self.assertIn("--offline", command)
            stage.assert_not_called()

    def test_run_preserves_arguments_and_exit_code(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime"):
            run.side_effect = [type("Result", (), {"returncode": 0})(), type("Result", (), {"returncode": 5})()]
            self.assertEqual(local_build.main(["run", "--build", "--profile", "dev", "--", "a b", "--help"]), 5)
            self.assertEqual(run.call_args.args[0], [str(self.root / "target/debug/servoshell.exe"), "a b", "--help"])

    def test_dry_run_does_not_compile_or_stage(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime") as stage:
            self.assertEqual(local_build.main(["--dry-run"]), 0)
            run.assert_not_called()
            stage.assert_not_called()

    def test_powershell_run_tail_keeps_urls_and_browser_options(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime"):
            run.return_value.returncode = 0
            url = "https://example.org/?a=1&b=2"
            self.assertEqual(local_build.main(["run", "--offline", url, "--profile", "browser value"]), 0)
            self.assertEqual(run.call_args.args[0][1:], [url, "--profile", "browser value"])

    def test_powershell_browser_version_without_separator(self) -> None:
        with patch.object(local_build.subprocess, "run") as run, patch.object(local_build, "stage_runtime"):
            run.return_value.returncode = 0
            self.assertEqual(local_build.main(["run", "--version"]), 0)
            self.assertEqual(run.call_args.args[0][1:], ["--version"])

    def test_mach_configuration_is_not_silently_ignored(self) -> None:
        (self.root / ".servobuild").write_text("[build]\n", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "settings"):
            local_build.main([])

    def test_run_never_prepares_build_environment_or_invokes_cargo(self) -> None:
        with (
            patch.object(local_build, "environment", side_effect=AssertionError("Build setup invoked")),
            patch.object(local_build.subprocess, "run") as run,
            patch.object(local_build, "stage_runtime") as stage,
        ):
            run.return_value.returncode = 7
            self.assertEqual(local_build.main(["run", "google.fr\\"]), 7)
            run.assert_called_once_with(
                [str(self.root / "target/medium/servoshell.exe"), "google.fr\\"],
                cwd=self.root,
            )
            stage.assert_not_called()

    def test_run_missing_binary_does_not_start_a_build(self) -> None:
        with patch.object(local_build.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "caddy build --profile fast"):
                local_build.main(["run", "--profile", "fast"])
            run.assert_not_called()

    def test_unchanged_dll_is_not_rewritten_but_changed_content_is(self) -> None:
        source, destination = self.root / "source.dll", self.root / "destination.dll"
        source.write_bytes(b"first")
        local_build.copy_changed(source, destination)
        before = destination.stat().st_mtime_ns
        local_build.copy_changed(source, destination)
        self.assertEqual(before, destination.stat().st_mtime_ns)
        source.write_bytes(b"other")
        local_build.copy_changed(source, destination)
        self.assertEqual(destination.read_bytes(), b"other")


if __name__ == "__main__":
    unittest.main()
