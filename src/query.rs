//! Username expansion and site filters.

use crate::catalog::Site;

#[derive(Clone, Debug)]
pub struct Filter {
    conds: Vec<Cond>,
    ops: Vec<Logic>,
}

#[derive(Clone, Debug)]
struct Cond {
    prop: String,
    op: String,
    value: String,
}

#[derive(Clone, Copy, Debug)]
enum Logic {
    And,
    Or,
}

/// Sherlock's `{?}` stands for `_`, `-`, and `.` in turn.
pub fn expand_brace(name: &str) -> Vec<String> {
    if name.contains("{?}") {
        ["_", "-", "."]
            .into_iter()
            .map(|sep| name.replace("{?}", sep))
            .collect()
    } else {
        vec![name.to_string()]
    }
}

/// Blackbird's username permutations. Single tokens are kept only for `all`.
/// Results are de-duplicated, which the original does not do.
pub fn permute(elements: &[String], all: bool) -> Vec<String> {
    let names: Vec<String> = elements
        .iter()
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if names.len() <= 1 {
        return names;
    }
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for length in 1..=names.len() {
        let mut idx = vec![0usize; length];
        let mut used = vec![false; names.len()];
        gen(
            &names, length, 0, &mut idx, &mut used, all, &mut out, &mut seen,
        );
    }
    out
}

fn gen(
    names: &[String],
    length: usize,
    depth: usize,
    idx: &mut [usize],
    used: &mut [bool],
    all: bool,
    out: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<String>,
) {
    if depth == length {
        if length == 1 {
            if all {
                let one = names[idx[0]].clone();
                push_unique(out, seen, one.clone());
                push_unique(out, seen, format!("_{one}"));
                push_unique(out, seen, format!("{one}_"));
            }
            return;
        }
        let parts: Vec<&str> = idx.iter().map(|&i| names[i].as_str()).collect();
        for sep in ["", "_", "-", "."] {
            let joined = parts.join(sep);
            push_unique(out, seen, joined.clone());
            if sep.is_empty() {
                push_unique(out, seen, format!("_{joined}"));
                push_unique(out, seen, format!("{joined}_"));
            }
        }
        return;
    }
    for i in 0..names.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        idx[depth] = i;
        gen(names, length, depth + 1, idx, used, all, out, seen);
        used[i] = false;
    }
}

fn push_unique(out: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>, s: String) {
    if seen.insert(s.clone()) {
        out.push(s);
    }
}

pub fn parse_filter(input: &str) -> Result<Filter, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty filter".into());
    }
    let mut chunks = Vec::new();
    let mut ops = Vec::new();
    let mut rest = input;
    loop {
        if let Some(i) = find_logic(rest) {
            let (head, tail) = rest.split_at(i);
            chunks.push(head.trim());
            if tail.starts_with(" and ") {
                ops.push(Logic::And);
                rest = &tail[" and ".len()..];
            } else if tail.starts_with(" or ") {
                ops.push(Logic::Or);
                rest = &tail[" or ".len()..];
            } else {
                return Err(format!("bad filter near '{tail}'"));
            }
        } else {
            chunks.push(rest.trim());
            break;
        }
    }
    if chunks.iter().any(|c| c.is_empty()) || ops.len() + 1 != chunks.len() {
        return Err(format!("bad filter '{input}'"));
    }
    let mut conds = Vec::new();
    for chunk in chunks {
        conds.push(parse_cond(chunk)?);
    }
    Ok(Filter { conds, ops })
}

fn find_logic(s: &str) -> Option<usize> {
    let and_at = s.find(" and ");
    let or_at = s.find(" or ");
    match (and_at, or_at) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn parse_cond(chunk: &str) -> Result<Cond, String> {
    let mut chars = chunk.char_indices();
    let mut prop_end = None;
    for (i, c) in chars.by_ref() {
        if c.is_whitespace() || "=~><!".contains(c) {
            prop_end = Some(i);
            break;
        }
    }
    let prop_end = prop_end.ok_or_else(|| format!("filter needs an operator in '{chunk}'"))?;
    let prop = chunk[..prop_end].trim();
    if prop.is_empty() {
        return Err(format!("filter is missing a property in '{chunk}'"));
    }
    let after = chunk[prop_end..].trim_start();
    let ops = ["!=", ">=", "<=", "=", "~", ">", "<"];
    let op = ops
        .iter()
        .find(|op| after.starts_with(*op))
        .ok_or_else(|| format!("unknown operator in '{chunk}'"))?;
    let value = after[op.len()..].trim();
    if value.is_empty() {
        return Err(format!("filter is missing a value in '{chunk}'"));
    }
    Ok(Cond {
        prop: prop.to_string(),
        op: (*op).to_string(),
        value: value.to_string(),
    })
}

pub fn matches_filter(filter: &Filter, site: &Site) -> bool {
    let mut result = eval(&filter.conds[0], site);
    for (i, cond) in filter.conds.iter().enumerate().skip(1) {
        let next = eval(cond, site);
        result = match filter.ops[i - 1] {
            Logic::And => result && next,
            Logic::Or => result || next,
        };
    }
    result
}

fn eval(cond: &Cond, site: &Site) -> bool {
    let Some(raw) = property(site, &cond.prop) else {
        return false;
    };
    let left = raw.to_lowercase();
    let right = cond.value.to_lowercase();
    match cond.op.as_str() {
        "=" => left == right,
        "!=" => left != right,
        "~" => left.contains(&right),
        ">" | "<" | ">=" | "<=" => match (left.parse::<f64>(), right.parse::<f64>()) {
            (Ok(a), Ok(b)) => match cond.op.as_str() {
                ">" => a > b,
                "<" => a < b,
                ">=" => a >= b,
                "<=" => a <= b,
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

fn property(site: &Site, name: &str) -> Option<String> {
    match name.to_lowercase().as_str() {
        "name" => Some(site.name.clone()),
        "cat" | "category" => Some(site.category.clone()),
        "source" | "sources" => Some(site.sources.join("+")),
        "nsfw" => Some(if site.nsfw { "true" } else { "false" }.into()),
        "method" => Some(site.method.clone()),
        _ => None,
    }
}

pub fn select_sites(
    sites: &[Site],
    names: &[String],
    include_nsfw: bool,
    filter: Option<&Filter>,
) -> Result<Vec<Site>, String> {
    let wanted: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
    let mut missing = wanted.clone();
    let mut out = Vec::new();
    for site in sites {
        if !wanted.is_empty() {
            let hit = site
                .aliases
                .iter()
                .any(|alias| wanted.iter().any(|w| w == &alias.to_lowercase()));
            if !hit {
                continue;
            }
        } else if site.nsfw && !include_nsfw {
            continue;
        }
        if let Some(filter) = filter {
            if !matches_filter(filter, site) {
                continue;
            }
        }
        for alias in &site.aliases {
            missing.retain(|w| w != &alias.to_lowercase());
        }
        out.push(site.clone());
    }
    if !wanted.is_empty() && out.is_empty() {
        return Err(format!("no sites named {}", names.join(", ")));
    }
    if !missing.is_empty() && !wanted.is_empty() && missing.len() != wanted.len() {
        eprintln!("birdlock: sites not in the lists: {}", missing.join(", "));
    }
    if filter.is_some() && out.is_empty() {
        return Err("no sites matched the filter".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Rule, Site};

    #[test]
    fn permute_matches_blackbird_order() {
        let strict = permute(&["John".into(), "Doe".into()], false);
        assert_eq!(
            strict,
            vec![
                "johndoe", "_johndoe", "johndoe_", "john_doe", "john-doe", "john.doe", "doejohn",
                "_doejohn", "doejohn_", "doe_john", "doe-john", "doe.john",
            ]
        );
        let all = permute(&["John".into(), "Doe".into()], true);
        assert_eq!(all[0], "john");
        assert!(all.contains(&"doe".to_string()));
        assert_eq!(all.len(), 18);
    }

    #[test]
    fn brace_and_filter() {
        assert_eq!(expand_brace("a{?}b"), vec!["a_b", "a-b", "a.b"]);
        let filter = parse_filter("cat=xx NSFW xx and name~git").unwrap();
        let mut site = empty_site("GitHub");
        site.category = "xx NSFW xx".into();
        assert!(matches_filter(&filter, &site));
        site.category = "social".into();
        assert!(!matches_filter(&filter, &site));
        assert!(parse_filter("").is_err());
        assert!(parse_filter("cat").is_err());
    }

    fn empty_site(name: &str) -> Site {
        Site {
            name: name.into(),
            aliases: vec![name.into()],
            category: String::new(),
            nsfw: false,
            homepage: String::new(),
            display_template: String::new(),
            probe_template: String::new(),
            method: "GET".into(),
            headers: vec![],
            body: None,
            regex: None,
            strip_chars: String::new(),
            input_op: None,
            pre_check: None,
            rules: vec![Rule::Wmn {
                e_code: 200,
                e_string: String::new(),
                m_code: 404,
                m_string: String::new(),
            }],
            sources: vec!["wmn".into()],
            meta: vec![],
            protection: vec![],
        }
    }
}
