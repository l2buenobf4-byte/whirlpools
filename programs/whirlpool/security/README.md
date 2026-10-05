# Reposition and fee review: blueprint and gate

This folder holds the security review of `reposition_liquidity_v2` and the fee, reward and
rent accounting it touches. The review is enforced by tests in
`src/pinocchio/security_gate.rs`, which run in CI as part of `cargo test -p whirlpool`.

The goal is not "no failing tests". The goal is evidence, for every claim this code's safety
rests on, that an attacker can't break it.

## Files

| File | What it is |
| --- | --- |
| `reposition_fee_register.json` | Every claim (invariants I*, assumptions A*, attacker questions Q*), the functions it covers, its owner, status and evidence. |
| `mutants.json` | Planted bugs. Each must be caught by the test suite before the review can be called clean. |
| `run_mutants.py` | Plants each bug, runs its harness, restores the code, and writes `mutation_results.json`. |
| `mutation_results.json` | Last results, stamped with the fingerprint of the code they ran against. |

## What the gate enforces

These fail CI. There's no flag to skip them.

1. **Every function in the reviewed files has a claim.** Add a function to any file in
   `scope_files` and CI fails until it's added to an item's `covers`, or to `trivial` with a
   reason. Removing or renaming one fails too, until the register is updated.
2. **Cited evidence exists.** Every test named in `tests` or `tripwire` must exist. A `tested`
   or `partial` item needs at least one test. An `argued` item needs a written argument of
   300+ characters *and* a tripwire test that fails if the argument stops being true.
3. **Planted bugs target reviewed code.** Each mutant's file is in scope, and its anchor text
   appears exactly once.
4. **"clean" must be earned.** `verdict: "clean"` only passes when:
   - every item is `tested` or `argued`, and has an owner;
   - invariants I1-I7 each have a test in `legacy-sdk/` or `rust-sdk/`, the LiteSVM harnesses
     that run the release build;
   - `reviewer` is set and isn't the owner of any item;
   - every planted bug in `mutants.json` is `killed` in `mutation_results.json`;
   - `scope_fingerprint` matches the current code. **Any change to a scope file invalidates a
     clean verdict** until it's re-reviewed and the fingerprint updated.

## Statuses

| Status | Meaning |
| --- | --- |
| `open` | No evidence yet. |
| `partial` | Some tests exist but don't cover the whole claim (for example unit level only). Counts as open. |
| `tested` | Tests cover the whole claim. |
| `argued` | Can't be tested directly; written argument plus a tripwire test. |
| `broken` | A failing case was found. Record it here and fix it; the verdict can't be clean. |

## Workflow

1. **Assign owners.** One engineer per item. The `reviewer` must be someone who owns nothing.
2. **Add the money checks first (I1-I7)** as helpers in the LiteSVM harnesses, and call them
   after every instruction in every reposition, increase, decrease and collect test. A test that
   only checks "the transaction succeeded" can't find theft.
3. **Work items to `tested` or `argued`.** Write each test to fail on the attacker's success,
   not to pass on the happy path. Record the test path in `tests`.
4. **Grow `mutants.json`.** For every new claim, add at least one planted bug that would break
   it, then run `run_mutants.py` until every bug is caught. A surviving bug means a gap in the
   tests, not a problem with the bug.
5. **Sign off.** Run the gate with `-- --nocapture` to print the fingerprint, record it in
   `scope_fingerprint`, set `reviewer`, set `verdict` to `clean`, and push. CI will refuse it
   if anything above is missing.

## Running things

```sh
# Gate and unit tests (host, debug build: overflow checks ON)
cargo test -p whirlpool --lib pinocchio

# Same tests with the deployed overflow behaviour (release profile: overflow checks OFF)
cargo test -p whirlpool --lib --release pinocchio

# Print the current scope fingerprint
cargo test -p whirlpool --lib pinocchio::security_gate::print_scope_fingerprint -- --nocapture

# Planted bugs: host harness only, or everything (needs anchor + yarn)
python3 programs/whirlpool/security/run_mutants.py --only host
python3 programs/whirlpool/security/run_mutants.py
```

The deployed program is built with `overflow-checks = false` (root `Cargo.toml`). Host tests in
debug panic where the chain silently wraps, so money checks must run in the LiteSVM harnesses,
which load the real `target/deploy/whirlpool.so`.

## What the gate can't do

It checks that evidence exists, not that a test is good. A weak test can be cited as
evidence. That gap is closed by the planted bugs (a weak test lets them survive) and by the
independent reviewer. Neither the gate nor this review can prove the absence of every bug.
Keep the on-chain backstops: a vault solvency monitor, a way to switch off the Pinocchio
liquidity instructions, and a bug bounty.
