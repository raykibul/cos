//! Release versions: semver as cos tags it (`v0.1.4`, `v0.2.0-rc.1`), and
//! the latest release's tag from GitHub's `releases/latest` redirect.

use std::cmp::Ordering;
use std::fmt;

/// A semantic version. Build metadata (`+…`) is dropped, as semver ignores
/// it for ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    /// The pre-release part after `-`, such as `rc.1`.
    pre: Option<String>,
}

impl Version {
    /// Reads `0.1.4`, `v0.1.4`, `0.2.0-rc.1` or `0.1.4+build`.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(version, _)| version);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, Some(pre.to_owned())),
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = core.split('.').map(|part| {
            // No leading `+`, and no empty parts.
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                part.parse::<u64>().ok()
            }
        });
        let version = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
            pre,
        };
        parts.next().is_none().then_some(version)
    }

    /// The release tag: `v0.1.4`.
    pub(crate) fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // A pre-release comes before its release.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// semver's pre-release order: dot-separated identifiers, numbers compared
/// as numbers and before words, and a shorter list first when one is a
/// prefix of the other.
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut a = a.split('.');
    let mut b = b.split('.');
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The release a `releases/latest` redirect points at, from the response's
/// headers (`curl -sI` output): the tag that ends the last `Location`, such
/// as `https://github.com/raykibul/cos/releases/tag/v0.1.5`.
pub(crate) fn tag_from_location(headers: &str) -> Option<Version> {
    let location = headers
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().split_once(':')?;
            // wget adds ` [following]` to the redirect it logs.
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().trim_end_matches("[following]").trim_end())
        })
        .next_back()?;
    let location = location.split(['?', '#']).next()?.trim_end_matches('/');
    let (_, tag) = location.rsplit_once('/')?;
    Version::parse(tag).filter(|_| tag.starts_with('v'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
    }

    #[test]
    fn parses_tags_and_plain_versions() {
        assert_eq!(v("v0.1.4"), v("0.1.4"));
        assert_eq!(v("0.1.4").to_string(), "0.1.4");
        assert_eq!(v("v0.2.0-rc.1").to_string(), "0.2.0-rc.1");
        assert_eq!(v("1.2.3+abc").to_string(), "1.2.3");
        assert_eq!(v(" 0.1.4\n").tag(), "v0.1.4");
        for bad in [
            "", "v", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "latest", "1..3", "1.2.+3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn compares_as_semver() {
        assert!(v("0.1.5") > v("0.1.4"));
        assert!(v("0.1.10") > v("0.1.9"));
        assert!(v("0.2.0") > v("0.1.99"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("v0.1.4").cmp(&v("0.1.4")), Ordering::Equal);
        assert!(v("0.2.0") > v("0.2.0-rc.1"));
        assert!(v("0.2.0-rc.2") > v("0.2.0-rc.1"));
        assert!(v("0.2.0-rc.10") > v("0.2.0-rc.9"));
        assert!(v("0.2.0-rc.1") > v("0.2.0-beta"));
        assert!(v("0.2.0-alpha.1") > v("0.2.0-alpha"));
        assert!(v("0.2.0-alpha") > v("0.2.0-1"));
        assert!(v("0.2.0-rc.1") > v("0.1.9"));
    }

    #[test]
    fn reads_the_tag_from_the_latest_redirect() {
        let github = "HTTP/2 302 \r\ndate: Fri, 23 Oct 2026 10:00:00 GMT\r\n\
                      location: https://github.com/raykibul/cos/releases/tag/v0.1.5\r\n\
                      content-length: 0\r\n\r\n";
        assert_eq!(tag_from_location(github), Some(v("0.1.5")));
        // HTTP/1.1 capitalisation, and the last of several redirects.
        let chain = "HTTP/1.1 301 Moved\nLocation: https://example.com/releases/latest\n\n\
                     HTTP/1.1 302 Found\nLocation: https://example.com/releases/tag/v0.2.0-rc.1/\n";
        assert_eq!(tag_from_location(chain), Some(v("0.2.0-rc.1")));
        // wget -S indents the headers, and logs the redirect again.
        assert_eq!(
            tag_from_location(
                "  HTTP/1.1 302 Found\nLocation: /raykibul/cos/releases/tag/v1.0.0 [following]\n"
            ),
            Some(v("1.0.0"))
        );
        assert_eq!(
            tag_from_location("  HTTP/1.1 302 Found\n  Location: /raykibul/cos/releases/tag/v1.0.0?x=1\n"),
            Some(v("1.0.0"))
        );
        // No release yet: GitHub sends you to the releases page.
        assert_eq!(
            tag_from_location("HTTP/2 302\r\nlocation: https://github.com/raykibul/cos/releases\r\n"),
            None
        );
        assert_eq!(tag_from_location("HTTP/2 404\r\n\r\n"), None);
        assert_eq!(tag_from_location("location: https://x/tag/0.1.5\n"), None);
    }
}
