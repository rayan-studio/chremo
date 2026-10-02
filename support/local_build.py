# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Caddy: direct Cargo workflow for this fork on Windows x64."""

import argparse
import filecmp
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def prepend(env: dict[str, str], *paths: Path) -> None:
    existing = [str(p) for p in paths if p.is_dir()]
    env["PATH"] = os.pathsep.join(existing + [env.get("PATH", "")])


def environment() -> dict[str, str]:
    if sys.platform != "win32" or platform.machine().lower() not in ("amd64", "x86_64"):
        raise RuntimeError("This local builder supports native Windows x64 only.")
    env = os.environ.copy()
    if env.get("CARGO_BUILD_TARGET"):
        raise RuntimeError("Unset CARGO_BUILD_TARGET: cross compilation is not supported.")
    # Reuse the repository's standard-library-only VS discovery, without importing mach.
    sys.path.insert(0, str(ROOT / "python"))
    from servo.visual_studio import find_msvc_installations

    if not env.get("INCLUDE") or not env.get("LIB") or env.get("VSCMD_ARG_TGT_ARCH", "x64") != "x64":
        try:
            installation = find_msvc_installations()[0]
        except Exception as error:
            raise RuntimeError(f"Visual Studio C++ tools not found: {error}") from error
        vcvars = Path(installation.vc_install_path) / "Auxiliary/Build/vcvars64.bat"
        command = f'cmd.exe /d /s /c "call "{vcvars}" >nul && set"'
        result = subprocess.run(
            command,
            capture_output=True,
            text=True,
            errors="replace",
            check=True,
        )
        for line in result.stdout.splitlines():
            key, sep, value = line.partition("=")
            if sep and key:
                env[key.upper()] = value
    program_files = Path(env.get("PROGRAMFILES", r"C:\Program Files"))
    llvm = program_files / "LLVM/bin"
    prepend(
        env,
        Path(sys.executable).parent,
        Path.home() / ".cargo/bin",
        llvm,
        program_files / "CMake/bin",
        Path.home() / "AppData/Local/bin/NASM",
    )
    env.setdefault("LIBCLANG_PATH", str(llvm))
    env.setdefault("CC", "clang-cl.exe")
    env.setdefault("CXX", "clang-cl.exe")
    env.setdefault("PYTHON3", "python")
    if env.get("VCTOOLSINSTALLDIR"):
        env.setdefault("VC_PATH", env["VCTOOLSINSTALLDIR"].rstrip("\\/"))
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))
    env["CARGO_TARGET_DIR"] = str((ROOT / env["CARGO_TARGET_DIR"]).resolve())
    env["RUSTFLAGS"] = " ".join(env.get("RUSTFLAGS", "").split())
    env.setdefault("SERVO_STYLE_THREAD_STACK_SIZE_KB", "8192")
    if shutil.which("sccache", path=env["PATH"]):
        env.setdefault("RUSTC_WRAPPER", "sccache")
        env.setdefault("CCACHE", "sccache")
    return env


def cargo_command(args: argparse.Namespace) -> list[str]:
    command = [
        "cargo",
        "check" if args.action == "check" else "rustc",
        "--manifest-path",
        str(ROOT / "ports/servoshell/Cargo.toml"),
        "--locked",
        "--profile",
        args.profile,
        "--timings",
    ]
    if args.jobs:
        command += ["--jobs", str(args.jobs)]
    if args.offline:
        command.append("--offline")
    if args.verbose:
        command.append("-v")
    return command


def copy_changed(source: Path, destination: Path) -> None:
    if not destination.exists() or not filecmp.cmp(source, destination, shallow=False):
        shutil.copy2(source, destination)
        print(f"  DLL: {source.name}", flush=True)


def stage_runtime(binary_dir: Path) -> None:
    # Prefer the most recently built ANGLE runtime within this Cargo profile.
    for name in ("libEGL.dll", "libGLESv2.dll"):
        candidates = list((binary_dir / "build").rglob(name))
        if not candidates:
            raise RuntimeError(f"Missing built runtime library: {name}")
        copy_changed(max(candidates, key=lambda p: p.stat().st_mtime_ns), binary_dir / name)
    from servo.visual_studio import find_msvc_redist_dirs

    redist = next(Path(p) for p in find_msvc_redist_dirs("x64") if (Path(p) / "msvcp140.dll").is_file())
    for source in redist.glob("*.dll"):
        copy_changed(source, binary_dir / source.name)


def positive_int(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("jobs must be at least 1")
    return number


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    run_args = []
    # PowerShell consumes an unquoted -- when calling a .ps1 script. Also
    # support `caddy run URL [browser options]`, preserving the entire tail.
    if argv and argv[0] == "run" and "--" not in argv:
        index = 1
        while index < len(argv):
            option = argv[index]
            if option in ("--profile", "-j", "--jobs"):
                index += 2
            elif option in ("--offline", "-v", "--verbose", "--dry-run", "--build"):
                index += 1
            elif option.startswith(("--profile=", "--jobs=", "-j")):
                index += 1
            else:
                argv.insert(index, "--")
                break
    if "--" in argv:
        separator = argv.index("--")
        run_args, argv = argv[separator + 1 :], argv[:separator]
    parser = argparse.ArgumentParser(prog="caddy", description=__doc__)
    parser.add_argument("action", nargs="?", default="build", choices=("build", "check", "run", "doctor"))
    parser.add_argument("--profile", default="medium", choices=("dev", "medium", "fast", "release", "production"))
    parser.add_argument("-j", "--jobs", type=positive_int)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("-v", "--verbose", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--build", action="store_true", help="Compile before run (run otherwise starts the existing binary)"
    )
    args = parser.parse_args(argv)
    if run_args and args.action != "run":
        parser.error("Arguments after -- are only accepted by run")
    if args.build and args.action != "run":
        parser.error("--build is only accepted by run")
    if args.action == "run" and not args.build:
        # Starting the browser must not invoke Cargo, VS discovery, or DLL scans.
        target_dir = (ROOT / os.environ.get("CARGO_TARGET_DIR", "target")).resolve()
        binary = target_dir / ("debug" if args.profile == "dev" else args.profile) / "servoshell.exe"
        command = [str(binary), *run_args]
        if args.dry_run:
            print(subprocess.list2cmdline(command), flush=True)
            return 0
        if not binary.is_file():
            raise RuntimeError(f"No executable at {binary}. Run caddy build --profile {args.profile} first.")
        return subprocess.run(command, cwd=ROOT).returncode
    if (ROOT / ".servobuild").exists():
        raise RuntimeError(
            ".servobuild exists; this builder does not interpret mach settings. "
            "Use mach until those settings are migrated explicitly."
        )
    start = time.monotonic()
    env = environment()
    tools = ("cargo", "rustc", "clang-cl.exe", "lld-link.exe", "cmake", "nasm", "python")
    missing = [name for name in tools if not shutil.which(name, path=env["PATH"])]
    if args.action == "doctor":
        for name in tools:
            print(f"{name}: {shutil.which(name, path=env['PATH']) or 'MISSING'}")
        print(f"Target: {env['CARGO_TARGET_DIR']}")
        print(f"Python: {sys.executable}")
    if missing:
        raise RuntimeError("Missing build tools: " + ", ".join(missing))
    if args.action == "doctor":
        return 0
    command = cargo_command(args)
    binary_dir = Path(env["CARGO_TARGET_DIR"]) / ("debug" if args.profile == "dev" else args.profile)
    binary = binary_dir / "servoshell.exe"
    print(subprocess.list2cmdline(command), flush=True)
    print("Media: dummy (same as this fork's existing medium build).", flush=True)
    if args.dry_run:
        if args.action == "run":
            print(subprocess.list2cmdline([str(binary), *run_args]), flush=True)
        return 0
    status = subprocess.run(command, cwd=ROOT, env=env).returncode
    print(f"Cargo: {time.monotonic() - start:.1f}s", flush=True)
    if status or args.action == "check":
        return status
    stage_runtime(binary_dir)
    print(f"Executable: {binary}", flush=True)
    if args.action == "run":
        return subprocess.run([str(binary), *run_args], cwd=ROOT, env=env).returncode
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
    except (OSError, RuntimeError, subprocess.CalledProcessError, StopIteration) as error:
        print(f"Build error: {error}", file=sys.stderr)
        sys.exit(1)
