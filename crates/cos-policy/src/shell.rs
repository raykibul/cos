//! A parser for the shell commands agents run, just deep enough to classify
//! them: it splits lists and pipelines, keeps quoting, finds every command
//! substitution, redirection and heredoc, and refuses (with [`Unparseable`])
//! anything it can't follow, such as `case`, function definitions or arrays.
//! Refusing sends the request to you, so the parser never has to guess.

use std::fmt;

/// Longest command text worth parsing. Anything longer goes to you.
const MAX_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unparseable(pub String);

impl fmt::Display for Unparseable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

type Result<T> = std::result::Result<T, Unparseable>;

fn fail<T>(why: impl Into<String>) -> Result<T> {
    Err(Unparseable(why.into()))
}

/// A list of commands, in the order they run.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Script {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    pub node: Node,
    /// Its stdin is the previous item's stdout.
    pub piped_in: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Command(Command),
    /// `( … )`: runs in a subshell, so a `cd` inside doesn't leak out.
    Subshell(Script),
    /// `for NAME in WORDS`: binds NAME to each word in turn. `None` for
    /// `for NAME` without `in`, which loops over unknown arguments.
    Bind {
        name: String,
        values: Option<Vec<Word>>,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Command {
    /// `NAME=value` words before the command name.
    pub assigns: Vec<Assign>,
    pub words: Vec<Word>,
    pub redirs: Vec<Redirect>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Assign {
    pub name: String,
    pub value: Word,
}

/// One shell word after quote removal.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Word {
    pub parts: Vec<Part>,
    /// Has unquoted wildcards or braces, so it may expand to other names.
    pub glob: bool,
    /// Has any quoting or escaping.
    pub quoted: bool,
    /// Command and process substitutions inside it, as script text.
    pub substs: Vec<String>,
    /// Every variable it reads, including ones inside `${…}` operators.
    pub vars: Vec<String>,
    /// Set when the word is `NAME=value` with an unquoted name: the name and
    /// the index of the first part of the value.
    assign: Option<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Part {
    Lit(String),
    /// A plain `$NAME` or `${NAME}`.
    Var(String),
    /// Something only known when the command runs: `$(…)`, `$1`, `${X:-y}`.
    Dynamic,
}

impl Word {
    /// The word's text when it has no expansions.
    pub fn literal(&self) -> Option<String> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Lit(text) => out.push_str(text),
                _ => return None,
            }
        }
        Some(out)
    }

    /// The literal text of an unquoted word, used for reserved words.
    fn bare(&self) -> Option<String> {
        if self.quoted { None } else { self.literal() }
    }

    fn push_lit(&mut self, c: char) {
        match self.parts.last_mut() {
            Some(Part::Lit(text)) => text.push(c),
            _ => self.parts.push(Part::Lit(c.to_string())),
        }
    }

    fn is_empty(&self) -> bool {
        self.parts.is_empty() && !self.quoted
    }

    fn into_assign(self) -> std::result::Result<Assign, Word> {
        let Some((name, start)) = self.assign.clone() else {
            return Err(self);
        };
        let mut parts = self.parts;
        let mut value_parts = parts.split_off(start);
        // The `=` ends the literal before `start`; anything after it in that
        // literal belongs to the value.
        if let Some(Part::Lit(text)) = parts.last()
            && let Some(rest) = text.split_once('=').map(|(_, rest)| rest.to_owned())
            && !rest.is_empty()
        {
            value_parts.insert(0, Part::Lit(rest));
        }
        Ok(Assign {
            name,
            value: Word {
                parts: value_parts,
                glob: false,
                quoted: self.quoted,
                substs: self.substs,
                vars: self.vars,
                assign: None,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Redirect {
    /// The file descriptor before the operator, as in `2>`.
    pub fd: Option<u32>,
    pub op: RedirOp,
    /// The file, the fd to duplicate, the here-string, or the heredoc's
    /// delimiter.
    pub target: Word,
    pub heredoc: Option<Heredoc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedirOp {
    /// `<`
    In,
    /// `>`, `>|`, `&>`
    Out,
    /// `>>`, `&>>`
    Append,
    /// `<>`
    ReadWrite,
    /// `<&` and `>&`: duplicate a descriptor, or close it with `-`.
    Dup,
    /// `<<` and `<<-`
    HereDoc,
    /// `<<<`
    HereString,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Heredoc {
    pub body: String,
    /// The delimiter was unquoted, so the shell expands `$…` in the body.
    pub expands: bool,
    /// What the expansion would run or read; empty when `expands` is false.
    pub expansions: Word,
}

/// Parses `text` as a list of commands.
pub(crate) fn parse(text: &str) -> Result<Script> {
    if text.len() > MAX_LEN {
        return fail("the command is too long to check");
    }
    let tokens = Lexer::new(text).run()?;
    let mut parser = Parser {
        tokens: tokens.into_iter().peekable(),
    };
    parser.list(false)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(Word),
    Op(Op),
    Redir(Redirect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    And,
    Or,
    Semi,
    Background,
    Pipe,
    Newline,
    Open,
    Close,
}

/// Words that start or end a compound command and run nothing themselves.
const SKIPPED_RESERVED: &[&str] = &[
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "{", "}", "!", "time",
];

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
    /// Heredoc redirections whose bodies start after the next newline: the
    /// token index, and whether `<<-` strips leading tabs.
    pending: Vec<(usize, bool)>,
    /// The next word would be a command name.
    command_start: bool,
    /// Inside `[[ … ]]`, where `<`, `&&` and `(` are operands, not syntax.
    in_test: bool,
}

impl Lexer {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
            pending: Vec::new(),
            command_start: true,
            in_test: false,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn run(mut self) -> Result<Vec<Token>> {
        loop {
            self.skip_blanks();
            let Some(c) = self.peek() else { break };
            if self.in_test && "&|<>()".contains(c) {
                let mut word = Word::default();
                while let Some(c) = self.peek().filter(|c| "&|<>()".contains(*c)) {
                    word.push_lit(c);
                    self.pos += 1;
                }
                self.tokens.push(Token::Word(word));
                continue;
            }
            match c {
                '\n' => {
                    self.pos += 1;
                    self.op(Op::Newline);
                    self.read_heredocs()?;
                }
                '#' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                ';' => {
                    if self.peek_at(1) == Some(';') {
                        return fail("`case` statements aren't supported");
                    }
                    self.pos += 1;
                    self.op(Op::Semi);
                }
                '&' if self.peek_at(1) == Some('&') => {
                    self.pos += 2;
                    self.op(Op::And);
                }
                '&' if self.peek_at(1) == Some('>') => {
                    self.pos += 2;
                    let op = if self.peek() == Some('>') {
                        self.pos += 1;
                        RedirOp::Append
                    } else {
                        RedirOp::Out
                    };
                    self.redirect(None, op)?;
                }
                '&' => {
                    self.pos += 1;
                    self.op(Op::Background);
                }
                '|' => {
                    self.pos += 1;
                    match self.peek() {
                        Some('|') => {
                            self.pos += 1;
                            self.op(Op::Or);
                        }
                        Some('&') => {
                            self.pos += 1;
                            self.op(Op::Pipe);
                        }
                        _ => self.op(Op::Pipe),
                    }
                }
                '(' => {
                    self.pos += 1;
                    self.op(Op::Open);
                }
                ')' => {
                    self.pos += 1;
                    self.op(Op::Close);
                }
                '<' | '>' if self.peek_at(1) == Some('(') => {
                    self.pos += 2;
                    let inner = self.until_close_paren()?;
                    let word = Word {
                        parts: vec![Part::Dynamic],
                        substs: vec![inner],
                        ..Word::default()
                    };
                    self.word(word);
                }
                '<' | '>' => self.redirection(None)?,
                c if c.is_ascii_digit() && self.digits_then_redirect() => {
                    let mut fd = 0u32;
                    while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
                        fd = fd.saturating_mul(10).saturating_add(d);
                        self.pos += 1;
                    }
                    self.redirection(Some(fd))?;
                }
                _ => {
                    let word = self.lex_word()?;
                    self.word(word);
                }
            }
        }
        if !self.pending.is_empty() {
            return fail("a heredoc has no body");
        }
        if self.in_test {
            return fail("`[[` without `]]`");
        }
        Ok(self.tokens)
    }

    fn op(&mut self, op: Op) {
        self.tokens.push(Token::Op(op));
        self.command_start = true;
    }

    fn word(&mut self, word: Word) {
        let bare = word.bare();
        if self.command_start && bare.as_deref() == Some("[[") {
            self.in_test = true;
        } else if self.in_test && bare.as_deref() == Some("]]") {
            self.in_test = false;
        }
        self.command_start = match bare.as_deref() {
            Some(text) if SKIPPED_RESERVED.contains(&text) => self.command_start,
            _ => word.assign.is_some() && self.command_start,
        };
        self.tokens.push(Token::Word(word));
    }

    fn skip_blanks(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r') => self.pos += 1,
                Some('\\') if self.peek_at(1) == Some('\n') => self.pos += 2,
                _ => return,
            }
        }
    }

    fn digits_then_redirect(&self) -> bool {
        let mut i = self.pos;
        while self.chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
            i += 1;
        }
        matches!(self.chars.get(i), Some('<' | '>')) && self.chars.get(i + 1) != Some(&'(')
    }

    /// Lexes a redirection operator at `pos` and its target.
    fn redirection(&mut self, fd: Option<u32>) -> Result<()> {
        let op = if self.starts_with("<<<") {
            self.pos += 3;
            RedirOp::HereString
        } else if self.starts_with("<<") {
            self.pos += 2;
            let strip_tabs = self.peek() == Some('-');
            if strip_tabs {
                self.pos += 1;
            }
            return self.heredoc(fd, strip_tabs);
        } else if self.starts_with("<>") {
            self.pos += 2;
            RedirOp::ReadWrite
        } else if self.starts_with("<&") || self.starts_with(">&") {
            self.pos += 2;
            RedirOp::Dup
        } else if self.starts_with(">>") {
            self.pos += 2;
            RedirOp::Append
        } else if self.starts_with(">|") {
            self.pos += 2;
            RedirOp::Out
        } else if self.peek() == Some('>') {
            self.pos += 1;
            RedirOp::Out
        } else {
            self.pos += 1;
            RedirOp::In
        };
        self.redirect(fd, op)
    }

    fn redirect(&mut self, fd: Option<u32>, op: RedirOp) -> Result<()> {
        self.skip_blanks();
        if self.peek().is_none_or(|c| "\n;&|()<>".contains(c)) {
            return fail("a redirection has no target");
        }
        let target = self.lex_word()?;
        // `>& file` (not a number or `-`) sends both stdout and stderr to a file.
        let op = match (op, target.literal()) {
            (RedirOp::Dup, Some(text)) if text == "-" || text.chars().all(|c| c.is_ascii_digit()) => op,
            (RedirOp::Dup, _) => RedirOp::Out,
            _ => op,
        };
        self.tokens.push(Token::Redir(Redirect {
            fd,
            op,
            target,
            heredoc: None,
        }));
        Ok(())
    }

    fn heredoc(&mut self, fd: Option<u32>, strip_tabs: bool) -> Result<()> {
        self.skip_blanks();
        let delimiter = self.lex_word()?;
        let Some(text) = delimiter.literal() else {
            return fail("a heredoc delimiter with expansions");
        };
        if text.is_empty() {
            return fail("a heredoc has no delimiter");
        }
        let expands = !delimiter.quoted;
        self.tokens.push(Token::Redir(Redirect {
            fd,
            op: RedirOp::HereDoc,
            target: delimiter,
            heredoc: Some(Heredoc {
                body: String::new(),
                expands,
                expansions: Word::default(),
            }),
        }));
        self.pending.push((self.tokens.len() - 1, strip_tabs));
        Ok(())
    }

    /// Reads the bodies of the heredocs started on the line that just ended.
    fn read_heredocs(&mut self) -> Result<()> {
        for (index, strip_tabs) in std::mem::take(&mut self.pending) {
            let Some(Token::Redir(redirect)) = self.tokens.get(index) else {
                continue;
            };
            let delimiter = redirect.target.literal().unwrap_or_default();
            let mut body = String::new();
            loop {
                if self.peek().is_none() {
                    // Bash warns and uses what it has.
                    break;
                }
                let mut line = String::new();
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == '\n' {
                        break;
                    }
                    line.push(c);
                }
                let line = if strip_tabs {
                    line.trim_start_matches('\t').to_owned()
                } else {
                    line
                };
                if line == delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            let Some(Token::Redir(redirect)) = self.tokens.get_mut(index) else {
                continue;
            };
            if let Some(heredoc) = redirect.heredoc.as_mut() {
                if heredoc.expands {
                    heredoc.expansions = expansions(&body)?;
                }
                heredoc.body = body;
            }
        }
        Ok(())
    }

    fn lex_word(&mut self) -> Result<Word> {
        let mut word = Word::default();
        // Whether everything so far could be the name in `NAME=value`.
        let mut name_so_far = true;
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\r' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>' => break,
                '\\' => {
                    self.pos += 1;
                    match self.peek() {
                        Some('\n') => self.pos += 1,
                        Some(next) => {
                            self.pos += 1;
                            word.quoted = true;
                            word.push_lit(next);
                        }
                        None => word.push_lit('\\'),
                    }
                    name_so_far = false;
                }
                '\'' => {
                    self.pos += 1;
                    word.quoted = true;
                    name_so_far = false;
                    if word.parts.is_empty() {
                        word.parts.push(Part::Lit(String::new()));
                    }
                    loop {
                        match self.peek() {
                            None => return fail("a quote is never closed"),
                            Some('\'') => {
                                self.pos += 1;
                                break;
                            }
                            Some(c) => {
                                self.pos += 1;
                                word.push_lit(c);
                            }
                        }
                    }
                }
                '"' => {
                    self.pos += 1;
                    name_so_far = false;
                    self.double_quoted(&mut word)?;
                }
                '$' => {
                    name_so_far = false;
                    self.dollar(&mut word, false)?;
                }
                '`' => {
                    name_so_far = false;
                    self.backtick(&mut word)?;
                }
                '=' if name_so_far && word.assign.is_none() => {
                    self.pos += 1;
                    let name = word.literal().unwrap_or_default();
                    if is_name(&name) {
                        word.push_lit('=');
                        word.assign = Some((name, word.parts.len()));
                    } else {
                        word.push_lit('=');
                    }
                    name_so_far = false;
                }
                '*' | '?' | '[' | '{' => {
                    self.pos += 1;
                    word.glob = true;
                    name_so_far = false;
                    word.push_lit(c);
                }
                _ => {
                    self.pos += 1;
                    if !(c.is_ascii_alphanumeric() || c == '_') {
                        name_so_far = false;
                    }
                    word.push_lit(c);
                }
            }
        }
        if let Some(text) = word.literal()
            && matches!(text.as_str(), "{" | "{}" | "[" | "[[")
        {
            word.glob = false;
        }
        if word.is_empty() {
            return fail("an empty word");
        }
        Ok(word)
    }

    fn double_quoted(&mut self, word: &mut Word) -> Result<()> {
        word.quoted = true;
        if word.parts.is_empty() {
            word.parts.push(Part::Lit(String::new()));
        }
        loop {
            match self.peek() {
                None => return fail("a quote is never closed"),
                Some('"') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some('\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('\n') => self.pos += 1,
                        Some(c @ ('$' | '`' | '"' | '\\')) => {
                            self.pos += 1;
                            word.push_lit(c);
                        }
                        _ => word.push_lit('\\'),
                    }
                }
                Some('$') => self.dollar(word, true)?,
                Some('`') => self.backtick(word)?,
                Some(c) => {
                    self.pos += 1;
                    word.push_lit(c);
                }
            }
        }
    }

    /// Lexes an expansion starting at `$`.
    fn dollar(&mut self, word: &mut Word, in_quotes: bool) -> Result<()> {
        self.pos += 1;
        match self.peek() {
            Some('(') if self.peek_at(1) == Some('(') => {
                self.pos += 2;
                let inner = self.until_close_paren()?;
                if inner.contains("$(") || inner.contains('`') {
                    return fail("a command substitution inside arithmetic");
                }
                if self.peek() != Some(')') {
                    return fail("arithmetic that isn't closed with `))`");
                }
                self.pos += 1;
                word.parts.push(Part::Dynamic);
            }
            Some('(') => {
                self.pos += 1;
                let inner = self.until_close_paren()?;
                word.substs.push(inner);
                word.parts.push(Part::Dynamic);
            }
            Some('{') => {
                self.pos += 1;
                let mut inner = String::new();
                let mut depth = 1;
                loop {
                    let Some(c) = self.peek() else {
                        return fail("`${` is never closed");
                    };
                    self.pos += 1;
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    inner.push(c);
                }
                if inner.contains("$(") || inner.contains('`') {
                    return fail("a command substitution inside `${…}`");
                }
                let name: String = inner
                    .trim_start_matches(['#', '!'])
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    word.vars.push(name.clone());
                }
                // Variables inside the operator part, like `${A:-$B}`.
                for (i, _) in inner.match_indices('$') {
                    let var: String = inner[i + 1..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !var.is_empty() {
                        word.vars.push(var);
                    }
                }
                if is_name(&inner) {
                    word.parts.push(Part::Var(inner));
                } else {
                    word.parts.push(Part::Dynamic);
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let mut name = String::new();
                while let Some(c) = self.peek().filter(|c| c.is_ascii_alphanumeric() || *c == '_') {
                    name.push(c);
                    self.pos += 1;
                }
                word.vars.push(name.clone());
                word.parts.push(Part::Var(name));
            }
            Some(c) if c.is_ascii_digit() || "@*#?$!-".contains(c) => {
                self.pos += 1;
                word.parts.push(Part::Dynamic);
            }
            Some('\'') if !in_quotes => {
                self.pos += 1;
                word.quoted = true;
                self.ansi_c(word)?;
            }
            Some('"') if !in_quotes => {
                self.pos += 1;
                self.double_quoted(word)?;
            }
            _ => word.push_lit('$'),
        }
        Ok(())
    }

    /// `$'…'` strings. Escapes that can spell arbitrary characters are refused,
    /// so a hidden `;` or `/` can't slip past.
    fn ansi_c(&mut self, word: &mut Word) -> Result<()> {
        if word.parts.is_empty() {
            word.parts.push(Part::Lit(String::new()));
        }
        loop {
            let Some(c) = self.peek() else {
                return fail("a quote is never closed");
            };
            self.pos += 1;
            match c {
                '\'' => return Ok(()),
                '\\' => {
                    let Some(next) = self.peek() else {
                        return fail("a quote is never closed");
                    };
                    self.pos += 1;
                    let ch = match next {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' | '\'' | '"' | '?' => next,
                        _ => return fail("`$'…'` escapes that encode characters"),
                    };
                    word.push_lit(ch);
                }
                c => word.push_lit(c),
            }
        }
    }

    fn backtick(&mut self, word: &mut Word) -> Result<()> {
        self.pos += 1;
        let mut inner = String::new();
        loop {
            let Some(c) = self.peek() else {
                return fail("a backtick is never closed");
            };
            self.pos += 1;
            match c {
                '`' => break,
                '\\' => match self.peek() {
                    Some(next @ ('`' | '\\' | '$')) => {
                        self.pos += 1;
                        inner.push(next);
                    }
                    _ => inner.push('\\'),
                },
                c => inner.push(c),
            }
        }
        word.substs.push(inner);
        word.parts.push(Part::Dynamic);
        Ok(())
    }

    /// Reads up to the `)` that closes an already consumed `(`, skipping
    /// quoted text, and returns what was in between.
    fn until_close_paren(&mut self) -> Result<String> {
        let start = self.pos;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' => self.pos += 1,
                '\'' => {
                    while self.peek().is_some_and(|c| c != '\'') {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                '"' => {
                    while let Some(c) = self.peek() {
                        self.pos += 1;
                        match c {
                            '\\' => self.pos += 1,
                            '"' => break,
                            _ => {}
                        }
                    }
                }
                '`' => {
                    while self.peek().is_some_and(|c| c != '`') {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(self.chars[start..self.pos - 1].iter().collect());
                    }
                }
                _ => {}
            }
        }
        fail("a `(` is never closed")
    }
}

/// What an unquoted heredoc body expands: its variables and substitutions.
fn expansions(body: &str) -> Result<Word> {
    let mut lexer = Lexer::new(body);
    let mut word = Word::default();
    while let Some(c) = lexer.peek() {
        match c {
            '\\' => lexer.pos += 2,
            '$' => lexer.dollar(&mut word, true)?,
            '`' => lexer.backtick(&mut word)?,
            _ => lexer.pos += 1,
        }
    }
    word.parts.clear();
    Ok(word)
}

pub(crate) fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

struct Parser {
    tokens: std::iter::Peekable<std::vec::IntoIter<Token>>,
}

impl Parser {
    fn list(&mut self, nested: bool) -> Result<Script> {
        let mut script = Script::default();
        let mut command = Command::default();
        let mut piped_in = false;
        loop {
            let Some(token) = self.tokens.next() else {
                if nested {
                    return fail("a `(` is never closed");
                }
                finish(&mut script, &mut command, &mut piped_in);
                return Ok(script);
            };
            match token {
                Token::Op(Op::Close) => {
                    if !nested {
                        return fail("an unmatched `)`");
                    }
                    finish(&mut script, &mut command, &mut piped_in);
                    return Ok(script);
                }
                Token::Op(Op::Open) => {
                    if !command.words.is_empty() || !command.assigns.is_empty() {
                        return fail("function definitions and arrays aren't supported");
                    }
                    let inner = self.list(true)?;
                    script.items.push(Item {
                        node: Node::Subshell(inner),
                        piped_in,
                    });
                    piped_in = false;
                }
                Token::Op(Op::Pipe) => {
                    finish(&mut script, &mut command, &mut piped_in);
                    piped_in = true;
                }
                Token::Op(_) => {
                    finish(&mut script, &mut command, &mut piped_in);
                    piped_in = false;
                }
                Token::Redir(redirect) => command.redirs.push(redirect),
                Token::Word(word) => {
                    if command.words.is_empty() {
                        match word.bare().as_deref() {
                            Some(text) if SKIPPED_RESERVED.contains(&text) => continue,
                            Some("for" | "select") => {
                                let bind = self.for_header()?;
                                script.items.push(Item {
                                    node: bind,
                                    piped_in: false,
                                });
                                continue;
                            }
                            Some("case" | "function" | "coproc" | "esac") => {
                                return fail("`case`, functions and coprocesses aren't supported");
                            }
                            _ => {}
                        }
                        match word.into_assign() {
                            Ok(assign) => command.assigns.push(assign),
                            Err(word) => command.words.push(word),
                        }
                    } else {
                        command.words.push(word);
                    }
                }
            }
        }
    }

    /// Reads `NAME [in WORDS]` after `for`.
    fn for_header(&mut self) -> Result<Node> {
        let Some(Token::Word(name)) = self.tokens.next() else {
            return fail("only `for NAME in …` loops are supported");
        };
        let Some(name) = name.bare().filter(|name| is_name(name)) else {
            return fail("only `for NAME in …` loops are supported");
        };
        let has_in =
            matches!(self.tokens.peek(), Some(Token::Word(word)) if word.bare().as_deref() == Some("in"));
        if !has_in {
            return Ok(Node::Bind { name, values: None });
        }
        self.tokens.next();
        let mut values = Vec::new();
        while let Some(Token::Word(_)) = self.tokens.peek() {
            if let Some(Token::Word(word)) = self.tokens.next() {
                values.push(word);
            }
        }
        Ok(Node::Bind {
            name,
            values: Some(values),
        })
    }
}

fn finish(script: &mut Script, command: &mut Command, piped_in: &mut bool) {
    let command = std::mem::take(command);
    if command.words.is_empty() && command.assigns.is_empty() && command.redirs.is_empty() {
        return;
    }
    script.items.push(Item {
        node: Node::Command(command),
        piped_in: *piped_in,
    });
    *piped_in = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(text: &str) -> Vec<Vec<Option<String>>> {
        fn walk(script: &Script, out: &mut Vec<Vec<Option<String>>>) {
            for item in &script.items {
                match &item.node {
                    Node::Command(command) => out.push(command.words.iter().map(Word::literal).collect()),
                    Node::Subshell(inner) => walk(inner, out),
                    Node::Bind { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&parse(text).unwrap(), &mut out);
        out
    }

    fn lit(words: &[&str]) -> Vec<Option<String>> {
        words.iter().map(|w| Some((*w).to_owned())).collect()
    }

    #[test]
    fn splits_lists_and_pipelines() {
        assert_eq!(
            commands("cd src && cargo test || echo 'it failed'; ls | wc -l"),
            vec![
                lit(&["cd", "src"]),
                lit(&["cargo", "test"]),
                lit(&["echo", "it failed"]),
                lit(&["ls"]),
                lit(&["wc", "-l"]),
            ]
        );
    }

    #[test]
    fn keeps_quotes_and_escapes() {
        assert_eq!(
            commands(r#"git commit -m "fix: a \"b\" c" -m 'x;y' a\ b"#),
            vec![lit(&[
                "git",
                "commit",
                "-m",
                "fix: a \"b\" c",
                "-m",
                "x;y",
                "a b"
            ])]
        );
    }

    #[test]
    fn finds_substitutions() {
        let script = parse("echo $(rm -rf /) `whoami` \"$(date)\"").unwrap();
        let Node::Command(command) = &script.items[0].node else {
            panic!()
        };
        let substs: Vec<_> = command.words.iter().flat_map(|w| w.substs.clone()).collect();
        assert_eq!(substs, vec!["rm -rf /", "whoami", "date"]);
    }

    #[test]
    fn reads_heredocs() {
        let script = parse("cat > out.txt <<'EOF'\nhello $(not run)\nEOF\necho done").unwrap();
        assert_eq!(script.items.len(), 2);
        let Node::Command(command) = &script.items[0].node else {
            panic!()
        };
        let heredoc = command.redirs[1].heredoc.as_ref().unwrap();
        assert_eq!(heredoc.body, "hello $(not run)\n");
        assert!(!heredoc.expands);

        let script = parse("cat <<EOF\n$(id)\nEOF").unwrap();
        let Node::Command(command) = &script.items[0].node else {
            panic!()
        };
        let heredoc = command.redirs[0].heredoc.as_ref().unwrap();
        assert_eq!(heredoc.expansions.substs, vec!["id"]);
    }

    #[test]
    fn parses_assignments_redirections_and_loops() {
        let script = parse("RUST_LOG=debug cargo run 2>&1 > log.txt").unwrap();
        let Node::Command(command) = &script.items[0].node else {
            panic!()
        };
        assert_eq!(command.assigns[0].name, "RUST_LOG");
        assert_eq!(command.assigns[0].value.literal().as_deref(), Some("debug"));
        assert_eq!(command.redirs[0].op, RedirOp::Dup);
        assert_eq!(command.redirs[1].op, RedirOp::Out);

        let script = parse("for f in a.rs b.rs; do rustfmt \"$f\"; done").unwrap();
        assert!(
            matches!(&script.items[0].node, Node::Bind { name, values: Some(v) } if name == "f" && v.len() == 2)
        );
        assert_eq!(commands("if [[ -f x && -d y ]]; then echo ok; fi").len(), 2);
    }

    #[test]
    fn refuses_what_it_cannot_follow() {
        for text in [
            "case $x in a) echo;; esac",
            "f() { rm -rf /; }; f",
            "echo 'unclosed",
            "echo $(unclosed",
            "arr=(a b)",
            "echo $'\\x2f'",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
