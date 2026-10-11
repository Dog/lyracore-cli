//! Prepare the Gateway's private Rust bindings without contacting a Realm.

use super::preflight::{pinned_rust, pinned_spacetimedb, schema_command};
use crate::cmd::doctor::parse_version;
use crate::proc::{CommandSpec, ProcessRunner};
use crate::project::ProjectLayout;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const CACHE: &str = ".lyracore/generated/gateway";
const RECEIPT: &str = "current.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Inputs {
    files: BTreeMap<PathBuf, String>,
    watches: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    inputs: Inputs,
    directory: PathBuf,
    output: BTreeMap<PathBuf, String>,
}

/// The Cargo build script reads this JSON after a successful `bindings --check`.
#[derive(Debug, Serialize)]
pub struct Prepared {
    pub directory: PathBuf,
    pub watches: Vec<PathBuf>,
}

fn stale(reason: impl std::fmt::Display) -> Error {
    Error::Process(format!(
        "Gateway bindings need preparation ({reason}). Run `./lyracore bindings`."
    ))
}

fn collect(
    path: &Path,
    files: &mut BTreeMap<PathBuf, String>,
    ancestors: &mut Vec<PathBuf>,
) -> Result<()> {
    let resolved = fs::canonicalize(path)?;
    if path.is_dir() {
        if ancestors.contains(&resolved) {
            return Err(stale(format!(
                "source directory cycle at {}",
                path.display()
            )));
        }
        ancestors.push(resolved);
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if matches!(
                entry.file_name().to_str(),
                Some("target" | ".git" | "node_modules")
            ) {
                continue;
            }
            collect(&entry.path(), files, ancestors)?;
        }
        ancestors.pop();
    } else {
        files.insert(
            path.to_path_buf(),
            blake3::hash(&fs::read(path)?).to_hex().to_string(),
        );
    }
    Ok(())
}

fn inputs(project: &ProjectLayout) -> Result<Inputs> {
    let mut watches: Vec<PathBuf> = [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".lyracore-cli-rev",
        ".cargo",
        "module",
        "crates",
        "packages",
        "datascripts/runtime-scripts/events.json",
    ]
    .iter()
    .map(|path| project.root.join(path))
    .collect();
    let mut files = BTreeMap::new();
    // Package content outside src/ does not enter the Module wasm. Watch the directory itself
    // for installations and removals, and each source tree for edits through Package symlinks.
    for path in watches.clone() {
        if path.ends_with("packages") {
            if path.is_dir() {
                for entry in fs::read_dir(&path)? {
                    let package = entry?.path();
                    if !package.is_dir() {
                        continue;
                    }
                    let source = package.join("src");
                    watches.push(source.clone());
                    if source.exists() {
                        collect(&source, &mut files, &mut Vec::new())?;
                    }
                }
            }
        } else if path.exists() {
            collect(&path, &mut files, &mut Vec::new())?;
        }
    }
    watches.sort();
    Ok(Inputs { files, watches })
}

fn read_receipt(project: &ProjectLayout) -> Result<Receipt> {
    let bytes = fs::read(project.root.join(CACHE).join(RECEIPT)).map_err(stale)?;
    serde_json::from_slice(&bytes).map_err(stale)
}

/// Read-only. It needs neither Cargo nor the SpacetimeDB executable.
pub fn check(project: &ProjectLayout) -> Result<Prepared> {
    let receipt = read_receipt(project)?;
    if receipt.inputs != inputs(project)? {
        return Err(stale("Module inputs changed"));
    }
    let mut output = BTreeMap::new();
    collect(&receipt.directory, &mut output, &mut Vec::new()).map_err(stale)?;
    if output != receipt.output || !receipt.directory.join("mod.rs").is_file() {
        return Err(stale("generated files changed or are missing"));
    }
    let mut watches = receipt.inputs.watches;
    watches.extend(receipt.inputs.files.into_keys());
    watches.push(project.root.join(CACHE).join(RECEIPT));
    watches.push(receipt.directory.clone());
    Ok(Prepared {
        directory: receipt.directory,
        watches,
    })
}

fn versions(project: &ProjectLayout, runner: &dyn ProcessRunner) -> Result<()> {
    let rust = pinned_rust(&fs::read_to_string(project.rust_toolchain_file())?)
        .ok_or_else(|| stale("cannot read the Rust pin"))?;
    let spacetime = pinned_spacetimedb(&fs::read_to_string(project.module_manifest())?)
        .ok_or_else(|| stale("cannot read the SpacetimeDB pin"))?;
    for (program, wanted) in [("rustc", rust), ("spacetime", spacetime)] {
        let banner = runner.run_capturing_stderr(
            &CommandSpec::new(program)
                .arg("--version")
                .cwd(&project.root),
        )?;
        if parse_version(&banner) != Some(wanted) {
            return Err(stale(format!("{program} must be {wanted}")));
        }
    }
    Ok(())
}

/// Reuse current output or generate a complete replacement. Existing readers keep their immutable
/// directory; a failed generation never replaces the receipt. No command here publishes a Module.
pub fn ensure(project: &ProjectLayout, runner: &dyn ProcessRunner) -> Result<Prepared> {
    let cache = project.root.join(CACHE);
    fs::create_dir_all(&cache)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(cache.join("lock"))?;
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)
        .map_err(std::io::Error::from)?;
    if let Ok(prepared) = check(project) {
        return Ok(prepared);
    }
    versions(project, runner)?;
    let before = inputs(project)?;
    let staging = tempfile::Builder::new()
        .prefix("bindings-")
        .tempdir_in(&cache)?;
    eprintln!("lyracore: generating Gateway bindings...");
    runner.run_and_wait(&schema_command(project, staging.path()).arg("--include-private"))?;
    if !staging.path().join("mod.rs").is_file() {
        return Err(stale("generator did not write mod.rs"));
    }
    if before != inputs(project)? {
        return Err(stale("Module inputs changed during generation"));
    }
    let mut output = BTreeMap::new();
    collect(staging.path(), &mut output, &mut Vec::new())?;
    let receipt = Receipt {
        inputs: before,
        directory: staging.path().to_path_buf(),
        output,
    };
    let mut current = tempfile::NamedTempFile::new_in(&cache)?;
    current.write_all(&serde_json::to_vec(&receipt).map_err(stale)?)?;
    // Keep the directory before publishing its path. Old directories remain usable by a compiler
    // that started before this generation; removing .lyracore/generated/ clears the cache.
    let _directory = staging.keep();
    current.persist(cache.join(RECEIPT)).map_err(stale)?;
    check(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake::{FakeStack, FAKE_RUST_VERSION, FAKE_SPACETIME_VERSION};

    fn fixture(root: &Path) -> ProjectLayout {
        fs::create_dir_all(root.join("module/src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(
            root.join("rust-toolchain.toml"),
            format!("channel = \"{FAKE_RUST_VERSION}\"\n"),
        )
        .unwrap();
        fs::write(
            root.join("module/Cargo.toml"),
            format!("spacetimedb = {{ version = \"={FAKE_SPACETIME_VERSION}\" }}\n"),
        )
        .unwrap();
        fs::write(root.join("module/src/lib.rs"), "// Module\n").unwrap();
        ProjectLayout::from_root(root).unwrap()
    }

    #[test]
    fn a_clean_checkout_gets_private_bindings_and_reuses_them_without_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        assert!(check(&project)
            .unwrap_err()
            .to_string()
            .contains("./lyracore bindings"));
        let stack = FakeStack::new();
        let prepared = ensure(&project, &stack.runner()).unwrap();
        assert!(prepared.directory.join("mod.rs").is_file());
        let calls = stack.rendered();
        let generate = calls
            .iter()
            .find(|call| call.starts_with("spacetime generate"))
            .unwrap();
        assert!(generate.contains("--include-private"));
        assert!(generate.contains("--features=debug_reducers"));
        assert!(generate.contains("--no-config"));
        let generation = schema_command(&project, tmp.path());
        let publish = crate::cmd::publish::publish_command(&project, "fixture").unwrap();
        for variable in [
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
        ] {
            assert!(generation.removes_env(variable));
            assert!(publish.removes_env(variable));
        }

        let no_tools = FakeStack::new()
            .fail_on("spacetime", "unavailable")
            .fail_on("rustc", "unavailable");
        assert_eq!(
            ensure(&project, &no_tools.runner()).unwrap().directory,
            prepared.directory
        );
        assert!(no_tools.rendered().is_empty());
    }

    #[test]
    fn changed_added_and_removed_inputs_require_preparation() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        let stack = FakeStack::new();
        for path in [
            "module/src/lib.rs",
            "crates/new/src/lib.rs",
            "packages/example/src/mod.rs",
            ".lyracore-cli-rev",
            "Cargo.lock",
            "rust-toolchain.toml",
        ] {
            ensure(&project, &stack.runner()).unwrap();
            let path = tmp.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let previous = fs::read(&path).unwrap_or_default();
            let mut changed = previous.clone();
            changed.extend_from_slice(b"\n// changed\n");
            fs::write(&path, changed).unwrap();
            assert!(check(&project).is_err(), "{}", path.display());
            fs::write(&path, previous).unwrap();
        }
        ensure(&project, &stack.runner()).unwrap();
        fs::remove_file(tmp.path().join("packages/example/src/mod.rs")).unwrap();
        assert!(check(&project).is_err());
    }

    #[test]
    fn a_failed_replacement_keeps_the_previous_complete_output() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        let first = ensure(&project, &FakeStack::new().runner()).unwrap();
        let receipt = fs::read(tmp.path().join(CACHE).join(RECEIPT)).unwrap();
        fs::write(tmp.path().join("module/src/new.rs"), "// new table").unwrap();
        let failed = FakeStack::new().fail_on("spacetime generate", "invalid schema");
        assert!(ensure(&project, &failed.runner()).is_err());
        assert_eq!(
            fs::read(tmp.path().join(CACHE).join(RECEIPT)).unwrap(),
            receipt
        );
        assert!(first.directory.join("mod.rs").is_file());
        assert!(check(&project).is_err());
        let replacement = ensure(&project, &FakeStack::new().runner()).unwrap();
        assert_ne!(first.directory, replacement.directory);
        assert!(first.directory.join("mod.rs").is_file());
    }

    #[test]
    fn modified_or_deleted_output_cannot_pass_the_check() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        let prepared = ensure(&project, &FakeStack::new().runner()).unwrap();
        fs::write(prepared.directory.join("character_type.rs"), "broken").unwrap();
        assert!(check(&project).is_err());
        let prepared = ensure(&project, &FakeStack::new().runner()).unwrap();
        fs::remove_file(prepared.directory.join("character_type.rs")).unwrap();
        assert!(check(&project).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn edits_and_retargeting_of_linked_packages_invalidate_bindings() {
        let tmp = tempfile::tempdir().unwrap();
        let package = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        fs::create_dir_all(tmp.path().join("packages")).unwrap();
        fs::create_dir_all(package.path().join("src")).unwrap();
        fs::write(package.path().join("src/mod.rs"), "// package").unwrap();
        let link = tmp.path().join("packages/example");
        std::os::unix::fs::symlink(package.path(), &link).unwrap();
        ensure(&project, &FakeStack::new().runner()).unwrap();
        fs::write(package.path().join("src/mod.rs"), "// changed package").unwrap();
        assert!(check(&project).is_err());
        ensure(&project, &FakeStack::new().runner()).unwrap();
        fs::remove_file(link).unwrap();
        assert!(check(&project).is_err());
    }

    #[test]
    fn wrong_generator_version_fails_before_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let project = fixture(tmp.path());
        let stack =
            FakeStack::new().with_stdout("spacetime --version", "spacetimedb tool version 0.1.0");
        assert!(ensure(&project, &stack.runner()).is_err());
        assert!(!stack
            .rendered()
            .iter()
            .any(|call| call.starts_with("spacetime generate")));
    }
}
