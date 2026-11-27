#!/usr/bin/env python3
"""Replay native file, Pi, and checkpoint observations in the Restore model."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


MODEL = Path(__file__).resolve().parent
ROOT = MODEL.parents[2]
PREFIX = "VERITY_RESTORE_TRACE "
ROUTES = {
    "preflight": ["PreflightRefused"],
    "partial_apply": ["ApplyFailed"],
    "fallback": [
        "PiFork", "PiNewSession", "PiAccepted", "RedoSaved", "RedoCleared", "RedoUnavailable",
    ],
    "compensation": ["PiFork", "PiRejected", "Compensated"],
    "compensation_stale": ["PiFork", "EditedPath", "PiRejected", "CompensationStale"],
    "save_failure": ["PiFork", "PiAccepted", "RedoSaveFailed"],
    "clear_failure": [
        "PiFork", "PiAccepted", "RedoSaved", "RedoWrongOwner", "EditedSibling",
        "RedoClearFailed", "RedoStale",
    ],
}


def run(args, cwd):
    return subprocess.run(
        args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        encoding="utf-8", errors="replace", check=False,
    )


def parse_observations(output):
    traces = {}
    for line in output.splitlines():
        if PREFIX not in line:
            continue
        name, steps = json.loads(line.split(PREFIX, 1)[1])
        if name not in ROUTES or name in traces:
            raise SystemExit(f"Unknown or duplicated native trace {name!r}")
        if [step[0] for step in steps] != ROUTES[name]:
            raise SystemExit(f"Missing, extra, or reordered observations for {name}")
        for event, files, sibling, session, redo in steps:
            if (
                not isinstance(files, list) or len(files) != 2
                or any(version not in {"Before", "Checkpoint", "Edited", "Missing"} for version in files)
                or sibling not in {"Before", "Edited"}
                or session not in {"Parent", "Child"}
                or type(redo) is not bool
            ):
                raise SystemExit(f"Invalid observation for {name}/{event}")
        traces[name] = steps
    if traces.keys() != ROUTES.keys():
        raise SystemExit(f"Missing native traces {sorted(ROUTES.keys() - traces.keys())}")
    return traces


def tla(value):
    if isinstance(value, list):
        return "<<" + ", ".join(tla(item) for item in value) + ">>"
    return json.dumps(value) if isinstance(value, str) else str(value).upper()


def replay(command, directory, label, steps, reachable):
    (directory / "Replay.tla").write_text(
        r"""---- MODULE Replay ----
EXTENDS Restore
VARIABLE index
Observations == """ + tla(steps) + r"""
ReplayInit == Init /\ index = 1
ReplayNext ==
    /\ Next
    /\ IF state'.event = "Step"
       THEN UNCHANGED index
       ELSE /\ index <= Len(Observations)
            /\ Observe(state') = Observations[index]
            /\ index' = index + 1
Unreplayed == index <= Len(Observations)
====
""", encoding="utf-8",
    )
    result = run(command + ["-deadlock", "-config", "Replay.cfg", "Replay.tla"], directory)
    if reachable:
        passed = result.returncode == 12 and "Invariant Unreplayed is violated." in result.stdout
    else:
        passed = result.returncode == 0 and "Model checking completed. No error" in result.stdout
    if not passed:
        raise SystemExit(f"Unexpected TLC replay result for {label}\n{result.stdout}")
    print(f"{label}: {'matched' if reachable else 'rejected'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("jar", nargs="?", default=os.environ.get("VERITY_TLC_JAR"), type=Path)
    parser.add_argument("--java", default="java")
    args = parser.parse_args()
    if args.jar is None or not args.jar.is_file():
        parser.error("provide the TLC JAR path or set VERITY_TLC_JAR")

    native = run([
        "cargo", "test", "--locked", "--manifest-path", "desktop/src-tauri/Cargo.toml",
        "restore_model_", "--", "--nocapture", "--test-threads=1",
    ], ROOT)
    if native.returncode != 0:
        raise SystemExit(native.stdout)
    traces = parse_observations(native.stdout)
    command = [args.java, "-jar", str(args.jar.resolve()), "-cleanup", "-workers", "1"]
    with tempfile.TemporaryDirectory(prefix="verity-restore-") as tmp:
        directory = Path(tmp)
        shutil.copyfile(MODEL / "Restore.tla", directory / "Restore.tla")
        (directory / "Replay.cfg").write_text(
            "INIT ReplayInit\nNEXT ReplayNext\nCONSTANT SkipStaleCheck = FALSE\nINVARIANT Unreplayed\n",
            encoding="utf-8",
        )
        for name, steps in traces.items():
            replay(command, directory, name, steps, reachable=True)

        wrong = json.loads(json.dumps(traces["compensation_stale"]))
        wrong[-1][1][0] = "Before"
        replay(command, directory, "stale compensation overwrites an edit", wrong, reachable=False)

        unsaved = traces["fallback"][:3] + [
            [event, ["Checkpoint", "Checkpoint"], "Before", "Child", False]
            for event in ["Crash", "Reopened", "RedoUnavailable"]
        ]
        uncleared = traces["fallback"][:4] + [
            [event, ["Before", "Before"], "Before", "Child", True]
            for event in ["Crash", "Reopened", "RedoStale"]
        ]
        replay(command, directory, "modeled restart before saving redo", unsaved, reachable=True)
        replay(command, directory, "modeled restart before clearing redo", uncleared, reachable=True)

        config = (MODEL / "Restore.cfg").read_text(encoding="utf-8")
        (directory / "Fault.cfg").write_text(
            config.replace("SkipStaleCheck = FALSE", "SkipStaleCheck = TRUE"), encoding="utf-8",
        )
        result = run(command + ["-config", "Fault.cfg", "Restore.tla"], directory)
        if result.returncode != 12 or "Invariant ApplyRequiresFreshness is violated." not in result.stdout:
            raise SystemExit(f"Disabled stale guard was not detected\n{result.stdout}")
        print("disabled stale guard: detected")


if __name__ == "__main__":
    main()
