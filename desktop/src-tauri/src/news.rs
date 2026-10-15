//! The News screen's posts, kept in the overrides folder's
//! `portal/news.md`. The portal reads that file on every request (see
//! `portal/players.go`), so a saved post shows on the News page and in the
//! patcher's window straight away. Each post is a `## ` heading with its
//! text under it, newest first; anything above the first one, such as a
//! `# Rules` section, is the top of the News page. The split follows the
//! portal's: heading lines inside ``` or ~~~ code blocks don't count.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where the news lives, under the overrides folder.
pub const FILE: &str = "portal/news.md";

/// The portal reads at most this much of the file (`newsMaxSize`).
const MAX_SIZE: usize = 256 << 10;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Post {
    pub title: String,
    pub body: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct News {
    /// Everything above the first post: the top of the News page.
    pub intro: String,
    /// Newest first.
    pub posts: Vec<Post>,
    /// The file as it was read, so a save can tell whether it was edited
    /// by hand in the meantime.
    #[serde(default)]
    pub text: String,
}

pub fn load(overrides: &Path) -> News {
    let text = fs::read_to_string(overrides.join(FILE)).unwrap_or_default();
    let mut news = parse(&text);
    news.text = text;
    news
}

/// Writes the posts to the news file. `expected` is the file as the editor
/// read it: when it has changed since, nothing is written, so a hand edit
/// isn't lost. Returns the news as saved.
pub fn save(overrides: &Path, news: &News) -> Result<News, String> {
    let path = overrides.join(FILE);
    let current = fs::read_to_string(&path).unwrap_or_default();
    if current != news.text {
        return Err("news.md was changed outside the app since this screen opened. Reload to see the changes, then make yours again.".into());
    }
    let text = render(news)?;
    if text.len() > MAX_SIZE {
        return Err(format!("The news is {} KB. The portal shows only the first 256 KB, so remove some older posts.", text.len() / 1024));
    }
    if text != current {
        if text.is_empty() {
            match fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
                _ => {}
            }
        } else {
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("md.tmp");
            fs::write(&tmp, &text).map_err(|e| e.to_string())?;
            fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        }
    }
    let mut saved = parse(&text);
    saved.text = text;
    Ok(saved)
}

/// Splits the file into the part above the first post and the posts.
pub fn parse(text: &str) -> News {
    let mut news = News::default();
    let mut intro = String::new();
    let mut fence = "";
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let title = if fence.is_empty() { post_title(trimmed) } else { None };
        if let Some(title) = title {
            news.posts.push(Post { title: title.into(), body: String::new() });
        } else if let Some(post) = news.posts.last_mut() {
            post.body.push_str(line);
        } else {
            intro.push_str(line);
        }
        if let Some(f) = fence_of(trimmed) {
            if fence.is_empty() {
                fence = f;
            } else if f == fence {
                fence = "";
            }
        }
    }
    news.intro = tidy(&intro);
    for post in &mut news.posts {
        post.body = tidy(&post.body);
    }
    news
}

/// The file for these posts: the intro, then each post's heading and text,
/// with a blank line between them.
fn render(news: &News) -> Result<String, String> {
    let mut parts = Vec::new();
    let intro = tidy(&news.intro);
    if !intro.is_empty() {
        if starts_post(&intro) {
            return Err("The top of the page has a line starting with ## , which would start a post. Use ### for a subheading.".into());
        }
        parts.push(intro);
    }
    for (i, post) in news.posts.iter().enumerate() {
        let title = post.title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return Err(format!("Post {} has no title.", i + 1));
        }
        let body = tidy(&post.body);
        if starts_post(&body) {
            return Err(format!("\"{title}\" has a line starting with ## , which would split it in two. Use ### for a subheading."));
        }
        parts.push(if body.is_empty() { format!("## {title}") } else { format!("## {title}\n\n{body}") });
    }
    let mut text = parts.join("\n\n");
    if !text.is_empty() {
        text.push('\n');
    }
    Ok(text)
}

/// Whether a line of this text (outside code blocks) is a `## ` heading,
/// which would start a post of its own.
fn starts_post(text: &str) -> bool {
    let mut fence = "";
    for line in text.lines() {
        let trimmed = line.trim();
        if fence.is_empty() && post_title(trimmed).is_some() {
            return true;
        }
        if let Some(f) = fence_of(trimmed) {
            if fence.is_empty() {
                fence = f;
            } else if f == fence {
                fence = "";
            }
        }
    }
    false
}

/// The title of a `## ` heading line.
fn post_title(line: &str) -> Option<&str> {
    if line == "##" {
        return Some("");
    }
    line.strip_prefix("## ").map(str::trim)
}

fn fence_of(line: &str) -> Option<&'static str> {
    ["```", "~~~"].into_iter().find(|f| line.starts_with(f))
}

/// The text without blank lines or spaces at its end or blank lines at its
/// start, with Windows line ends made plain.
fn tidy(text: &str) -> String {
    text.replace("\r\n", "\n").trim_matches('\n').trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("news-test-{}-{}", std::process::id(), rand_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    #[test]
    fn parses_like_the_portal() {
        let text = "# Rules\n\nBe nice.\n\n## Maintenance tonight\n\nDown at 8pm.\n\n```\n## not a post\n```\n\n## Older\r\nold\n";
        let news = parse(text);
        assert_eq!(news.intro, "# Rules\n\nBe nice.");
        assert_eq!(news.posts.len(), 2);
        assert_eq!(news.posts[0].title, "Maintenance tonight");
        assert_eq!(news.posts[0].body, "Down at 8pm.\n\n```\n## not a post\n```");
        assert_eq!(news.posts[1], Post { title: "Older".into(), body: "old".into() });
    }

    #[test]
    fn saving_unchanged_news_keeps_the_file() {
        let text = "# Rules\n\nBe nice.\n\n## New\n\nText\n\n- a\n- b\n\n## Old\n\nold\n";
        let mut news = parse(text);
        news.text = text.into();
        assert_eq!(render(&news).unwrap(), text);
    }

    #[test]
    fn save_writes_a_new_post_on_top() {
        let dir = dir();
        let mut news = load(&dir);
        assert!(news.posts.is_empty() && news.text.is_empty());
        news.posts.insert(0, Post { title: "  First\npost ".into(), body: "Hello\r\n\n".into() });
        let saved = save(&dir, &news).unwrap();
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), "## First post\n\nHello\n");

        let mut news = saved;
        news.posts.insert(0, Post { title: "Second".into(), body: String::new() });
        save(&dir, &news).unwrap();
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), "## Second\n\n## First post\n\nHello\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_every_post_removes_the_file() {
        let dir = dir();
        let mut news = News { posts: vec![Post { title: "Only".into(), body: "x".into() }], ..News::default() };
        news = save(&dir, &news).unwrap();
        news.posts.clear();
        save(&dir, &news).unwrap();
        assert!(!dir.join(FILE).exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_refuses_after_a_hand_edit() {
        let dir = dir();
        let news = save(&dir, &News { posts: vec![Post { title: "A".into(), body: String::new() }], ..News::default() }).unwrap();
        fs::write(dir.join(FILE), "## By hand\n").unwrap();
        assert!(save(&dir, &news).unwrap_err().contains("changed outside the app"));
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), "## By hand\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refuses_headings_that_would_split_a_post() {
        let post = |body: &str| News { posts: vec![Post { title: "A".into(), body: body.into() }], ..News::default() };
        assert!(render(&post("text\n## Sub")).is_err());
        assert!(render(&post("# Archive")).is_ok());
        assert!(render(&post("### Sub\n```\n## code\n```")).is_ok());
        assert!(render(&post("#hashtag")).is_ok());
        assert!(render(&News { posts: vec![Post::default()], ..News::default() }).unwrap_err().contains("no title"));
        assert!(render(&News { intro: "## Rules".into(), ..News::default() }).is_err());
    }
}
