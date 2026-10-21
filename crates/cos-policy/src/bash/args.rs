//! Expanded arguments and a small getopt-style option splitter.

/// One argument after expansion. `text` is `None` when it depends on
/// something only known when the command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arg {
    pub text: Option<String>,
    /// Has unquoted wildcards.
    pub glob: bool,
}

impl Arg {
    pub fn lit(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            glob: false,
        }
    }

    pub fn unknown() -> Self {
        Self {
            text: None,
            glob: false,
        }
    }

    pub fn is(&self, text: &str) -> bool {
        self.text.as_deref() == Some(text)
    }

    pub fn str(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// For messages.
    pub fn shown(&self) -> &str {
        self.text.as_deref().unwrap_or("an unknown path")
    }

    /// Starts with `-` and isn't just `-` (stdin).
    pub fn is_option(&self) -> bool {
        self.text
            .as_deref()
            .is_some_and(|t| t.starts_with('-') && t.len() > 1)
    }
}

/// Which options take a value.
pub(crate) struct Spec {
    /// Short options followed by a value, attached (`-n5`) or not (`-n 5`).
    pub short: &'static str,
    /// Long options followed by a value, without the leading `--`. Any long
    /// option also accepts `--name=value`.
    pub long: &'static [&'static str],
}

impl Spec {
    pub const NONE: Spec = Spec { short: "", long: &[] };
}

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    /// `-x` or `--name`, with its value.
    pub flags: Vec<(String, Option<Arg>)>,
    pub operands: Vec<Arg>,
}

impl Parsed {
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(flag, _)| flag == name)
    }

    pub fn has_any(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.has(name))
    }

    pub fn values(&self, names: &[&str]) -> Vec<&Arg> {
        self.flags
            .iter()
            .filter(|(flag, _)| names.contains(&flag.as_str()))
            .filter_map(|(_, value)| value.as_ref())
            .collect()
    }

    pub fn value(&self, names: &[&str]) -> Option<&Arg> {
        self.values(names).into_iter().next()
    }
}

/// Splits `args` (without the program name) into options and operands.
/// Options may come after operands, as GNU tools allow; `--` ends them.
pub(crate) fn parse(args: &[Arg], spec: &Spec) -> Parsed {
    let mut parsed = Parsed::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let Some(text) = arg.text.as_deref().filter(|_| arg.is_option()) else {
            parsed.operands.push(arg.clone());
            continue;
        };
        if text == "--" {
            parsed.operands.extend(rest.cloned());
            break;
        }
        if let Some(long) = text.strip_prefix("--") {
            match long.split_once('=') {
                Some((name, value)) => parsed.flags.push((format!("--{name}"), Some(Arg::lit(value)))),
                None if spec.long.contains(&long) => {
                    parsed.flags.push((text.to_owned(), rest.next().cloned()));
                }
                None => parsed.flags.push((text.to_owned(), None)),
            }
            continue;
        }
        let cluster: Vec<char> = text[1..].chars().collect();
        for (i, c) in cluster.iter().enumerate() {
            if spec.short.contains(*c) {
                let attached: String = cluster[i + 1..].iter().collect();
                let value = if attached.is_empty() {
                    rest.next().cloned()
                } else {
                    Some(Arg::lit(attached))
                };
                parsed.flags.push((format!("-{c}"), value));
                break;
            }
            parsed.flags.push((format!("-{c}"), None));
        }
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<Arg> {
        words.iter().map(|w| Arg::lit(*w)).collect()
    }

    #[test]
    fn splits_options_and_operands() {
        let spec = Spec {
            short: "nA",
            long: &["max-count"],
        };
        let parsed = parse(
            &args(&[
                "-rn5",
                "--max-count",
                "3",
                "pat",
                "--color=never",
                "-A",
                "2",
                "--",
                "-x",
            ]),
            &spec,
        );
        assert!(parsed.has("-r"));
        assert_eq!(parsed.value(&["-n"]), Some(&Arg::lit("5")));
        assert_eq!(parsed.value(&["--max-count"]), Some(&Arg::lit("3")));
        assert_eq!(parsed.value(&["--color"]), Some(&Arg::lit("never")));
        assert_eq!(parsed.value(&["-A"]), Some(&Arg::lit("2")));
        assert_eq!(parsed.operands, args(&["pat", "-x"]));
    }
}
