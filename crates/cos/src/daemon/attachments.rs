//! Files you attach to a message, from any device. The app (or `cos … say
//! --file`) uploads each file to the daemon, which saves it in the folder the
//! agent works in, under `.cos/attachments/`, so Claude can read it without
//! asking for a path outside its worktree. The message then names the files
//! in a block cos appends ([`with_attachments`]), and the transcript reads
//! that block back as `attachments` ([`split`]). The contract is
//! "Attachments" in `docs/agents-api.md`.
//!
//! The folder holds a `.gitignore` of `*`, so attachments never show in
//! `git status`, never get committed, and don't stop a workspace from being
//! removed.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::api::ApiError;

/// Where attachments go, inside an agent's folder.
pub(crate) const DIR: &str = ".cos/attachments";
/// The largest file cos takes: Claude reads images and PDFs well below this.
pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
/// How many files one message may carry.
pub(crate) const MAX_PER_MESSAGE: usize = 20;
/// The longest stored file name, in characters.
const MAX_NAME_CHARS: usize = 100;
/// The first line of the block cos appends to a message.
const HEADER: &str = "Attached files (saved by cos; read them with the Read tool):";

/// A saved file. `name` is how messages refer to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Attachment {
    /// The file's name in the folder, unique there.
    pub name: String,
    /// Its absolute path, which the agent reads.
    pub path: PathBuf,
    pub size: u64,
    pub content_type: String,
}

/// An attachment as the transcript shows it, read back from the block cos
/// appended to a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Attached {
    pub name: String,
    pub path: String,
    pub content_type: String,
    /// Its size as the message gives it: `245 KB`.
    pub size_label: String,
}

/// The attachments folder of an agent working in `folder`.
pub(crate) fn dir(folder: &Path) -> PathBuf {
    folder.join(DIR)
}

/// Saves `bytes` as `name` in `folder`'s attachments, renaming it
/// (`photo-2.png`) if the name is taken. `content_type` is what the upload
/// said, used only to give a name without an extension one.
pub(crate) fn save(
    folder: &Path,
    name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<Attachment, ApiError> {
    if bytes.len() > MAX_BYTES {
        return Err(too_large());
    }
    let dir = prepare(folder)?;
    let name = file_name(name, content_type);
    let (stem, extension) = split_extension(&name);
    for n in 1..10_000 {
        let candidate = match n {
            1 => name.clone(),
            n => format!("{stem}-{n}{extension}"),
        };
        let path = dir.join(&candidate);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path);
        match file {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes).map_err(|error| {
                    let _ = std::fs::remove_file(&path);
                    failed(&format!("could not save {candidate}: {error}"))
                })?;
                return Ok(Attachment {
                    content_type: content_type_of(&candidate).to_owned(),
                    name: candidate,
                    path,
                    size: bytes.len() as u64,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(failed(&format!("could not save {candidate}: {error}"))),
        }
    }
    Err(failed(&format!("there are too many files named like {name}")))
}

/// The attachment called `name` in `folder`'s attachments. `name` may also
/// be the absolute path a message gave, as long as it's in that folder.
pub(crate) fn find(folder: &Path, name: &str) -> Result<Attachment, ApiError> {
    let dir = dir(folder);
    let name = name.trim();
    let name = match Path::new(name).strip_prefix(&dir) {
        Ok(rest) => rest.to_str().unwrap_or_default(),
        Err(_) => name,
    };
    let missing = || ApiError::not_found(&format!("there is no attachment {name}"));
    if !is_plain_name(name) {
        return Err(missing());
    }
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path).map_err(|_| missing())?;
    if !meta.is_file() {
        return Err(missing());
    }
    Ok(Attachment {
        name: name.to_owned(),
        content_type: content_type_of(name).to_owned(),
        path,
        size: meta.len(),
    })
}

/// [`find`] for each of a message's attachments, in order, without repeats.
pub(crate) fn find_all(folder: &Path, names: &[String]) -> Result<Vec<Attachment>, ApiError> {
    if names.len() > MAX_PER_MESSAGE {
        return Err(bad(&format!(
            "a message can carry at most {MAX_PER_MESSAGE} files"
        )));
    }
    let mut found: Vec<Attachment> = Vec::with_capacity(names.len());
    for name in names {
        let attachment = find(folder, name)?;
        if !found.iter().any(|seen| seen.name == attachment.name) {
            found.push(attachment);
        }
    }
    Ok(found)
}

/// Copies an attachment into another folder's attachments, as when the
/// Chief of Staff hands one to an agent it starts.
pub(crate) fn copy_into(attachment: &Attachment, folder: &Path) -> Result<Attachment, ApiError> {
    if attachment.path.parent() == Some(dir(folder).as_path()) {
        return Ok(attachment.clone());
    }
    let bytes = std::fs::read(&attachment.path)
        .map_err(|error| failed(&format!("could not read {}: {error}", attachment.name)))?;
    // Handed over before: the same file is there already.
    if let Ok(there) = find(folder, &attachment.name)
        && there.size == attachment.size
        && std::fs::read(&there.path).is_ok_and(|existing| existing == bytes)
    {
        return Ok(there);
    }
    save(folder, &attachment.name, None, &bytes)
}

/// `text` with the block that names `attachments` appended. No attachments
/// leaves it as it is.
pub(crate) fn with_attachments(text: &str, attachments: &[Attachment]) -> String {
    if attachments.is_empty() {
        return text.to_owned();
    }
    let mut out = text.trim_end().to_owned();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(HEADER);
    for attachment in attachments {
        out.push_str(&format!(
            "\n- {} ({}, {})",
            attachment.path.display(),
            attachment.content_type,
            size_label(attachment.size)
        ));
    }
    out
}

/// A message's text without cos's attachments block, and what the block
/// named. Text without one comes back as it is.
pub(crate) fn split(text: &str) -> (String, Vec<Attached>) {
    let start = if text.starts_with(HEADER) {
        Some(0)
    } else {
        text.rfind(&format!("\n{HEADER}")).map(|at| at + 1)
    };
    let Some(start) = start else {
        return (text.to_owned(), Vec::new());
    };
    let mut attached = Vec::new();
    for line in text[start + HEADER.len()..].lines().skip(1) {
        match parse_line(line) {
            Some(file) => attached.push(file),
            // Something else follows: not cos's block after all.
            None if !line.trim().is_empty() => return (text.to_owned(), Vec::new()),
            None => {}
        }
    }
    if attached.is_empty() {
        return (text.to_owned(), Vec::new());
    }
    (text[..start].trim_end().to_owned(), attached)
}

/// `- /path/to/photo.png (image/png, 245 KB)`.
fn parse_line(line: &str) -> Option<Attached> {
    let rest = line.strip_prefix("- ")?.strip_suffix(')')?;
    let (path, details) = rest.rsplit_once(" (")?;
    let (content_type, size_label) = details.split_once(", ")?;
    let name = Path::new(path).file_name()?.to_str()?.to_owned();
    Some(Attached {
        name,
        path: path.to_owned(),
        content_type: content_type.to_owned(),
        size_label: size_label.to_owned(),
    })
}

/// Makes the attachments folder, with its `.gitignore`. Refuses a folder
/// that's a symlink, so a repo can't point uploads somewhere else.
fn prepare(folder: &Path) -> Result<PathBuf, ApiError> {
    if !folder.is_dir() {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            message: format!("the agent's folder {} is gone", folder.display()),
        });
    }
    let mut dir = folder.to_path_buf();
    for part in Path::new(DIR).components() {
        dir.push(part);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(failed(&format!(
                    "{} is not a plain folder, so cos won't save files there",
                    dir.display()
                )));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                std::fs::create_dir(&dir)
                    .map_err(|error| failed(&format!("could not make {}: {error}", dir.display())))?;
            }
            Err(error) => return Err(failed(&format!("could not open {}: {error}", dir.display()))),
        }
    }
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "# Files attached through cos; never committed.\n*\n")
            .map_err(|error| failed(&format!("could not write {}: {error}", ignore.display())))?;
    }
    Ok(dir)
}

/// A safe file name from what the upload called it: the last path segment,
/// with letters, digits, `.`, `-` and `_` kept, spaces and anything else
/// made `-`, no leading dot, at most [`MAX_NAME_CHARS`], and an extension
/// from the content type when it has none.
pub(crate) fn file_name(name: &str, content_type: Option<&str>) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut clean = String::with_capacity(base.len());
    for c in base.trim().chars() {
        let keep = c.is_alphanumeric() || matches!(c, '.' | '-' | '_');
        let c = if keep { c } else { '-' };
        // One dash for a run of them.
        if !(c == '-' && clean.ends_with('-')) {
            clean.push(c);
        }
    }
    let clean = clean.trim_start_matches(['.', '-']).trim_end_matches(['.', '-']);
    let mut clean = if clean.is_empty() {
        "file".to_owned()
    } else {
        clean.to_owned()
    };
    if split_extension(&clean).1.is_empty()
        && let Some(extension) = content_type.and_then(extension_for)
    {
        clean.push('.');
        clean.push_str(extension);
    }
    if clean.chars().count() > MAX_NAME_CHARS {
        let (stem, extension) = split_extension(&clean);
        let keep = MAX_NAME_CHARS.saturating_sub(extension.chars().count()).max(1);
        clean = stem.chars().take(keep).collect::<String>() + extension;
    }
    clean
}

/// `("photo", ".png")`; `("Makefile", "")`.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 && at < name.len() - 1 && name.len() - at <= 10 => name.split_at(at),
        _ => (name, ""),
    }
}

/// A name [`file_name`] could have made: one segment, no leading dot.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && name.chars().count() <= MAX_NAME_CHARS + 8
}

/// Common types by extension; anything else is `application/octet-stream`.
const TYPES: [(&str, &str); 30] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("heic", "image/heic"),
    ("svg", "image/svg+xml"),
    ("pdf", "application/pdf"),
    ("txt", "text/plain"),
    ("log", "text/plain"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("tsv", "text/tab-separated-values"),
    ("json", "application/json"),
    ("yaml", "application/yaml"),
    ("yml", "application/yaml"),
    ("toml", "application/toml"),
    ("xml", "application/xml"),
    ("html", "text/html"),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("ts", "text/plain"),
    ("rs", "text/plain"),
    ("swift", "text/plain"),
    ("py", "text/plain"),
    ("sh", "text/plain"),
    ("zip", "application/zip"),
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
    ("diff", "text/plain"),
];

pub(crate) fn content_type_of(name: &str) -> &'static str {
    let extension = split_extension(name)
        .1
        .trim_start_matches('.')
        .to_ascii_lowercase();
    TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map_or("application/octet-stream", |(_, kind)| kind)
}

/// The extension for a content type, for a name that has none.
fn extension_for(content_type: &str) -> Option<&'static str> {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if content_type == "text/plain" {
        return Some("txt");
    }
    TYPES
        .iter()
        .find(|(_, kind)| *kind == content_type)
        .map(|(extension, _)| *extension)
}

/// `812 B`, `245 KB`, `3.4 MB`.
pub(crate) fn size_label(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    match bytes {
        b if b < KB => format!("{b} B"),
        b if b < MB => format!("{} KB", b.div_ceil(KB)),
        b => format!("{:.1} MB", b as f64 / MB as f64),
    }
}

fn too_large() -> ApiError {
    ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        message: format!("a file can be at most {} MB", MAX_BYTES / 1024 / 1024),
    }
}

fn bad(message: &str) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: message.to_owned(),
    }
}

fn failed(message: &str) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests;
