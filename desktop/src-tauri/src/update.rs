//! "Server update available": each app version is released with one
//! rAthena commit, pinned in the bundled `.env.example`. The server keeps
//! running the commit in the user's `.env` until they choose to update.
//!
//! `.env` also remembers the last commit an app version offered
//! (`OFFERED_KEY`), set by updating or skipping. A commit set by hand
//! afterwards, to test a fork change, isn't offered back; the next app
//! version's commit is.
//!
//! The commit the server ran before (`PREVIOUS_KEY`) can be gone back to
//! from Settings, the way an update goes forward. Installs that haven't
//! moved since that was recorded go back to the commit the app's previous
//! release came with, which `.env.example` also names.

use serde::Serialize;

use crate::envfile::EnvFile;
use crate::stack;

pub const OFFERED_KEY: &str = "RATHENA_COMMIT_OFFERED";
pub const PREVIOUS_KEY: &str = "RATHENA_COMMIT_PREVIOUS";
pub const PREVIOUS_REPO_KEY: &str = "RATHENA_REPO_PREVIOUS";

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
    remember_current(env);
    env.set("RATHENA_REPO", &update.repo);
    env.set("RATHENA_COMMIT", &update.offered);
    skip(env, update);
}

/// Keeps the commit the server is on as the one to go back to.
fn remember_current(env: &mut EnvFile) {
    let current = env.get("RATHENA_COMMIT").unwrap_or_default();
    if !current.is_empty() {
        let repo = env.get("RATHENA_REPO").unwrap_or_default();
        env.set(PREVIOUS_KEY, &current);
        env.set(PREVIOUS_REPO_KEY, &repo);
    }
}

/// The rAthena commit the server is built from, and the one before it.
#[derive(Serialize, Debug, PartialEq)]
pub struct ServerVersion {
    pub current: String,
    pub previous: Option<Previous>,
}

/// A commit the server can go back to.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Previous {
    pub commit: String,
    pub repo: String,
    /// GitHub's page of the commits going back would undo, when there is one.
    pub changes_url: Option<String>,
}

pub fn version(env: &EnvFile) -> ServerVersion {
    let bundled = EnvFile::parse(stack::ENV_EXAMPLE);
    ServerVersion {
        current: env.get("RATHENA_COMMIT").unwrap_or_default(),
        previous: previous_from(env, &bundled),
    }
}

/// The commit "Go back" moves the server to, if there is one.
pub fn previous(env: &EnvFile) -> Option<Previous> {
    previous_from(env, &EnvFile::parse(stack::ENV_EXAMPLE))
}

fn previous_from(env: &EnvFile, bundled: &EnvFile) -> Option<Previous> {
    let current = env.get("RATHENA_COMMIT").filter(|c| !c.is_empty())?;
    let current_repo = env.get("RATHENA_REPO").unwrap_or_default();
    let recorded = env.get(PREVIOUS_KEY).filter(|c| !c.is_empty()).map(|commit| {
        let repo = env.get(PREVIOUS_REPO_KEY).filter(|r| !r.is_empty()).unwrap_or_else(|| current_repo.clone());
        (commit, repo)
    });
    // Before the app recorded one: the server is on this release's commit,
    // so the previous release's is the one before it.
    let released = || {
        let repo = bundled.get("RATHENA_REPO")?;
        let on_release = bundled.get("RATHENA_COMMIT").as_deref() == Some(current.as_str()) && repo == current_repo;
        let commit = bundled.get(PREVIOUS_KEY).filter(|c| on_release && !c.is_empty())?;
        Some((commit, repo))
    };
    let (commit, repo) = recorded.or_else(released)?;
    if commit == current && repo == current_repo {
        return None;
    }
    let changes_url = match github_repo(&repo) {
        Some(path) if repo == current_repo => Some(format!("https://github.com/{path}/compare/{commit}...{current}")),
        _ => None,
    };
    Some(Previous { commit, repo, changes_url })
}

/// Moves `.env` back to `previous`, once its image is here. The commit
/// left becomes the previous one, so going back again returns to it, and
/// this app version's commit isn't offered as an update again.
pub fn go_back(env: &mut EnvFile, previous: &Previous) {
    remember_current(env);
    env.set("RATHENA_REPO", &previous.repo);
    env.set("RATHENA_COMMIT", &previous.commit);
    if let Some(bundled) = EnvFile::parse(stack::ENV_EXAMPLE).get("RATHENA_COMMIT") {
        env.set(OFFERED_KEY, &bundled);
    }
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

    fn released(commit: &str, previous: &str) -> EnvFile {
        EnvFile::parse(&format!("RATHENA_REPO={REPO}\nRATHENA_COMMIT={commit}\n{PREVIOUS_KEY}={previous}\n"))
    }

    #[test]
    fn the_bundled_example_names_the_previous_release_commit() {
        let bundled = EnvFile::parse(stack::ENV_EXAMPLE);
        let previous = bundled.get(PREVIOUS_KEY).unwrap();
        assert_eq!(previous.len(), 40);
        assert_ne!(Some(previous), bundled.get("RATHENA_COMMIT"));
    }

    #[test]
    fn an_update_can_be_gone_back_on() {
        let mut e = env(REPO, "aaa");
        let update = offered(&e, &env(REPO, "bbb")).unwrap();
        pin(&mut e, &update);
        let previous = previous_from(&e, &env(REPO, "bbb")).unwrap();
        assert_eq!(previous.commit, "aaa");
        assert_eq!(previous.changes_url.as_deref(), Some("https://github.com/akreao/rathena/compare/aaa...bbb"));
    }

    #[test]
    fn going_back_twice_returns_to_the_newer_commit() {
        let bundled = released("bbb", "aaa");
        let mut e = env(REPO, "bbb");
        let previous = previous_from(&e, &bundled).unwrap();
        go_back(&mut e, &previous);
        assert_eq!(e.get("RATHENA_COMMIT").as_deref(), Some("aaa"));
        let next = previous_from(&e, &bundled).unwrap();
        assert_eq!(next.commit, "bbb");
        assert_eq!(next.changes_url.as_deref(), Some("https://github.com/akreao/rathena/compare/bbb...aaa"));
        go_back(&mut e, &next);
        assert_eq!(e.get("RATHENA_COMMIT").as_deref(), Some("bbb"));
        assert_eq!(previous_from(&e, &bundled).unwrap().commit, "aaa");
    }

    #[test]
    fn going_back_stops_offering_this_versions_commit() {
        let mut e = env(REPO, "bbb");
        go_back(&mut e, &Previous { commit: "aaa".into(), repo: REPO.into(), changes_url: None });
        let bundled = EnvFile::parse(stack::ENV_EXAMPLE).get("RATHENA_COMMIT").unwrap();
        assert_eq!(e.get(OFFERED_KEY), Some(bundled));
        assert_eq!(pending(&e), None);
    }

    #[test]
    fn before_any_update_the_previous_release_is_offered() {
        let bundled = released("bbb", "aaa");
        assert_eq!(previous_from(&env(REPO, "bbb"), &bundled).unwrap().commit, "aaa");
        // A hand-set commit or a fork has no known previous one.
        assert_eq!(previous_from(&env(REPO, "ccc"), &bundled), None);
        assert_eq!(previous_from(&env("https://example.com/r.git", "bbb"), &bundled), None);
    }

    #[test]
    fn a_recorded_commit_wins_over_the_previous_release() {
        let bundled = released("bbb", "aaa");
        let e = EnvFile::parse(&format!("RATHENA_REPO={REPO}\nRATHENA_COMMIT=bbb\n{PREVIOUS_KEY}=zzz\n"));
        assert_eq!(previous_from(&e, &bundled).unwrap().commit, "zzz");
    }

    #[test]
    fn going_back_to_another_repository_has_no_changes_page() {
        let mut e = env("https://example.com/r.git", "aaa");
        let update = offered(&e, &env(REPO, "bbb")).unwrap();
        pin(&mut e, &update);
        let previous = previous_from(&e, &env(REPO, "bbb")).unwrap();
        assert_eq!(previous.repo, "https://example.com/r.git");
        assert_eq!(previous.changes_url, None);
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
