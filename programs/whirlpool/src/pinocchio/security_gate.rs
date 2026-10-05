//! Security gate for the reposition / fee review (see `programs/whirlpool/security/README.md`).
//!
//! These tests keep `security/reposition_fee_register.json` honest:
//! - every function in the reviewed files must be tied to a claim (or listed as trivial),
//!   so new code can't land unreviewed;
//! - every piece of evidence the register cites must exist;
//! - the verdict can only be "clean" when every claim is tested or argued, the code still
//!   matches the reviewed fingerprint, an independent reviewer signed off, and every planted
//!   bug in `security/mutants.json` was caught.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const REGISTER: &str = "programs/whirlpool/security/reposition_fee_register.json";
const MUTANTS: &str = "programs/whirlpool/security/mutants.json";
const MUTATION_RESULTS: &str = "programs/whirlpool/security/mutation_results.json";

const KINDS: [&str; 3] = ["invariant", "assumption", "question"];
const STATUSES: [&str; 5] = ["open", "partial", "tested", "argued", "broken"];
// Invariants that must be checked against the release build (LiteSVM harnesses), because the
// deployed program is built with overflow checks off.
const RELEASE_BUILD_INVARIANTS: [&str; 7] = ["I1", "I2", "I3", "I4", "I5", "I6", "I7"];
const RELEASE_BUILD_HARNESS_PREFIXES: [&str; 2] = ["legacy-sdk/", "rust-sdk/"];
const MIN_ARGUMENT_LEN: usize = 300;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn read_json(rel: &str) -> Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("parse {rel}: {e}"))
}

fn str_list(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` must be an array"))
        .iter()
        .map(|v| {
            v.as_str()
                .expect("array entries must be strings")
                .to_string()
        })
        .collect()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// FNV-1a 64 over (path, NUL, contents, NUL) per scope file in sorted order.
/// Must match `fingerprint` in `security/run_mutants.py`.
fn scope_fingerprint(scope_files: &[String]) -> String {
    let mut sorted = scope_files.to_vec();
    sorted.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for rel in &sorted {
        let contents = std::fs::read(repo_root().join(rel)).expect("scope file");
        for chunk in [rel.as_bytes(), &[0], &contents, &[0]] {
            for byte in chunk {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{hash:016x}")
}

/// The file with every `#[cfg(test)]` item removed (the whole braced block, or up to `;`),
/// so non-test code placed after a test module is still seen.
fn without_test_items(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(pos) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let brace = after.find('{');
        let semi = after.find(';');
        rest = match (brace, semi) {
            (Some(b), Some(s)) if s < b => &after[s + 1..],
            (Some(b), _) => {
                let mut depth = 0usize;
                let mut end = after.len();
                for (i, c) in after[b..].char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = b + i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                &after[end..]
            }
            (None, Some(s)) => &after[s + 1..],
            (None, None) => "",
        };
    }
    out.push_str(rest);
    out
}

/// Names of non-test functions defined in a file.
fn functions_in(rel: &str) -> BTreeSet<String> {
    let source = without_test_items(&read(rel));
    let code = source.as_str();
    let mut names = BTreeSet::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while let Some(pos) = code[i..].find("fn ") {
        let start = i + pos;
        let at_word_start =
            start == 0 || !(bytes[start - 1] as char).is_alphanumeric() && bytes[start - 1] != b'_';
        let name: String = code[start + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let next = code[start + 3 + name.len()..].chars().next();
        if at_word_start && !name.is_empty() && matches!(next, Some('(') | Some('<')) {
            names.insert(name);
        }
        i = start + 3;
    }
    names
}

/// A reference is `path::rust_test_fn` or `path::ts test title`.
fn evidence_exists(reference: &str) -> Result<(), String> {
    let (path, name) = reference
        .split_once("::")
        .ok_or_else(|| format!("`{reference}` is not `path::test`"))?;
    let source = std::fs::read_to_string(repo_root().join(path))
        .map_err(|_| format!("`{reference}`: file {path} not found"))?;
    let found = if path.ends_with(".rs") {
        source.contains(&format!("fn {name}("))
    } else {
        source
            .lines()
            .any(|line| line.contains("it(") && line.contains(name))
            || source.contains(&format!("it(\"{name}\"")) // multi-line `it(` calls
            || source
                .lines()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|w| w[0].trim_end().ends_with("it(") && w[1].contains(name))
    };
    if found {
        Ok(())
    } else {
        Err(format!("`{reference}`: test not found in {path}"))
    }
}

struct Register {
    root: Value,
    items: Vec<Value>,
    scope_files: Vec<String>,
}

fn register() -> Register {
    let root = read_json(REGISTER);
    let items = root["items"].as_array().expect("items").clone();
    let scope_files = str_list(&root, "scope_files");
    Register {
        root,
        items,
        scope_files,
    }
}

#[test]
fn register_is_well_formed() {
    let reg = register();
    let mut ids = BTreeSet::new();
    for item in &reg.items {
        let id = text(item, "id");
        assert!(!id.is_empty(), "item without id");
        assert!(ids.insert(id.to_string()), "duplicate id {id}");
        assert!(KINDS.contains(&text(item, "kind")), "{id}: bad kind");
        assert!(STATUSES.contains(&text(item, "status")), "{id}: bad status");
        assert!(text(item, "claim").len() >= 20, "{id}: claim missing");
        for key in ["covers", "tests"] {
            str_list(item, key);
        }
    }
    let verdict = text(&reg.root, "verdict");
    assert!(
        verdict == "open" || verdict == "clean",
        "verdict must be \"open\" or \"clean\""
    );
    for rel in &reg.scope_files {
        assert!(repo_root().join(rel).exists(), "scope file {rel} missing");
    }
}

#[test]
fn cited_evidence_exists() {
    let reg = register();
    let mut problems = Vec::new();
    for item in &reg.items {
        let id = text(item, "id");
        let status = text(item, "status");
        let tests = str_list(item, "tests");
        for reference in &tests {
            if let Err(e) = evidence_exists(reference) {
                problems.push(format!("{id}: {e}"));
            }
        }
        match status {
            "tested" | "partial" if tests.is_empty() => {
                problems.push(format!("{id}: status {status} needs at least one test"))
            }
            "argued" => {
                if text(item, "argument").len() < MIN_ARGUMENT_LEN {
                    problems.push(format!(
                        "{id}: an argued claim needs a written argument of at least {MIN_ARGUMENT_LEN} characters"
                    ));
                }
                match evidence_exists(text(item, "tripwire")) {
                    Ok(()) => {}
                    Err(e) => problems.push(format!(
                        "{id}: an argued claim needs a tripwire test that fails if the argument stops holding ({e})"
                    )),
                }
            }
            _ => {}
        }
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n"));
}

#[test]
fn every_function_in_scope_is_claimed() {
    let reg = register();
    let mut claimed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in &reg.items {
        for key in str_list(item, "covers") {
            claimed
                .entry(key)
                .or_default()
                .push(text(item, "id").to_string());
        }
    }
    let trivial: BTreeSet<String> = reg.root["trivial"]
        .as_object()
        .expect("trivial")
        .keys()
        .cloned()
        .collect();

    let mut defined = BTreeSet::new();
    for rel in &reg.scope_files {
        for name in functions_in(rel) {
            defined.insert(format!("{rel}::{name}"));
        }
    }

    let unclaimed: Vec<_> = defined
        .iter()
        .filter(|f| !claimed.contains_key(*f) && !trivial.contains(*f))
        .collect();
    let stale: Vec<_> = claimed
        .keys()
        .chain(trivial.iter())
        .filter(|f| !defined.contains(*f))
        .collect();
    assert!(
        unclaimed.is_empty() && stale.is_empty(),
        "\nFunctions with no claim (add them to an item's `covers`, or to `trivial` with a reason):\n  {}\nEntries naming functions that no longer exist:\n  {}",
        unclaimed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  "),
        stale.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  "),
    );
}

#[test]
fn mutants_target_reviewed_code() {
    let reg = register();
    let mutants = read_json(MUTANTS);
    let harnesses = mutants["harnesses"].as_object().expect("harnesses");
    for mutant in mutants["mutants"].as_array().expect("mutants") {
        let id = text(mutant, "id");
        let file = text(mutant, "file");
        assert!(
            reg.scope_files.iter().any(|f| f == file),
            "{id}: {file} is not in scope_files"
        );
        assert!(
            harnesses.contains_key(text(mutant, "harness")),
            "{id}: unknown harness"
        );
        let count = read(file).matches(text(mutant, "find")).count();
        assert_eq!(
            count, 1,
            "{id}: anchor text must appear exactly once in {file}"
        );
    }
}

#[test]
fn clean_verdict_requires_full_evidence() {
    let reg = register();
    if text(&reg.root, "verdict") != "clean" {
        return;
    }
    let mut problems = Vec::new();

    let current = scope_fingerprint(&reg.scope_files);
    if text(&reg.root, "scope_fingerprint") != current {
        problems.push(format!(
            "the reviewed code changed since sign-off (scope_fingerprint is {current} now); re-review and update it"
        ));
    }

    let mut owners = BTreeSet::new();
    for item in &reg.items {
        let id = text(item, "id");
        let status = text(item, "status");
        if status != "tested" && status != "argued" {
            problems.push(format!("{id}: status is {status}"));
        }
        let owner = text(item, "owner");
        if owner.is_empty() {
            problems.push(format!("{id}: no owner"));
        }
        owners.insert(owner.to_string());
        if RELEASE_BUILD_INVARIANTS.contains(&id)
            && !str_list(item, "tests").iter().any(|t| {
                RELEASE_BUILD_HARNESS_PREFIXES
                    .iter()
                    .any(|p| t.starts_with(p))
            })
        {
            problems.push(format!(
                "{id}: needs a test in a LiteSVM harness (legacy-sdk/ or rust-sdk/) that runs the release build"
            ));
        }
    }

    let reviewer = text(&reg.root, "reviewer");
    if reviewer.is_empty() || owners.contains(reviewer) {
        problems.push("reviewer must be set and must not own any item".to_string());
    }

    match std::fs::read_to_string(repo_root().join(MUTATION_RESULTS)) {
        Err(_) => problems.push("no mutation_results.json; run security/run_mutants.py".into()),
        Ok(raw) => {
            let results: Value = serde_json::from_str(&raw).expect("mutation results");
            if text(&results, "fingerprint") != current {
                problems
                    .push("mutation results are for different code; rerun run_mutants.py".into());
            }
            for mutant in read_json(MUTANTS)["mutants"].as_array().unwrap() {
                let id = text(mutant, "id");
                let outcome = results["results"][id].as_str().unwrap_or("not_run");
                if outcome != "killed" {
                    problems.push(format!("planted bug {id} was not caught ({outcome})"));
                }
            }
        }
    }

    assert!(
        problems.is_empty(),
        "\nverdict \"clean\" is not supported:\n{}",
        problems.join("\n")
    );
}

#[test]
fn print_scope_fingerprint() {
    // Run with `-- --nocapture` to see the value to record at sign-off.
    println!(
        "scope_fingerprint = {}",
        scope_fingerprint(&register().scope_files)
    );
}
