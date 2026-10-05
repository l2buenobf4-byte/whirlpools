#!/usr/bin/env python3
"""Plant each bug from mutants.json, run its harness, and record whether the tests caught it.

A mutant is "killed" when the harness fails with the bug in place. Every source file is
restored after each run, even on error or Ctrl-C. Results are written to
mutation_results.json together with the scope fingerprint, so the security gate can tell
whether they still apply to the current code.

Usage (from anywhere in the repo):
    python3 programs/whirlpool/security/run_mutants.py            # all harnesses
    python3 programs/whirlpool/security/run_mutants.py --only host
    python3 programs/whirlpool/security/run_mutants.py M3 M8      # selected mutants
"""

import argparse
import json
import pathlib
import subprocess
import sys

SECURITY_DIR = pathlib.Path(__file__).resolve().parent
REPO_ROOT = SECURITY_DIR.parent.parent.parent
REGISTER = SECURITY_DIR / "reposition_fee_register.json"
MUTANTS = SECURITY_DIR / "mutants.json"
RESULTS = SECURITY_DIR / "mutation_results.json"


def fingerprint(scope_files):
    """FNV-1a 64 over (path, NUL, contents, NUL) for each scope file in sorted order.

    Must match `scope_fingerprint` in src/pinocchio/security_gate.rs.
    """
    h = 0xCBF29CE484222325
    for rel in sorted(scope_files):
        for chunk in (rel.encode(), b"\0", (REPO_ROOT / rel).read_bytes(), b"\0"):
            for byte in chunk:
                h ^= byte
                h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def run_mutant(mutant, command):
    path = REPO_ROOT / mutant["file"]
    original = path.read_text()
    count = original.count(mutant["find"])
    if count != 1:
        return f"invalid: anchor text found {count} times in {mutant['file']}"
    try:
        path.write_text(original.replace(mutant["find"], mutant["replace"], 1))
        proc = subprocess.run(
            command, shell=True, cwd=REPO_ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
    finally:
        path.write_text(original)
    return "killed" if proc.returncode != 0 else "survived"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("ids", nargs="*", help="mutant ids to run (default: all)")
    parser.add_argument("--only", choices=["host", "litesvm"], help="run one harness only")
    args = parser.parse_args()

    register = json.loads(REGISTER.read_text())
    spec = json.loads(MUTANTS.read_text())
    scope_print = fingerprint(register["scope_files"])

    previous = json.loads(RESULTS.read_text()) if RESULTS.exists() else {}
    results = previous.get("results", {}) if previous.get("fingerprint") == scope_print else {}

    # The unmutated suite must pass first, or every mutant would look "killed".
    for harness in sorted({m["harness"] for m in spec["mutants"]}):
        if args.only and harness != args.only:
            continue
        print(f"baseline [{harness}] ...", flush=True)
        if subprocess.run(spec["harnesses"][harness], shell=True, cwd=REPO_ROOT).returncode != 0:
            sys.exit(f"baseline {harness} suite fails without any mutant; fix that first")

    for mutant in spec["mutants"]:
        if args.ids and mutant["id"] not in args.ids:
            continue
        if args.only and mutant["harness"] != args.only:
            results.setdefault(mutant["id"], "not_run")
            continue
        outcome = run_mutant(mutant, spec["harnesses"][mutant["harness"]])
        results[mutant["id"]] = outcome
        print(f"{mutant['id']:>4} [{mutant['harness']}] {outcome}: {mutant['bug']}", flush=True)

    for mutant in spec["mutants"]:
        results.setdefault(mutant["id"], "not_run")

    RESULTS.write_text(
        json.dumps({"fingerprint": scope_print, "results": results}, indent=2, sort_keys=True) + "\n"
    )
    survived = [k for k, v in results.items() if v != "killed"]
    print(f"\n{len(results) - len(survived)} killed, {len(survived)} not killed: {', '.join(sorted(survived))}")
    sys.exit(1 if survived else 0)


if __name__ == "__main__":
    main()
