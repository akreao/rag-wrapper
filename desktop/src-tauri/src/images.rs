//! Prebuilt images: CI publishes the rAthena and portal images to a
//! registry (`.github/workflows/images.yml`), and the app pulls the one
//! that matches the server's settings instead of compiling it. Anything
//! without a published image (another PACKETVER, a hand-set commit, a
//! fork, no network) is built here as before.
//!
//! An image's tag names everything it was built from, so a pull never
//! gets a stale one: the rAthena commit, PACKETVER and mode, and a hash of
//! the build files bundled with the app (`docker/rathena/`, `portal/`).

use std::fs;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::envfile::EnvFile;
use crate::{host, stack};
use crate::task::Ctx;

/// `.env` key with the registry prefix images are pulled from. Empty
/// means every image is built on this computer.
pub const KEY: &str = "PREBUILT_IMAGES";

/// One image the stack needs, by its local name and the compose service that builds it.
#[derive(Debug, PartialEq)]
pub struct Image {
    pub local: String,
    pub service: &'static str,
    /// The published image to pull, when there may be one.
    pub remote: Option<String>,
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
    let rathena = (!registry.is_empty() && official && commit.len() >= 12 && !packetver.is_empty())
        .then(|| dir_hash(&dir.join("docker/rathena")).ok())
        .flatten()
        .map(|hash| format!("{registry}-rathena:{packetver}-{mode}-{}-{hash}", &commit[..12]));
    let portal = (!registry.is_empty())
        .then(|| dir_hash(&dir.join("portal")).ok())
        .flatten()
        .map(|hash| format!("{registry}-portal:{hash}"));
    vec![
        Image {
            local: format!("rag-wrapper/rathena:{packetver}"),
            service: "db-init",
            remote: rathena,
        },
        Image {
            local: "rag-wrapper/portal".into(),
            service: "portal",
            remote: portal,
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

fn image_id(name: &str) -> Option<String> {
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
    ctx.run(cmd)?;
    let mut tag = host::command("docker");
    tag.args(["tag", remote, &image.local]);
    stack::output(tag).map(|_| ())
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
            to_build.push(image.service);
        }
    }
    if to_build.is_empty() {
        return Ok(());
    }
    build.args(["--progress", "plain", "build"]).args(&to_build);
    ctx.run(build)
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
