//! "Server update available": each app version is released with one
//! rAthena commit, pinned in the bundled `.env.example`. The server keeps
//! running the commit in the user's `.env` until they choose to update.
//!
//! `.env` also remembers the last commit an app version offered
//! (`OFFERED_KEY`), set by updating or skipping. A commit set by hand
//! afterwards, to test a fork change, isn't offered back; the next app
//! version's commit is.

use serde::Serialize;

use crate::envfile::EnvFile;
use crate::stack;

pub const OFFERED_KEY: &str = "RATHENA_COMMIT_OFFERED";

#[derive(Serialize, Debug, PartialEq)]
pub struct ServerUpdate {
    /// The commit the server is built from now, from `.env`.
    pub current: String,
    /// The commit this version of the app comes with.
    pub offered: String,
    pub repo: String,
    /// GitHub's page of the commits in between, when both are on the same GitHub repository.
    pub changes_url: Option<String>,
}

/// The update this version of the app offers, if the server isn't on its commit yet.
pub fn pending(env: &EnvFile) -> Option<ServerUpdate> {
    offered(env, &EnvFile::parse(stack::ENV_EXAMPLE))
}

fn offered(env: &EnvFile, bundled: &EnvFile) -> Option<ServerUpdate> {
    let offered = bundled.get("RATHENA_COMMIT").filter(|c| !c.is_empty())?;
    let repo = bundled.get("RATHENA_REPO").filter(|r| !r.is_empty())?;
    let current = env.get("RATHENA_COMMIT").unwrap_or_default();
    let current_repo = env.get("RATHENA_REPO").unwrap_or_default();
    if (current == offered && current_repo == repo) || env.get(OFFERED_KEY).as_deref() == Some(offered.as_str()) {
        return None;
    }
    let changes_url = match github_repo(&repo) {
        Some(path) if current_repo == repo && !current.is_empty() => {
            Some(format!("https://github.com/{path}/compare/{current}...{offered}"))
        }
        _ => None,
    };
    Some(ServerUpdate {
        current,
        offered,
        repo,
        changes_url,
    })
}

/// `owner/name` for a `https://github.com/owner/name(.git)` URL.
fn github_repo(url: &str) -> Option<&str> {
    let path = url.strip_prefix("https://github.com/")?;
    let path = path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/');
    (path.split('/').count() == 2).then_some(path)
}

/// Records the update in `.env`, once the new image has been built.
pub fn pin(env: &mut EnvFile, update: &ServerUpdate) {
    env.set("RATHENA_REPO", &update.repo);
    env.set("RATHENA_COMMIT", &update.offered);
    skip(env, update);
}

/// Stops offering this commit, keeping the one in `.env`.
pub fn skip(env: &mut EnvFile, update: &ServerUpdate) {
    env.set(OFFERED_KEY, &update.offered);
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://github.com/akreao/rathena.git";

    fn env(repo: &str, commit: &str) -> EnvFile {
        EnvFile::parse(&format!("RATHENA_REPO={repo}\nRATHENA_COMMIT={commit}\n"))
    }

    #[test]
    fn nothing_when_pinned_to_the_bundled_commit() {
        assert_eq!(offered(&env(REPO, "aaa"), &env(REPO, "aaa")), None);
    }

    #[test]
    fn offers_a_new_commit_with_its_changes() {
        let update = offered(&env(REPO, "aaa"), &env(REPO, "bbb")).unwrap();
        assert_eq!(update.current, "aaa");
        assert_eq!(update.offered, "bbb");
        assert_eq!(update.changes_url.as_deref(), Some("https://github.com/akreao/rathena/compare/aaa...bbb"));
    }

    #[test]
    fn a_new_repository_has_no_changes_page() {
        let update = offered(&env("https://example.com/rathena.git", "aaa"), &env(REPO, "aaa")).unwrap();
        assert_eq!(update.repo, REPO);
        assert_eq!(update.changes_url, None);
    }

    #[test]
    fn a_missing_pin_is_offered_the_bundled_one() {
        let update = offered(&EnvFile::parse("PACKETVER=20250716\n"), &env(REPO, "bbb")).unwrap();
        assert_eq!(update.current, "");
        assert_eq!(update.changes_url, None);
    }

    #[test]
    fn the_bundled_example_has_a_pin() {
        let bundled = EnvFile::parse(stack::ENV_EXAMPLE);
        assert_eq!(bundled.get("RATHENA_COMMIT").unwrap().len(), 40);
        assert_eq!(github_repo(&bundled.get("RATHENA_REPO").unwrap()), Some("akreao/rathena"));
    }

    #[test]
    fn pin_writes_both_keys() {
        let mut e = env("https://example.com/r.git", "aaa");
        let update = offered(&e, &env(REPO, "bbb")).unwrap();
        pin(&mut e, &update);
        assert_eq!(e.get("RATHENA_REPO").as_deref(), Some(REPO));
        assert_eq!(e.get("RATHENA_COMMIT").as_deref(), Some("bbb"));
        assert_eq!(offered(&e, &env(REPO, "bbb")), None);
    }

    #[test]
    fn a_hand_set_commit_is_not_offered_back() {
        let mut e = env(REPO, "aaa");
        let update = offered(&e, &env(REPO, "bbb")).unwrap();
        pin(&mut e, &update);
        e.set("RATHENA_COMMIT", "ccc");
        assert_eq!(offered(&e, &env(REPO, "bbb")), None);
        // The next app version's commit is offered again.
        assert_eq!(offered(&e, &env(REPO, "ddd")).unwrap().current, "ccc");
    }

    #[test]
    fn a_skipped_commit_is_not_offered_again() {
        let mut e = env(REPO, "aaa");
        let update = offered(&e, &env(REPO, "bbb")).unwrap();
        skip(&mut e, &update);
        assert_eq!(e.get("RATHENA_COMMIT").as_deref(), Some("aaa"));
        assert_eq!(offered(&e, &env(REPO, "bbb")), None);
    }
}
