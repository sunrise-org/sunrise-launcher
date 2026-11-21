//! Pinned native Markdown and its optional same-repository images. Reading a README never loads
//! an image: each typed image is fetched only when its rendering code needs it.
use super::{
    bounded, digest,
    github::{Failure, Github, STATE},
    safe_path,
};
use crate::i18n::tr;
use image::{ImageDecoder, ImageFormat};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use url::Url;

pub(super) const MAX_README: u64 = 128 << 10;
const MAX_IMAGES: usize = 8;
const MAX_IMAGE: u64 = 2 << 20;
const MAX_EDGE: u32 = 2048;

#[derive(Clone)]
pub struct Readme {
    markdown: String,
    base: Url,
    images: Vec<ReadmeImage>,
}

impl Readme {
    pub fn markdown(&self) -> &str {
        &self.markdown
    }
    pub fn images(&self) -> &[ReadmeImage] {
        &self.images
    }

    /// Links open only on an explicit click. They are never fetched by discovery or rendering.
    pub fn link(&self, destination: &str) -> Option<String> {
        if destination.contains('\\')
            || destination.contains(char::is_control)
            || destination.starts_with("//")
        {
            return None;
        }
        let url = self.base.join(destination).ok()?;
        (url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none())
        .then(|| url.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReadmeImage {
    destination: String,
    alt: String,
    repo: String,
    sha: String,
    path: String,
    allow_fetch: bool,
}

impl ReadmeImage {
    pub fn destination(&self) -> &str {
        &self.destination
    }
    pub fn alt(&self) -> &str {
        &self.alt
    }
}

pub(super) fn markdown_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "mdown", "mkd"]
                .iter()
                .any(|kind| extension.eq_ignore_ascii_case(kind))
        })
}

fn image_path(repo: &str, sha: &str, readme_path: &str, dest: &str) -> Option<String> {
    let dest = dest.strip_prefix("./").unwrap_or(dest);
    let raw = format!("https://raw.githubusercontent.com/{repo}/{sha}/");
    let blob = format!("https://github.com/{repo}/blob/{sha}/");
    let path = if let Some(relative) = dest.strip_prefix(&raw).or_else(|| dest.strip_prefix(&blob))
    {
        relative.to_owned()
    } else if let Some(root) = dest.strip_prefix('/') {
        root.to_owned()
    } else if let Some((dir, _)) = readme_path.rsplit_once('/') {
        format!("{dir}/{dest}")
    } else {
        dest.to_owned()
    };
    // Validation precedes URL normalization: no percent escapes, protocol-relative URLs,
    // traversal, credentials, or a repository/ref from the Markdown can reach the network.
    safe_path(&path).then_some(path)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    path: String,
    markdown: String,
}

impl Saved {
    fn parse(self, repo: &str, sha: &str, allow_fetch: bool) -> Readme {
        let mut base = Url::parse("https://github.com/").expect("constant URL");
        base.path_segments_mut()
            .expect("HTTPS URL")
            .pop_if_empty()
            .extend(repo.split('/'))
            .push("blob")
            .push(sha)
            .extend(self.path.split('/'));
        let markdown = strip_html(self.markdown);
        let mut images = Vec::new();
        let mut looking = None;
        for event in parser(&markdown) {
            match event {
                Event::Start(Tag::Image { dest_url, .. }) if looking.is_none() => {
                    looking = Some((dest_url.into_string(), String::new()));
                }
                Event::Text(text) | Event::Code(text) if looking.is_some() => {
                    looking.as_mut().expect("inside image").1.push_str(&text);
                }
                Event::End(TagEnd::Image) => {
                    if let Some((destination, alt)) = looking.take() {
                        if let Some(path) = image_path(repo, sha, &self.path, &destination) {
                            images.push(ReadmeImage {
                                destination,
                                alt,
                                repo: repo.to_owned(),
                                sha: sha.to_owned(),
                                path,
                                allow_fetch,
                            });
                            if images.len() == MAX_IMAGES {
                                break;
                            }
                        }
                    }
                }
                _ => (),
            }
        }
        Readme {
            markdown,
            base,
            images,
        }
    }
}

fn parser(markdown: &str) -> Parser<'_> {
    Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    )
}

fn strip_html(markdown: String) -> String {
    let mut clean = String::with_capacity(markdown.len());
    let mut next = 0;
    for (event, span) in parser(&markdown).into_offset_iter() {
        if matches!(event, Event::Html(_) | Event::InlineHtml(_)) {
            if next <= span.start {
                clean.push_str(&markdown[next..span.start]);
            }
            next = next.max(span.end);
        }
    }
    clean.push_str(&markdown[next..]);
    clean
}

pub(super) fn load(
    github: &Github,
    repo: &str,
    sha: &str,
    offline: bool,
    read: impl FnOnce() -> Result<Option<(String, Vec<u8>)>, Failure>,
) -> Result<Option<Readme>, Failure> {
    let key = digest(format!("{repo}/{sha}").as_bytes());
    let target = github.cache.join(format!("readme-{key}.json"));
    let saved = fs::File::open(&target).ok().and_then(|file| {
        let bytes = bounded(file, MAX_README * 6 + 4096).ok()?;
        let saved: Saved = serde_json::from_slice(&bytes).ok()?;
        (safe_path(&saved.path)
            && markdown_file(&saved.path)
            && saved.markdown.len() as u64 <= MAX_README)
            .then_some(saved)
    });
    if let Some(saved) = saved {
        return Ok(Some(saved.parse(repo, sha, !offline)));
    }
    if offline {
        return Ok(None);
    }
    let Some((path, bytes)) = read()? else {
        return Ok(None);
    };
    let Ok(markdown) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    let saved = Saved { path, markdown };
    if let Ok(bytes) = serde_json::to_vec(&saved) {
        if let Err(error) = github.save(&target, &bytes) {
            eprintln!("Save pinned README: {error}");
        }
    }
    Ok(Some(saved.parse(repo, sha, true)))
}

/// Validate byte length, dimensions before decode, media type and lack of animation.
fn check_image(bytes: &[u8]) -> Result<&'static str, String> {
    let fail = || tr("README image unavailable.").to_string();
    if bytes.len() as u64 > MAX_IMAGE {
        return Err(fail());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(48 << 20);
    let format = image::guess_format(bytes).map_err(|_| fail())?;
    let suffix = match format {
        ImageFormat::Png => {
            let png =
                image::codecs::png::PngDecoder::with_limits(Cursor::new(bytes), limits.clone())
                    .map_err(|_| fail())?;
            if png.is_apng().map_err(|_| fail())? {
                return Err(fail());
            }
            "png"
        }
        ImageFormat::Jpeg => "jpg",
        ImageFormat::WebP => {
            let decoder =
                image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|_| fail())?;
            let (w, h) = decoder.dimensions();
            if w > MAX_EDGE || h > MAX_EDGE || decoder.has_animation() {
                return Err(fail());
            }
            "webp"
        }
        _ => return Err(fail()),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map_err(|_| fail())?;
    Ok(suffix)
}

pub fn readme_image(image: &ReadmeImage) -> Result<PathBuf, String> {
    let github = Github::current();
    let key = digest(format!("{}/{}/{}", image.repo, image.sha, image.path).as_bytes());
    let unavailable = || tr("README image unavailable.").to_string();
    let target = |suffix| github.cache.join(format!("image-{key}.{suffix}"));
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    for suffix in ["png", "jpg", "webp"] {
        let saved = target(suffix);
        if fs::File::open(&saved)
            .ok()
            .and_then(|file| bounded(file, MAX_IMAGE).ok())
            .is_some_and(|bytes| check_image(&bytes).ok() == Some(suffix))
        {
            return Ok(saved);
        }
    }
    if !image.allow_fetch {
        return Err(unavailable());
    }
    let bytes = github
        .raw(&mut state, &image.repo, &image.sha, &image.path, MAX_IMAGE)
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    let file = target(check_image(&bytes)?);
    github.save(&file, &bytes).map_err(|_| unavailable())?;
    Ok(file)
}

#[cfg(test)]
mod tests;
