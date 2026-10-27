//! Forwards of ports below 1024. macOS lets only root listen on a loopback
//! port below 1024, so a forward of `:443` listens on `:8443` and pf
//! redirects this Mac's `localhost:443` there. Links and OAuth redirects
//! that name `https://localhost/` then reach it.
//!
//! [`Redirects`] is the table of those redirects, shared by every
//! [`crate::ForwardSet`]; each change hands the whole table to a
//! [`Redirector`]. The desktop app's runs a root helper (itself, with
//! `--pf-redirect`), started once behind macOS's password prompt. The
//! helper speaks the line protocol in [`serve`] over a Unix socket and
//! removes every rule when the app goes away, so nothing outlives it.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The pf anchor the rules go in. `/etc/pf.conf` evaluates `com.apple/*`,
/// so a sub-anchor there runs without touching Apple's rules.
pub const ANCHOR: &str = "com.apple/cos";
/// How far up a redirected forward listens: 443 on 8443, 80 on 8080.
pub const OFFSET: u16 = 8000;
/// The first port that needs no redirect.
pub const UNPRIVILEGED: u16 = 1024;

/// Loopback port → the port it is redirected to.
pub type Table = BTreeMap<u16, u16>;

/// Makes a table the whole set of redirects on this Mac.
pub trait Redirector: Send + Sync + 'static {
    /// May block, on a password prompt; an error says why in words.
    fn apply(&self, table: &Table) -> Result<(), String>;
}

/// The redirects in use, shared by every forward set.
#[derive(Clone)]
pub struct Redirects(Arc<Inner>);

struct Inner {
    redirector: Box<dyn Redirector>,
    /// Port → (the id of the redirect that owns it, where it goes).
    table: Mutex<BTreeMap<u16, (u64, u16)>>,
    next_id: AtomicU64,
}

impl Redirects {
    pub fn new(redirector: impl Redirector) -> Self {
        Self(Arc::new(Inner {
            redirector: Box::new(redirector),
            table: Mutex::default(),
            next_id: AtomicU64::new(0),
        }))
    }

    /// The redirects now.
    pub fn table(&self) -> Table {
        plain(&self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<u16, (u64, u16)>> {
        self.0.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Redirects `from` to `to` until the [`Redirect`] is dropped. A newer
    /// redirect of the same port (another host's forward) takes it over.
    /// Blocks while the redirector does.
    pub(crate) fn add(&self, from: u16, to: u16) -> Result<Redirect, String> {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut table = self.lock();
        let previous = table.insert(from, (id, to));
        if let Err(error) = self.0.redirector.apply(&plain(&table)) {
            match previous {
                Some(previous) => table.insert(from, previous),
                None => table.remove(&from),
            };
            return Err(error);
        }
        Ok(Redirect {
            redirects: self.clone(),
            from,
            id,
        })
    }
}

fn plain(table: &BTreeMap<u16, (u64, u16)>) -> Table {
    table.iter().map(|(from, (_, to))| (*from, *to)).collect()
}

/// One redirect, removed when dropped unless a newer one took its port.
pub(crate) struct Redirect {
    redirects: Redirects,
    from: u16,
    id: u64,
}

impl Drop for Redirect {
    fn drop(&mut self) {
        let mut table = self.redirects.lock();
        if table.get(&self.from).is_some_and(|(id, _)| *id == self.id) {
            table.remove(&self.from);
            // Nothing to tell anyone if it fails: the forward is gone anyway.
            let _ = self.redirects.0.redirector.apply(&plain(&table));
        }
    }
}

/// The pf rules for `table`: this Mac's loopback, v4 and v6, and nothing
/// else.
pub fn rules(table: &Table) -> String {
    let mut text = String::new();
    for (from, to) in table {
        text.push_str(&format!(
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port {from} -> 127.0.0.1 port {to}\n\
             rdr pass on lo0 inet6 proto tcp from any to ::1 port {from} -> ::1 port {to}\n"
        ));
    }
    text
}

/// A table as one line of the protocol: `443:8443 80:8080`, or empty.
pub fn format_table(table: &Table) -> String {
    table
        .iter()
        .map(|(from, to)| format!("{from}:{to}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reads a line of the protocol. The helper runs as root, so it takes only
/// what a forward needs: a port below 1024 to one at or above it.
pub fn parse_table(line: &str) -> Result<Table, String> {
    let mut table = Table::new();
    for pair in line.split_whitespace() {
        let parsed = pair
            .split_once(':')
            .and_then(|(from, to)| Some((from.parse::<u16>().ok()?, to.parse::<u16>().ok()?)));
        match parsed {
            Some((from, to)) if (1..UNPRIVILEGED).contains(&from) && to >= UNPRIVILEGED => {
                table.insert(from, to);
            }
            _ => return Err(format!("not a redirect: {pair}")),
        }
    }
    Ok(table)
}

/// What the helper says back: `ready` or `ok`, or `error <why>`.
pub fn read_reply(line: &str) -> Result<(), String> {
    match line.trim_end() {
        "ok" | "ready" => Ok(()),
        reply => Err(reply.strip_prefix("error ").unwrap_or(reply).to_owned()),
    }
}

/// pf, as the helper drives it.
pub trait Pf {
    /// Turns pf on for as long as the helper runs.
    fn enable(&mut self) -> Result<(), String>;
    /// Makes `rules` the anchor's rules; empty removes them all.
    fn load(&mut self, rules: &str) -> Result<(), String>;
    /// Lets pf go back to how it was.
    fn disable(&mut self);
}

/// The helper's side: `ready` once pf is on, then for each line a table,
/// answered `ok` or `error <why>`. When the app closes its end, or quits
/// or crashes, every rule goes and pf is let go.
pub fn serve(input: impl BufRead, mut output: impl Write, pf: &mut impl Pf) {
    if let Err(error) = pf.enable() {
        let _ = writeln!(output, "error {error}");
        return;
    }
    if writeln!(output, "ready").and_then(|()| output.flush()).is_ok() {
        for line in input.lines() {
            let Ok(line) = line else { break };
            let reply = match parse_table(&line).and_then(|table| pf.load(&rules(&table))) {
                Ok(()) => "ok".to_owned(),
                Err(error) => format!("error {}", error.replace('\n', " ")),
            };
            if writeln!(output, "{reply}").and_then(|()| output.flush()).is_err() {
                break;
            }
        }
    }
    let _ = pf.load("");
    pf.disable();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records each table, and fails while `fail` is set.
    #[derive(Clone, Default)]
    struct Recorder {
        tables: Arc<Mutex<Vec<Table>>>,
        fail: Arc<Mutex<bool>>,
    }

    impl Redirector for Recorder {
        fn apply(&self, table: &Table) -> Result<(), String> {
            if *self.fail.lock().unwrap() {
                return Err("You cancelled the password prompt.".into());
            }
            self.tables.lock().unwrap().push(table.clone());
            Ok(())
        }
    }

    #[test]
    fn a_redirect_lasts_until_dropped_and_a_newer_one_takes_its_port() {
        let recorder = Recorder::default();
        let redirects = Redirects::new(recorder.clone());
        let https = redirects.add(443, 8443).unwrap();
        let http = redirects.add(80, 8080).unwrap();
        assert_eq!(redirects.table(), Table::from([(80, 8080), (443, 8443)]));
        drop(http);
        assert_eq!(redirects.table(), Table::from([(443, 8443)]));

        // Another host's forward of 443 takes it; the old one's drop leaves it.
        let newer = redirects.add(443, 8444).unwrap();
        drop(https);
        assert_eq!(redirects.table(), Table::from([(443, 8444)]));
        drop(newer);
        assert!(redirects.table().is_empty());
        assert_eq!(recorder.tables.lock().unwrap().last(), Some(&Table::new()));

        // A failure changes nothing.
        *recorder.fail.lock().unwrap() = true;
        let error = redirects.add(443, 8443).err().unwrap();
        assert_eq!(error, "You cancelled the password prompt.");
        assert!(redirects.table().is_empty());
    }

    #[test]
    fn tables_go_over_one_line_and_only_low_ports_to_high_ones_pass() {
        let table = Table::from([(80, 8080), (443, 8443)]);
        assert_eq!(format_table(&table), "80:8080 443:8443");
        assert_eq!(parse_table("80:8080 443:8443").unwrap(), table);
        assert_eq!(parse_table("").unwrap(), Table::new());
        for bad in ["443", "443:80", "8443:9443", "0:8000", "443:x", "443:8443;pass"] {
            assert_eq!(parse_table(bad).unwrap_err(), format!("not a redirect: {bad}"));
        }
        assert_eq!(
            rules(&Table::from([(443, 8443)])),
            "rdr pass on lo0 inet proto tcp from any to 127.0.0.1 port 443 -> 127.0.0.1 port 8443\n\
             rdr pass on lo0 inet6 proto tcp from any to ::1 port 443 -> ::1 port 8443\n"
        );
        assert_eq!(rules(&Table::new()), "");
        assert_eq!(read_reply("ok\n"), Ok(()));
        assert_eq!(read_reply("ready"), Ok(()));
        assert_eq!(read_reply("error pfctl: no\n"), Err("pfctl: no".into()));
    }

    #[derive(Default)]
    struct FakePf {
        log: Vec<String>,
        refuse: bool,
    }

    impl Pf for FakePf {
        fn enable(&mut self) -> Result<(), String> {
            if self.refuse {
                return Err("pfctl: Operation not permitted".into());
            }
            self.log.push("enable".into());
            Ok(())
        }
        fn load(&mut self, rules: &str) -> Result<(), String> {
            self.log.push(format!("load {}", rules.lines().count()));
            Ok(())
        }
        fn disable(&mut self) {
            self.log.push("disable".into());
        }
    }

    #[test]
    fn the_helper_applies_each_table_and_cleans_up_when_the_app_goes() {
        let mut pf = FakePf::default();
        let mut output = Vec::new();
        serve(
            &b"443:8443\n22:2222 443:8443\nrm -rf /\n\n"[..],
            &mut output,
            &mut pf,
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "ready\nok\nok\nerror not a redirect: rm\nok\n"
        );
        // The app went (end of input): every rule goes, then pf is let go.
        assert_eq!(
            pf.log,
            ["enable", "load 2", "load 4", "load 0", "load 0", "disable"]
        );

        let mut pf = FakePf {
            refuse: true,
            ..FakePf::default()
        };
        let mut output = Vec::new();
        serve(&b"443:8443\n"[..], &mut output, &mut pf);
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "error pfctl: Operation not permitted\n"
        );
        assert!(pf.log.is_empty());
    }
}
