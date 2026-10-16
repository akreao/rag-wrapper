//! "Tell players what's new": after an app update, a draft News post for
//! players, made from the "For players" section of the release notes of
//! each version since the last one offered. The notes are bundled at build
//! time (see `build.rs`); the rest of a version's notes are for the person
//! running the server, so only that section is used.
//!
//! `.env` remembers the last app version a post was offered for
//! (`OFFERED_KEY`), set when the post is saved or skipped. A new install
//! starts on its own version, so it isn't offered one.

use serde::Serialize;

use crate::envfile::EnvFile;

include!(concat!(env!("OUT_DIR"), "/releases.rs"));

pub const OFFERED_KEY: &str = "PLAYER_NEWS_OFFERED";

/// The heading of the release notes' section for players.
const SECTION: &str = "## For players";

#[derive(Serialize, Debug, PartialEq)]
pub struct PlayerNews {
    /// The app version the post is for, recorded once it's saved or skipped.
    pub version: String,
    /// The post's Markdown: every new version's lines for players, newest first.
    pub text: String,
}

/// The post this version of the app offers, if any.
pub fn pending(env: &EnvFile) -> Option<PlayerNews> {
    draft(env.get(OFFERED_KEY).as_deref(), env!("CARGO_PKG_VERSION"), RELEASES)
}

/// Stops offering a post until the next app version.
pub fn record(env: &mut EnvFile) {
    env.set(OFFERED_KEY, env!("CARGO_PKG_VERSION"));
}

/// The lines for players of every version after `offered`, up to `current`.
/// With nothing recorded, the app was just updated from a version that
/// didn't record it, so only `current` is new.
fn draft(offered: Option<&str>, current: &str, releases: &[(&str, &str)]) -> Option<PlayerNews> {
    let now = version(current)?;
    let since = offered.and_then(version);
    let mut new: Vec<_> = releases
        .iter()
        .filter_map(|(v, notes)| Some((version(v)?, *notes)))
        .filter(|(v, _)| *v <= now && since.map_or(*v == now, |s| *v > s))
        .collect();
    new.sort_by_key(|(v, _)| std::cmp::Reverse(*v));
    let text = new.iter().map(|(_, notes)| for_players(notes)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
    (!text.is_empty()).then(|| PlayerNews { version: current.into(), text })
}

/// The text of the notes' "For players" section, up to the next `## ` heading.
fn for_players(notes: &str) -> String {
    let mut lines = notes.lines().skip_while(|l| l.trim_end() != SECTION).skip(1);
    let section: Vec<_> = lines.by_ref().take_while(|l| !l.starts_with("## ")).collect();
    section.join("\n").trim_matches('\n').trim_end().to_owned()
}

fn version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.split('.').map(|p| p.parse().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &[(&str, &str)] = &[
        ("0.9.0", "## New\n- Admin thing.\n\n## For players\n- Emotes work.\n- New warper.\n\n## Before you update\n- Nothing.\n"),
        ("0.8.1", "## Fixed\n- Admin fix only.\n"),
        ("0.8.0", "## For players\n\n- Rankings page.\n"),
        ("0.7.0", "## For players\n- Old news.\n"),
    ];

    #[test]
    fn takes_only_the_players_section() {
        assert_eq!(for_players(NOTES[0].1), "- Emotes work.\n- New warper.");
        assert_eq!(for_players(NOTES[1].1), "");
    }

    #[test]
    fn joins_every_version_since_the_last_offered_newest_first() {
        let news = draft(Some("0.7.0"), "0.9.0", NOTES).unwrap();
        assert_eq!(news.version, "0.9.0");
        assert_eq!(news.text, "- Emotes work.\n- New warper.\n- Rankings page.");
    }

    #[test]
    fn without_a_record_only_this_version_is_new() {
        assert_eq!(draft(None, "0.9.0", NOTES).unwrap().text, "- Emotes work.\n- New warper.");
    }

    #[test]
    fn nothing_once_offered_or_without_lines_for_players() {
        assert_eq!(draft(Some("0.9.0"), "0.9.0", NOTES), None);
        assert_eq!(draft(Some("0.8.0"), "0.8.1", NOTES), None);
        assert_eq!(draft(None, "0.8.1", NOTES), None);
        // Going back to an older app version offers nothing.
        assert_eq!(draft(Some("0.9.0"), "0.8.0", NOTES), None);
    }

    #[test]
    fn the_bundled_notes_are_there() {
        assert!(RELEASES.iter().any(|(v, _)| *v == "0.7.0"));
        assert!(RELEASES.iter().all(|(v, _)| version(v).is_some()));
    }

    #[test]
    fn record_stops_the_offer() {
        let mut env = EnvFile::parse("");
        record(&mut env);
        assert_eq!(env.get(OFFERED_KEY).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }
}
