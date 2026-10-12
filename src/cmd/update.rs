//! Update Core without losing local commits or untracked files.
//! Ordinary updates must be fast-forwards. Rewritten mirror history requires an explicit reset,
//! which first retains HEAD on a recovery branch. Restarting the realm remains a separate action.

use crate::proc::{CommandSpec, ProcessRunner};
use crate::project::ProjectLayout;
use crate::{Error, Result};

const LYRACORE_CLI_PIN: &str = ".lyracore-cli-rev";

pub fn run(project: &ProjectLayout, runner: &dyn ProcessRunner, reset_mirror: bool) -> Result<()> {
    let git = || CommandSpec::new("git").cwd(project.root.clone());
    // Refuse local edits before even fetching. --keep below also protects edits made during fetch.
    refuse_if_dirty(&runner.run_and_wait(&git().arg("status").arg("--porcelain"))?)?;
    println!("· fetching origin...");
    runner.run_and_wait(&git().arg("fetch").arg("origin"))?;
    let head = runner.run_and_wait(&git().arg("rev-parse").arg("HEAD"))?;
    let upstream = runner.run_and_wait(&git().arg("rev-parse").arg("origin/main"))?;
    let head = head.trim();
    let upstream = upstream.trim();
    if head == upstream {
        println!("already up to date.");
        return Ok(());
    }
    if !reset_mirror {
        let local = runner.run_and_wait(
            &git()
                .arg("rev-list")
                .arg("--count")
                .arg(format!("{upstream}..{head}")),
        )?;
        let count = local.trim().parse::<u64>().map_err(|_| {
            Error::Process("could not count local commits; nothing was updated".into())
        })?;
        if count != 0 {
            return Err(Error::Process(format!(
                "HEAD has {count} commit(s) absent from origin/main. Nothing was updated. \
                 Reconcile local development with Git. To replace rewritten mirror history, use \
                 `lyracore update --reset-mirror`; it saves the current revision on a recovery branch first."
            )));
        }
    }
    // Git reset protects ordinary untracked files, but can overwrite ignored ones. Check both,
    // including untracked directories that an incoming tracked file would replace.
    let untracked = runner.run_and_wait(
        &git()
            .arg("ls-files")
            .arg("--others")
            .arg("--directory")
            .arg("-z"),
    )?;
    let incoming = runner.run_and_wait(
        &git()
            .arg("ls-tree")
            .arg("-r")
            .arg("--name-only")
            .arg("-z")
            .arg(upstream),
    )?;
    for path in untracked.split('\0').filter(|p| !p.is_empty()) {
        let path = path.trim_end_matches('/');
        if incoming
            .split('\0')
            .filter(|p| !p.is_empty())
            .any(|tracked| {
                tracked == path
                    || tracked.starts_with(&format!("{path}/"))
                    || path.starts_with(&format!("{tracked}/"))
            })
        {
            return Err(Error::Process(format!("untracked or ignored path {path:?} overlaps origin/main. Move it aside before updating; nothing was updated.")));
        }
    }
    if reset_mirror {
        let backup = format!("lyracore-backup/{head}");
        // The name identifies its content, so repeating an interrupted reset preserves the same ref.
        runner.run_and_wait(
            &git()
                .arg("update-ref")
                .arg(format!("refs/heads/{backup}"))
                .arg(head),
        )?;
        println!("saved the previous revision on branch {backup}");
    }
    let old_pin = read_pin(project);
    runner.run_and_wait(&git().arg("reset").arg("--keep").arg(upstream))?;
    println!("updated {} → {}", short(head), short(upstream));
    if old_pin != read_pin(project) {
        println!(
            "the CLI pin changed; the next `lyracore` invocation installs the pinned revision."
        );
    }
    println!("this does not restart anything. To rebuild and republish the local realm, run:");
    println!("  ./lyracore dev down && ./lyracore dev up");
    Ok(())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn refuse_if_dirty(porcelain: &str) -> Result<()> {
    let dirty: Vec<_> = porcelain
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with("??"))
        .collect();
    if dirty.is_empty() {
        return Ok(());
    }
    Err(Error::Process(format!(
        "commit or stash tracked edits before updating:\n{}",
        dirty.join("\n")
    )))
}

fn read_pin(project: &ProjectLayout) -> Option<String> {
    std::fs::read_to_string(project.root.join(LYRACORE_CLI_PIN))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    fn project(tmp: &TempDir) -> ProjectLayout {
        std::fs::write(tmp.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        ProjectLayout::from_root(tmp.path()).unwrap()
    }
    #[test]
    fn refuse_if_dirty_treats_only_non_double_question_mark_lines_as_dirty() {
        assert!(refuse_if_dirty("?? new.txt\n?? another.txt\n").is_ok());
        assert!(refuse_if_dirty("").is_ok());
        assert!(refuse_if_dirty(" M changed.rs\n").is_err());
        assert!(refuse_if_dirty("A  staged.rs\n").is_err());
        assert!(refuse_if_dirty(" D removed.rs\n").is_err());
        assert!(refuse_if_dirty("?? untracked.rs\n M changed.rs\n").is_err());
    }

    #[test]
    fn the_pin_is_read_trimmed_or_reads_as_absent() {
        let tmp = TempDir::new().unwrap();
        let project = project(&tmp);
        assert_eq!(read_pin(&project), None);
        std::fs::write(project.root.join(LYRACORE_CLI_PIN), "abc123\n").unwrap();
        assert_eq!(read_pin(&project).as_deref(), Some("abc123"));
    }

    #[test]
    fn short_truncates_a_full_sha_to_seven_characters() {
        assert_eq!(
            short("aaaa1111111111111111111111111111111111111"),
            "aaaa111"
        );
    }

    #[test]
    fn short_leaves_anything_already_seven_characters_or_shorter_alone() {
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
        assert_eq!(short("1234567"), "1234567");
    }
}
