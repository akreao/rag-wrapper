//! Prebuilt images: CI publishes the rAthena and portal images to a
//! registry (`.github/workflows/images.yml`), and the app pulls the one
//! that matches the server's settings instead of compiling it. Anything
//! without a published image (another PACKETVER, a hand-set commit, a
//! fork, no network) is built here as before.
//!
//! An image's tag names everything it was built from, so a pull never
//! gets a stale one: the rAthena commit, PACKETVER and mode, and a hash of
//! the build files bundled with the app (`docker/rathena/`, `portal/`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::envfile::EnvFile;
use crate::{host, stack};
use crate::task::Ctx;

/// `.env` key with the registry prefix images are pulled from. Empty
/// means every image is built on this computer.
pub const KEY: &str = "PREBUILT_IMAGES";

/// One image the stack needs, by its local name and the compose service that builds it.
#[derive(Debug, PartialEq, Clone)]
pub struct Image {
    pub local: String,
    pub service: &'static str,
    /// The published image to pull, when there may be one.
    pub remote: Option<String>,
    /// What it's made from, as its published tag names it:
    /// `<PACKETVER>-<mode>-<commit>-<hash>` for rAthena, `<hash>` for the
    /// portal. Empty when the build files can't be read.
    pub source: String,
}

/// The rAthena and portal images for these settings. `commit` and `repo`
/// override `.env`'s, for a server update that isn't pinned yet.
pub fn wanted(dir: &Path, env: &EnvFile, repo: &str, commit: &str) -> Vec<Image> {
    let bundled = EnvFile::parse(stack::ENV_EXAMPLE);
    let registry = env.get(KEY).or_else(|| bundled.get(KEY)).unwrap_or_default();
    let registry = registry.trim_end_matches('/');
    let packetver = env.get("PACKETVER").unwrap_or_default();
    let mode = if env.get("PRERE").as_deref() == Some("no") { "re" } else { "prere" };

    // CI builds only the fork the app is released with.
    let official = bundled.get("RATHENA_REPO").as_deref() == Some(repo);
    let short = commit.get(..12).unwrap_or(commit);
    let rathena_source = dir_hash(&dir.join("docker/rathena"))
        .map(|hash| format!("{packetver}-{mode}-{short}-{hash}"))
        .unwrap_or_default();
    let rathena = (!registry.is_empty() && official && commit.len() >= 12 && !packetver.is_empty() && !rathena_source.is_empty())
        .then(|| format!("{registry}-rathena:{rathena_source}"));
    let portal_source = dir_hash(&dir.join("portal")).unwrap_or_default();
    let portal = (!registry.is_empty() && !portal_source.is_empty()).then(|| format!("{registry}-portal:{portal_source}"));
    vec![
        Image {
            local: format!("rag-wrapper/rathena:{packetver}"),
            service: "db-init",
            remote: rathena,
            source: rathena_source,
        },
        Image {
            local: "rag-wrapper/portal".into(),
            service: "portal",
            remote: portal,
            source: portal_source,
        },
    ]
}

/// The images for the commit in `.env`.
pub fn current(dir: &Path, env: &EnvFile) -> Vec<Image> {
    let repo = env.get("RATHENA_REPO").unwrap_or_default();
    let commit = env.get("RATHENA_COMMIT").unwrap_or_default();
    wanted(dir, env, &repo, &commit)
}

/// The first 12 hex digits of
/// `find . -type f -not -path '*/.*' -printf '%P\n' | LC_ALL=C sort | xargs -d '\n' sha256sum | sha256sum`
/// run in `dir`, which is how the images workflow names the same files.
/// Dotfiles are left out: the app's bundle doesn't carry them.
pub fn dir_hash(dir: &Path) -> std::io::Result<String> {
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    files.sort();
    let mut list = String::new();
    for path in files {
        let digest = Sha256::digest(fs::read(dir.join(&path))?);
        list.push_str(&format!("{digest:x}  {path}\n"));
    }
    Ok(format!("{:x}", Sha256::digest(list))[..12].to_owned())
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir.join(prefix))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            walk(dir, &path, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub fn exists(name: &str) -> bool {
    image_id(name).is_some()
}

pub fn image_id(name: &str) -> Option<String> {
    let mut cmd = host::command("docker");
    cmd.args(["image", "inspect", "--format", "{{.Id}}", name]);
    stack::output(cmd).ok().map(|id| id.trim().to_owned())
}

/// Whether the image under the stack's name is the published one these
/// settings want. After an app update ships new build files, the wanted
/// tag changes and the image already here is out of date.
fn is_current(image: &Image) -> bool {
    match (image_id(&image.local), image.remote.as_deref().and_then(image_id)) {
        (Some(local), Some(remote)) => local == remote,
        _ => false,
    }
}

/// Pulls the published image and names it as the stack expects. Fails
/// when there is none to pull; the caller then builds it.
fn pull(ctx: &Ctx, image: &Image) -> Result<(), String> {
    let remote = image.remote.as_deref().ok_or("No prebuilt image for these settings.")?;
    ctx.line(format!("Downloading the prebuilt image {remote}"));
    let mut cmd = host::command("docker");
    cmd.args(["pull", remote]);
    if let Err(e) = ctx.run(cmd) {
        // Unless it was cancelled, Start can't replace this image by itself.
        if ctx.check_cancelled().is_ok() {
            remember(&ctx.dir, |state| {
                state.unpublished.insert(remote.to_owned());
            });
        }
        return Err(e);
    }
    let mut tag = host::command("docker");
    tag.args(["tag", remote, &image.local]);
    stack::output(tag)?;
    remember(&ctx.dir, |state| {
        state.unpublished.remove(remote);
    });
    record(&ctx.dir, std::slice::from_ref(image));
    Ok(())
}

/// Gets every image: pulled where a published one exists, built here
/// otherwise. `build` is the `docker compose` command to build with,
/// which a server update points at its new commit.
pub fn fetch_or_build(ctx: &Ctx, images: &[Image], mut build: Command) -> Result<(), String> {
    let mut to_build = Vec::new();
    for image in images {
        ctx.check_cancelled()?;
        if let Err(e) = pull(ctx, image) {
            ctx.check_cancelled()?;
            if image.remote.is_some() {
                ctx.line(format!("No prebuilt image to download ({e}); building {} on this computer.", image.local));
            }
            to_build.push(image.clone());
        }
    }
    if to_build.is_empty() {
        return Ok(());
    }
    build.args(["--progress", "plain", "build"]).args(to_build.iter().map(|i| i.service));
    ctx.run(build)?;
    record(&ctx.dir, &to_build);
    Ok(())
}

/// Before a start: pulls any published image that isn't here yet or that
/// an app update has replaced. One that can't be pulled is left for
/// `docker compose up` to build, or kept if it's already here.
pub fn fetch_missing(ctx: &Ctx, images: &[Image]) -> Result<(), String> {
    for image in images.iter().filter(|i| i.remote.is_some() && !is_current(i)) {
        ctx.check_cancelled()?;
        if pull(ctx, image).is_err() {
            ctx.check_cancelled()?;
            if exists(&image.local) {
                ctx.line(format!("No newer prebuilt image to download; keeping {}.", image.local));
            } else {
                ctx.line(format!("No prebuilt image to download; {} will be built on this computer.", image.local));
            }
        }
    }
    Ok(())
}

// ---- What each image was made from, for the "Rebuild" notice ----

/// Kept in the stack folder; only the notice reads it, so nothing breaks without it.
const STATE: &str = "images.json";

#[derive(Serialize, Deserialize, Default, Debug)]
struct State {
    /// By local name: the image the app last pulled or built under it.
    #[serde(default)]
    built: BTreeMap<String, Built>,
    /// Published tags the last try couldn't download, so Start won't
    /// replace the image by itself.
    #[serde(default)]
    unpublished: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Built {
    id: String,
    source: String,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join(STATE)
}

fn load(dir: &Path) -> State {
    fs::read(state_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Changes the record. It only feeds a notice, so failing to write it
/// never fails a task.
fn remember(dir: &Path, change: impl FnOnce(&mut State)) {
    let mut state = load(dir);
    change(&mut state);
    if let Ok(json) = serde_json::to_vec_pretty(&state) {
        let _ = fs::write(state_path(dir), json);
    }
}

/// Records what the images now here under these names were made from,
/// after they were pulled or built from the current build files.
pub fn record(dir: &Path, images: &[Image]) {
    let made: Vec<(String, Built)> = images
        .iter()
        .filter(|i| !i.source.is_empty())
        .filter_map(|i| Some((i.local.clone(), Built { id: image_id(&i.local)?, source: i.source.clone() })))
        .collect();
    if !made.is_empty() {
        remember(dir, |state| state.built.extend(made));
    }
}

/// Which images a Rebuild would replace. `needed` when the server was
/// built for other build settings or another rAthena commit than `.env`
/// names (a build that failed or was cancelled after saving, a hand-edited
/// `.env`); otherwise only the app's build files changed, and a Rebuild is
/// recommended.
#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Rebuild {
    pub needed: bool,
    pub server: bool,
    pub portal: bool,
}

/// The images here that are out of date and that Start won't replace by
/// itself (nothing published to download, or the download failed).
pub fn rebuild(dir: &Path, env: &EnvFile) -> Option<Rebuild> {
    let state = load(dir);
    let mut out = Rebuild::default();
    for image in current(dir, env) {
        let Some((id, tags)) = inspect(&image.local) else {
            continue;
        };
        if let Some(needed) = stale(&image, &id, &tags, &state) {
            out.needed |= needed;
            if image.service == "portal" {
                out.portal = true;
            } else {
                out.server = true;
            }
        }
    }
    (out.server || out.portal).then_some(out)
}

/// The local image's ID and every tag it carries.
fn inspect(name: &str) -> Option<(String, Vec<String>)> {
    let mut cmd = host::command("docker");
    cmd.args(["image", "inspect", "--format", "{{.Id}}{{range .RepoTags}} {{.}}{{end}}", name]);
    let out = stack::output(cmd).ok()?;
    let mut words = out.split_whitespace().map(str::to_owned);
    Some((words.next()?, words.collect()))
}

/// `Some(needed)` when the image with `id` and `tags` isn't made from
/// what `image` wants and only a Rebuild replaces it; `None` when it's up
/// to date, Start will replace it, or the app can't tell what it's made from.
fn stale(image: &Image, id: &str, tags: &[String], state: &State) -> Option<bool> {
    if image.source.is_empty() {
        return None;
    }
    let remote = image.remote.as_deref();
    if remote.is_some_and(|r| tags.iter().any(|t| t == r)) {
        return None;
    }
    // A pulled image carries its published tag, which names its source.
    // That covers images an older app version pulled before keeping records.
    let published = || {
        let repo = remote?.rsplit_once(':')?.0;
        tags.iter().find_map(|t| t.strip_prefix(repo)?.strip_prefix(':'))
    };
    let built = match state.built.get(&image.local).filter(|b| b.id == id) {
        Some(b) => b.source.as_str(),
        None => published()?,
    };
    if built == image.source {
        return None;
    }
    // Start downloads a newer published image (`fetch_missing`).
    if remote.is_some_and(|r| !state.unpublished.contains(r)) {
        return None;
    }
    // Everything before the build files' hash: PACKETVER, mode and commit.
    let settings = |s: &str| s.rsplit_once('-').map(|(head, _)| head.to_owned()).unwrap_or_default();
    Some(settings(built) != settings(&image.source))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rag-wrapper-images-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stack(name: &str) -> std::path::PathBuf {
        let dir = temp_dir(name);
        fs::create_dir_all(dir.join("docker/rathena")).unwrap();
        fs::create_dir_all(dir.join("portal/static")).unwrap();
        fs::write(dir.join("docker/rathena/Dockerfile"), "FROM scratch\n").unwrap();
        fs::write(dir.join("portal/main.go"), "package main\n").unwrap();
        fs::write(dir.join("portal/static/style.css"), "body {}\n").unwrap();
        dir
    }

    #[test]
    fn hashes_like_the_workflow() {
        // `printf 'FROM scratch\n' > Dockerfile; printf '#!/bin/sh\n' > entrypoint.sh`, then the
        // pipeline in dir_hash's comment.
        let dir = temp_dir("hash");
        fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
        fs::write(dir.join("entrypoint.sh"), "#!/bin/sh\n").unwrap();
        fs::write(dir.join(".dockerignore"), "Dockerfile\n").unwrap();
        assert_eq!(dir_hash(&dir).unwrap(), "872c9250a166");

        // Nested paths are relative with `/`, sorted bytewise.
        let dir = stack("nested");
        assert_eq!(dir_hash(&dir.join("portal")).unwrap(), "724f36cd9cd7");
    }

    #[test]
    fn hashes_the_build_folders_like_the_workflow() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for dir in ["docker/rathena", "portal"] {
            let out = Command::new("sh")
                .current_dir(repo.join(dir))
                .args([
                    "-c",
                    "find . -type f -not -path '*/.*' -printf '%P\\n' | LC_ALL=C sort | xargs -d '\\n' sha256sum | sha256sum | cut -c1-12",
                ])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), dir_hash(&repo.join(dir)).unwrap(), "{dir}");
        }
    }

    #[test]
    fn names_both_published_images() {
        let dir = stack("names");
        let bundled = EnvFile::parse(stack::ENV_EXAMPLE);
        let repo = bundled.get("RATHENA_REPO").unwrap();
        let env = EnvFile::parse(&format!("PACKETVER=20250716\nPRERE=no\nRATHENA_REPO={repo}\n"));
        let images = wanted(&dir, &env, &repo, "0123456789abcdef0123456789abcdef01234567");
        let rathena_hash = dir_hash(&dir.join("docker/rathena")).unwrap();
        assert_eq!(images[0].local, "rag-wrapper/rathena:20250716");
        assert_eq!(
            images[0].remote.as_deref(),
            Some(format!("ghcr.io/akreao/rag-wrapper-rathena:20250716-re-0123456789ab-{rathena_hash}").as_str())
        );
        assert_eq!(images[1].service, "portal");
        assert_eq!(images[1].remote.as_deref(), Some("ghcr.io/akreao/rag-wrapper-portal:724f36cd9cd7"));
    }

    fn rathena(source: &str, remote: bool) -> Image {
        Image {
            local: "rag-wrapper/rathena:20250716".into(),
            service: "db-init",
            remote: remote.then(|| format!("ghcr.io/akreao/rag-wrapper-rathena:{source}")),
            source: source.into(),
        }
    }

    fn built(local: &str, id: &str, source: &str) -> State {
        let mut state = State::default();
        state.built.insert(local.into(), Built { id: id.into(), source: source.into() });
        state
    }

    #[test]
    fn a_local_build_from_old_files_wants_a_rebuild() {
        let image = rathena("20250716-prere-0123456789ab-newhash", false);
        let state = built(&image.local, "sha256:1", "20250716-prere-0123456789ab-oldhash");
        assert_eq!(stale(&image, "sha256:1", &[], &state), Some(false));
        // Up to date, or another image than the one recorded: nothing to say.
        let state = built(&image.local, "sha256:1", &image.source);
        assert_eq!(stale(&image, "sha256:1", &[], &state), None);
        assert_eq!(stale(&image, "sha256:2", &[], &state), None);
    }

    #[test]
    fn other_build_settings_need_a_rebuild() {
        // Saved Renewal, but the build was cancelled.
        let image = rathena("20250716-re-0123456789ab-hash", false);
        let state = built(&image.local, "sha256:1", "20250716-prere-0123456789ab-hash");
        assert_eq!(stale(&image, "sha256:1", &[], &state), Some(true));
    }

    #[test]
    fn start_replaces_a_published_image_unless_it_could_not() {
        let image = rathena("20250716-prere-0123456789ab-newhash", true);
        // Pulled by 0.1.0, before the app kept records: its published tag says what it is.
        let tags = vec!["rag-wrapper/rathena:20250716".to_owned(), "ghcr.io/akreao/rag-wrapper-rathena:20250716-prere-0123456789ab-oldhash".to_owned()];
        let mut state = State::default();
        assert_eq!(stale(&image, "sha256:1", &tags, &state), None);
        state.unpublished.insert(image.remote.clone().unwrap());
        assert_eq!(stale(&image, "sha256:1", &tags, &state), Some(false));
        // The wanted image itself.
        let tags = vec![image.remote.clone().unwrap()];
        assert_eq!(stale(&image, "sha256:1", &tags, &state), None);
    }

    #[test]
    fn a_portal_change_is_only_recommended() {
        let image = Image { local: "rag-wrapper/portal".into(), service: "portal", remote: None, source: "newhash".into() };
        let state = built(&image.local, "sha256:1", "oldhash");
        assert_eq!(stale(&image, "sha256:1", &[], &state), Some(false));
    }

    #[test]
    fn a_fork_or_an_empty_registry_builds_here() {
        let dir = stack("off");
        let env = EnvFile::parse("PACKETVER=20250716\n");
        let images = wanted(&dir, &env, "https://example.com/rathena.git", "0123456789abcdef");
        assert_eq!(images[0].remote, None);
        assert!(images[1].remote.is_some());

        let env = EnvFile::parse(&format!("PACKETVER=20250716\n{KEY}=\n"));
        assert!(wanted(&dir, &env, "x", "0123456789abcdef").iter().all(|i| i.remote.is_none()));
    }
}
