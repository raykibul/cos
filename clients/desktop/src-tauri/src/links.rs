//! Links leave the app: a web or mail link opens in the default browser (or
//! mail app), never inside the window. The UI hands the links it sees
//! clicked to `open_url`; this plugin catches any other navigation away from
//! the app's own page.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};

/// `url`, parsed, if it's a web (http, https) or mail link.
pub(crate) fn external(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    matches!(url.scheme(), "http" | "https" | "mailto").then_some(url)
}

/// Stops the window leaving the app for a web or mail link, and opens the
/// link outside instead.
pub(crate) fn plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("links")
        .on_navigation(|webview, url| {
            let dev_url = if tauri::is_dev() {
                webview.config().build.dev_url.clone()
            } else {
                None
            };
            if stays_inside(url, dev_url.as_ref()) {
                return true;
            }
            let url = url.to_string();
            // Off the main thread, which is busy deciding this navigation.
            std::thread::spawn(move || {
                if let Err(error) = crate::commands::open_url(url.clone()) {
                    eprintln!("cos: could not open {url}: {}", error.message);
                }
            });
            false
        })
        .build()
}

/// Whether the window may go to `url`: the app's own page (`tauri://`, or
/// Vite's dev server in a dev build), or anything that isn't a web or mail
/// link, as before.
fn stays_inside(url: &Url, dev_url: Option<&Url>) -> bool {
    match url.scheme() {
        "http" | "https" => {
            url.host_str() == Some("tauri.localhost")
                || dev_url.is_some_and(|dev| dev.origin() == url.origin())
        }
        "mailto" => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_web_and_mail_links_are_external() {
        assert!(external("https://github.com/raykibul/cos").is_some());
        assert!(external("http://localhost:3000/").is_some());
        assert!(external("mailto:someone@example.com").is_some());
        assert!(external("file:///etc/hosts").is_none());
        assert!(external("javascript:alert(1)").is_none());
        assert!(external("/relative/path").is_none());
        assert!(external("-a Calculator").is_none());
    }

    #[test]
    fn the_app_stays_inside_and_the_web_goes_out() {
        let dev = url("http://localhost:5173");
        assert!(stays_inside(&url("tauri://localhost/index.html"), None));
        assert!(stays_inside(&url("http://tauri.localhost/"), None));
        assert!(stays_inside(&url("about:blank"), None));
        assert!(stays_inside(
            &url("http://localhost:5173/?page=waiting"),
            Some(&dev)
        ));

        assert!(!stays_inside(&url("https://github.com/raykibul/cos"), Some(&dev)));
        assert!(!stays_inside(&url("http://localhost:3000/"), Some(&dev)));
        assert!(!stays_inside(&url("http://localhost:5173/"), None));
        assert!(!stays_inside(&url("mailto:someone@example.com"), None));
    }
}
