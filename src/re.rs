//! A small backtracking matcher for the username patterns in Sherlock's list.
//!
//! It covers the subset those patterns use: anchors, classes, escapes, greedy
//! and lazy quantifiers, alternation, groups, lookahead, and lookbehind.
//! `is_match` follows Python `re.search` (a match anywhere), not `re.match`.
//! Fuel stops a runaway pattern. Running out of fuel is reported as "no match".

use std::cell::Cell;

#[derive(Clone, Debug)]
pub struct Pattern {
    ast: Ast,
}

#[derive(Clone, Debug)]
enum Ast {
    Empty,
    Seq(Vec<Ast>),
    Alt(Vec<Ast>),
    Repeat {
        node: Box<Ast>,
        min: u32,
        max: Option<u32>,
        lazy: bool,
    },
    Look {
        node: Box<Ast>,
        kind: LookKind,
    },
    Dot,
    Class {
        neg: bool,
        items: Vec<ClassItem>,
    },
    Char(char),
    Bol,
    Eol,
}

#[derive(Clone, Copy, Debug)]
enum LookKind {
    AheadPos,
    AheadNeg,
    BehindPos,
    BehindNeg,
}

#[derive(Clone, Debug)]
enum ClassItem {
    Char(char),
    Range(char, char),
    Word,
    Digit,
    Space,
    NotWord,
    NotDigit,
    NotSpace,
}

pub fn compile(source: &str) -> Result<Pattern, String> {
    let mut p = Cur { s: source, i: 0 };
    let ast = p.parse_alt()?;
    if p.i != p.s.len() {
        return Err(format!("unparsed tail at byte {} in /{source}/", p.i));
    }
    Ok(Pattern { ast })
}

pub fn is_match(pattern: &Pattern, text: &str) -> bool {
    let fuel = Cell::new(200_000u32);
    let mut i = 0;
    loop {
        let mut ok = false;
        spans(&pattern.ast, text, i, &fuel, &mut |_| {
            ok = true;
            true
        });
        if ok {
            return true;
        }
        if fuel.get() == 0 || i >= text.len() {
            return false;
        }
        let ch = text[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
}

fn burn(fuel: &Cell<u32>) -> bool {
    let v = fuel.get();
    if v == 0 {
        return false;
    }
    fuel.set(v - 1);
    true
}

fn spans(
    node: &Ast,
    text: &str,
    i: usize,
    fuel: &Cell<u32>,
    f: &mut dyn FnMut(usize) -> bool,
) -> bool {
    if !burn(fuel) {
        return false;
    }
    match node {
        Ast::Empty => f(i),
        Ast::Bol => {
            if i == 0 {
                f(i)
            } else {
                false
            }
        }
        Ast::Eol => {
            if i == text.len() {
                f(i)
            } else {
                false
            }
        }
        Ast::Char(c) => match text[i..].chars().next() {
            Some(ch) if ch == *c => f(i + ch.len_utf8()),
            _ => false,
        },
        Ast::Dot => match text[i..].chars().next() {
            Some(ch) if ch != '\n' => f(i + ch.len_utf8()),
            _ => false,
        },
        Ast::Class { neg, items } => match text[i..].chars().next() {
            Some(ch) if class_match(*neg, items, ch) => f(i + ch.len_utf8()),
            _ => false,
        },
        Ast::Seq(nodes) => spans_seq(nodes, text, i, fuel, f),
        Ast::Alt(nodes) => {
            for n in nodes {
                if spans(n, text, i, fuel, f) {
                    return true;
                }
            }
            false
        }
        Ast::Look { node, kind } => {
            let ok = match kind {
                LookKind::AheadPos => any_span(node, text, i, fuel),
                LookKind::AheadNeg => !any_span(node, text, i, fuel),
                LookKind::BehindPos => ends_at(node, text, i, fuel),
                LookKind::BehindNeg => !ends_at(node, text, i, fuel),
            };
            if ok {
                f(i)
            } else {
                false
            }
        }
        Ast::Repeat {
            node,
            min,
            max,
            lazy,
        } => repeat_spans(node, *min, *max, *lazy, text, i, fuel, f),
    }
}

fn any_span(node: &Ast, text: &str, i: usize, fuel: &Cell<u32>) -> bool {
    let mut ok = false;
    spans(node, text, i, fuel, &mut |_| {
        ok = true;
        true
    });
    ok
}

fn ends_at(node: &Ast, text: &str, end_i: usize, fuel: &Cell<u32>) -> bool {
    let mut pos = end_i;
    for _ in 0..48 {
        let mut found = false;
        spans(node, text, pos, fuel, &mut |end| {
            if end == end_i {
                found = true;
                true
            } else {
                false
            }
        });
        if found {
            return true;
        }
        if pos == 0 {
            break;
        }
        pos = prev_char(text, pos);
    }
    false
}

fn prev_char(text: &str, i: usize) -> usize {
    let mut p = i - 1;
    while p > 0 && !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn spans_seq(
    nodes: &[Ast],
    text: &str,
    i: usize,
    fuel: &Cell<u32>,
    f: &mut dyn FnMut(usize) -> bool,
) -> bool {
    if !burn(fuel) {
        return false;
    }
    if nodes.is_empty() {
        return f(i);
    }
    let (first, rest) = nodes.split_first().unwrap();
    spans(first, text, i, fuel, &mut |end| {
        spans_seq(rest, text, end, fuel, f)
    })
}

fn repeat_spans(
    node: &Ast,
    min: u32,
    max: Option<u32>,
    lazy: bool,
    text: &str,
    i: usize,
    fuel: &Cell<u32>,
    f: &mut dyn FnMut(usize) -> bool,
) -> bool {
    let hard_max = max.unwrap_or(4096).min(4096);
    let mut positions: Vec<Vec<usize>> = vec![vec![i]];
    let mut frontier = vec![i];
    let mut k = 0u32;
    while k < hard_max && !frontier.is_empty() {
        if !burn(fuel) {
            return false;
        }
        let mut next = Vec::new();
        for pos in &frontier {
            spans(node, text, *pos, fuel, &mut |end| {
                if end != *pos && !next.contains(&end) {
                    next.push(end);
                }
                false
            });
        }
        if next.is_empty() {
            break;
        }
        k += 1;
        positions.push(next.clone());
        frontier = next;
    }
    let achieved = positions.len() - 1;
    if (min as usize) > achieved {
        return false;
    }
    let start = min as usize;
    if lazy {
        for kk in start..=achieved {
            for end in &positions[kk] {
                if f(*end) {
                    return true;
                }
            }
        }
    } else {
        for kk in (start..=achieved).rev() {
            for end in &positions[kk] {
                if f(*end) {
                    return true;
                }
            }
        }
    }
    false
}

fn class_match(neg: bool, items: &[ClassItem], c: char) -> bool {
    let mut yes = false;
    for item in items {
        let hit = match item {
            ClassItem::Char(x) => c == *x,
            ClassItem::Range(a, b) => c >= *a && c <= *b,
            ClassItem::Word => is_word(c),
            ClassItem::Digit => c.is_ascii_digit(),
            ClassItem::Space => c.is_whitespace(),
            ClassItem::NotWord => !is_word(c),
            ClassItem::NotDigit => !c.is_ascii_digit(),
            ClassItem::NotSpace => !c.is_whitespace(),
        };
        if hit {
            yes = true;
            break;
        }
    }
    if neg {
        !yes
    } else {
        yes
    }
}

fn is_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

struct Cur<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cur<'a> {
    fn parse_alt(&mut self) -> Result<Ast, String> {
        let mut alts = vec![self.parse_concat()?];
        while self.peek() == Some('|') {
            self.bump();
            alts.push(self.parse_concat()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().unwrap())
        } else {
            Ok(Ast::Alt(alts))
        }
    }

    fn parse_concat(&mut self) -> Result<Ast, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.parse_atom()?;
            if let Some((min, max, lazy)) = self.parse_quant()? {
                items.push(Ast::Repeat {
                    node: Box::new(atom),
                    min,
                    max,
                    lazy,
                });
            } else {
                items.push(atom);
            }
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else if items.is_empty() {
            Ok(Ast::Empty)
        } else {
            Ok(Ast::Seq(items))
        }
    }

    fn parse_atom(&mut self) -> Result<Ast, String> {
        let c = self.bump().ok_or_else(|| self.err("unexpected end"))?;
        match c {
            '^' => Ok(Ast::Bol),
            '$' => Ok(Ast::Eol),
            '.' => Ok(Ast::Dot),
            '\\' => self.parse_escape_atom(),
            '[' => self.parse_class(),
            '(' => self.parse_group(),
            '*' | '+' | '?' | '{' | ')' | '|' => Err(self.err("unexpected quantifier")),
            other => Ok(Ast::Char(other)),
        }
    }

    fn parse_group(&mut self) -> Result<Ast, String> {
        if self.peek() == Some('?') {
            self.bump();
            match self.bump() {
                Some(':') => {
                    let inner = self.parse_alt()?;
                    self.expect(')')?;
                    Ok(inner)
                }
                Some('=') => {
                    let inner = self.parse_alt()?;
                    self.expect(')')?;
                    Ok(Ast::Look {
                        node: Box::new(inner),
                        kind: LookKind::AheadPos,
                    })
                }
                Some('!') => {
                    let inner = self.parse_alt()?;
                    self.expect(')')?;
                    Ok(Ast::Look {
                        node: Box::new(inner),
                        kind: LookKind::AheadNeg,
                    })
                }
                Some('<') => {
                    let kind = match self.bump() {
                        Some('=') => LookKind::BehindPos,
                        Some('!') => LookKind::BehindNeg,
                        _ => return Err(self.err("bad lookbehind")),
                    };
                    let inner = self.parse_alt()?;
                    self.expect(')')?;
                    Ok(Ast::Look {
                        node: Box::new(inner),
                        kind,
                    })
                }
                _ => Err(self.err("unsupported group")),
            }
        } else {
            let inner = self.parse_alt()?;
            self.expect(')')?;
            Ok(inner)
        }
    }

    fn parse_escape_atom(&mut self) -> Result<Ast, String> {
        let e = self.bump().ok_or_else(|| self.err("trailing backslash"))?;
        match e {
            'd' => Ok(class_atom(false, vec![ClassItem::Digit])),
            'D' => Ok(class_atom(false, vec![ClassItem::NotDigit])),
            'w' => Ok(class_atom(false, vec![ClassItem::Word])),
            'W' => Ok(class_atom(false, vec![ClassItem::NotWord])),
            's' => Ok(class_atom(false, vec![ClassItem::Space])),
            'S' => Ok(class_atom(false, vec![ClassItem::NotSpace])),
            'n' => Ok(Ast::Char('\n')),
            'r' => Ok(Ast::Char('\r')),
            't' => Ok(Ast::Char('\t')),
            other => Ok(Ast::Char(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Ast, String> {
        let neg = if self.peek() == Some('^') {
            self.bump();
            true
        } else {
            false
        };
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated class"))?;
            if c == ']' && !first {
                break;
            }
            first = false;
            if c == '\\' {
                let e = self.bump().ok_or_else(|| self.err("bad class escape"))?;
                items.push(class_escape(e));
                continue;
            }
            if c == '-' && !items.is_empty() && self.peek().is_some() && self.peek() != Some(']') {
                let mut end = self.bump().unwrap();
                if end == '\\' {
                    end = self.bump().ok_or_else(|| self.err("bad class escape"))?;
                }
                match items.pop() {
                    Some(ClassItem::Char(start)) if start <= end => {
                        items.push(ClassItem::Range(start, end));
                    }
                    Some(other) => {
                        items.push(other);
                        items.push(ClassItem::Char('-'));
                        items.push(ClassItem::Char(end));
                    }
                    None => items.push(ClassItem::Char('-')),
                }
                continue;
            }
            items.push(ClassItem::Char(c));
        }
        Ok(class_atom(neg, items))
    }

    fn parse_quant(&mut self) -> Result<Option<(u32, Option<u32>, bool)>, String> {
        let Some(c) = self.peek() else {
            return Ok(None);
        };
        let (min, max) = match c {
            '*' => {
                self.bump();
                (0, None)
            }
            '+' => {
                self.bump();
                (1, None)
            }
            '?' => {
                self.bump();
                (0, Some(1))
            }
            '{' => {
                self.bump();
                self.parse_brace()?
            }
            _ => return Ok(None),
        };
        let lazy = if self.peek() == Some('?') {
            self.bump();
            true
        } else {
            false
        };
        Ok(Some((min, max, lazy)))
    }

    fn parse_brace(&mut self) -> Result<(u32, Option<u32>), String> {
        let n1 = self.take_digits();
        if self.peek() == Some(',') {
            self.bump();
            let n2 = self.take_digits();
            self.expect('}')?;
            let min = n1.unwrap_or(0);
            if let Some(max) = n2 {
                if max < min {
                    return Err(self.err("reversed quantifier"));
                }
            }
            Ok((min, n2))
        } else {
            self.expect('}')?;
            let n = n1.ok_or_else(|| self.err("empty quantifier"))?;
            Ok((n, Some(n)))
        }
    }

    fn take_digits(&mut self) -> Option<u32> {
        if !matches!(self.peek(), Some('0'..='9')) {
            return None;
        }
        let mut n: u32 = 0;
        while let Some(c) = self.peek() {
            if let Some(d) = c.to_digit(10) {
                self.bump();
                n = n.saturating_mul(10).saturating_add(d);
            } else {
                break;
            }
        }
        Some(n)
    }

    fn expect(&mut self, want: char) -> Result<(), String> {
        if self.bump() == Some(want) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{want}'")))
        }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn err(&self, msg: &str) -> String {
        format!("{msg} at byte {} in /{}/", self.i, self.s)
    }
}

fn class_atom(neg: bool, items: Vec<ClassItem>) -> Ast {
    Ast::Class { neg, items }
}

fn class_escape(e: char) -> ClassItem {
    match e {
        'd' => ClassItem::Digit,
        'D' => ClassItem::NotDigit,
        'w' => ClassItem::Word,
        'W' => ClassItem::NotWord,
        's' => ClassItem::Space,
        'S' => ClassItem::NotSpace,
        'n' => ClassItem::Char('\n'),
        'r' => ClassItem::Char('\r'),
        't' => ClassItem::Char('\t'),
        other => ClassItem::Char(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, text: &str) -> bool {
        is_match(&compile(pat).unwrap(), text)
    }

    #[test]
    fn anchors_quantifiers_and_lookaround() {
        assert!(m("^abc$", "abc"));
        assert!(!m("^abc$", "ab"));
        assert!(m("[a-z]{3,5}", "zzabcdezz"));
        assert!(!m("^[a-z]{3,5}$", "abcdef"));
        assert!(m(r"^[^.]*$", "abc"));
        assert!(!m(r"^[^.]*$", "a.b"));
        assert!(m(r"^(?![.-])[a-zA-Z0-9_.-]{3,20}$", "abc"));
        assert!(!m(r"^(?![.-])[a-zA-Z0-9_.-]{3,20}$", ".abc"));
        assert!(m(r"^(?!-)[a-zA-Z0-9-]{3,}(?<!-)$", "ab-c"));
        assert!(!m(r"^(?!-)[a-zA-Z0-9-]{3,}(?<!-)$", "ab-"));
        assert!(m(r"^((?!\.).)*$", "abc"));
        assert!(!m(r"^((?!\.).)*$", "a.b"));
        assert!(m(r"^[\w-]{,34}$", "a-b"));
        assert!(m(
            r"^[a-zA-Z0-9](?:[a-zA-Z0-9]|-(?=[a-zA-Z0-9])){0,38}$",
            "a-b"
        ));
        assert!(!m(
            r"^[a-zA-Z0-9](?:[a-zA-Z0-9]|-(?=[a-zA-Z0-9])){0,38}$",
            "a-"
        ));
        assert!(m("[a-zA-Z0-9]{3,15}", "xxabcxx"));
    }
}
