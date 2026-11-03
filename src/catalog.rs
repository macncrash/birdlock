//! Load WhatsMyName, Sherlock, and Blackbird email lists into one site type.
//!
//! Username sites that share a method, probe URL, and body are one site with
//! both detection rules. A hit then requires both rules to agree.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::json::{self, Value};
use crate::re::{self, Pattern};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SherlockKind {
    Message,
    StatusCode,
    ResponseUrl,
}

#[derive(Clone, Debug)]
pub enum Rule {
    Wmn {
        e_code: i64,
        e_string: String,
        m_code: i64,
        m_string: String,
    },
    Sherlock {
        kinds: Vec<SherlockKind>,
        messages: Vec<String>,
        codes: Vec<i64>,
    },
}

#[derive(Clone, Debug)]
pub struct MetaField {
    pub name: String,
    pub path: Vec<PathSeg>,
}

#[derive(Clone, Debug)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug)]
pub struct PreCheck {
    pub method: String,
    pub url: String,
    pub cookie: String,
}

#[derive(Clone, Debug)]
pub struct Site {
    pub name: String,
    pub aliases: Vec<String>,
    pub category: String,
    pub nsfw: bool,
    pub homepage: String,
    pub display_template: String,
    pub probe_template: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub regex: Option<Pattern>,
    pub strip_chars: String,
    pub input_op: Option<String>,
    pub pre_check: Option<PreCheck>,
    pub rules: Vec<Rule>,
    pub sources: Vec<String>,
    pub meta: Vec<MetaField>,
    pub protection: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    pub user_sites: Vec<Site>,
    pub email_sites: Vec<Site>,
    pub wmn_count: usize,
    pub sherlock_count: usize,
    pub merged_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub wmn: Option<std::path::PathBuf>,
    pub sherlock: Option<std::path::PathBuf>,
    pub email: Option<std::path::PathBuf>,
    pub load_wmn: bool,
    pub load_sherlock: bool,
}

pub fn load(paths: &Paths) -> Result<Catalog, String> {
    let mut warnings = Vec::new();
    let mut bag = Bag::default();
    let mut wmn_count = 0;
    let mut sherlock_count = 0;

    if paths.load_wmn {
        let path = paths
            .wmn
            .clone()
            .ok_or_else(|| "WhatsMyName list path is missing".to_string())?;
        let text = read(&path)?;
        let value = json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        wmn_count = load_wmn(&value, &mut bag, &mut warnings)?;
    }
    if paths.load_sherlock {
        let path = paths
            .sherlock
            .clone()
            .ok_or_else(|| "Sherlock list path is missing".to_string())?;
        let text = read(&path)?;
        let value = json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        sherlock_count = load_sherlock(&value, &mut bag, &mut warnings)?;
    }

    let user_sites = bag.sites;
    let merged_count = user_sites.iter().filter(|s| s.sources.len() > 1).count();

    let mut email_sites = Vec::new();
    if let Some(path) = &paths.email {
        if path.exists() {
            let text = read(path)?;
            let value = json::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            email_sites = load_email(&value, &mut warnings)?;
        } else if paths.email.is_some() {
            return Err(format!("email list not found: {}", path.display()));
        }
    }

    Ok(Catalog {
        user_sites,
        email_sites,
        wmn_count,
        sherlock_count,
        merged_count,
        warnings,
    })
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

#[derive(Default)]
struct Bag {
    sites: Vec<Site>,
    index: HashMap<String, usize>,
}

impl Bag {
    fn upsert(&mut self, site: Site) {
        let key = site_key(&site);
        if let Some(&idx) = self.index.get(&key) {
            merge(&mut self.sites[idx], site);
        } else {
            self.index.insert(key, self.sites.len());
            self.sites.push(site);
        }
    }
}

fn site_key(site: &Site) -> String {
    let body = site
        .body
        .as_deref()
        .unwrap_or("")
        .replace("{}", "{account}");
    format!(
        "{}\n{}\n{body}",
        site.method.to_ascii_uppercase(),
        normalize_url(&site.probe_template)
    )
}

pub fn normalize_url(raw: &str) -> String {
    let trimmed = raw.trim();
    let with_token = trimmed.replace("{}", "{account}");
    let Some(scheme_at) = with_token.find("://") else {
        return with_token.trim_end_matches('/').to_string();
    };
    let scheme = with_token[..scheme_at].to_ascii_lowercase();
    let rest = &with_token[scheme_at + 3..];
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    format!(
        "{scheme}://{}{}",
        host.to_ascii_lowercase(),
        path.trim_end_matches('/')
    )
}

fn merge(into: &mut Site, extra: Site) {
    for alias in extra.aliases {
        if !into.aliases.iter().any(|a| a.eq_ignore_ascii_case(&alias)) {
            if !into.name.eq_ignore_ascii_case(&alias) {
                into.name = format!("{} / {alias}", into.name);
            }
            into.aliases.push(alias);
        }
    }
    if into.category.is_empty() {
        into.category = extra.category;
    }
    into.nsfw = into.nsfw || extra.nsfw;
    if !into.display_template.contains("{account}") && extra.display_template.contains("{account}")
    {
        into.display_template = extra.display_template;
    }
    if into.homepage.is_empty() {
        into.homepage = extra.homepage;
    }
    if into.regex.is_none() {
        into.regex = extra.regex;
    }
    if into.strip_chars.is_empty() {
        into.strip_chars = extra.strip_chars;
    }
    if into.input_op.is_none() {
        into.input_op = extra.input_op;
    }
    if into.pre_check.is_none() {
        into.pre_check = extra.pre_check;
    }
    if into.headers.is_empty() {
        into.headers = extra.headers;
    } else {
        for (k, v) in extra.headers {
            if !into
                .headers
                .iter()
                .any(|(ek, _)| ek.eq_ignore_ascii_case(&k))
            {
                into.headers.push((k, v));
            }
        }
    }
    if into.meta.is_empty() {
        into.meta = extra.meta;
    }
    for p in extra.protection {
        if !into.protection.contains(&p) {
            into.protection.push(p);
        }
    }
    for rule in extra.rules {
        into.rules.push(rule);
    }
    for source in extra.sources {
        if !into.sources.contains(&source) {
            into.sources.push(source);
        }
    }
}

fn load_wmn(root: &Value, bag: &mut Bag, warnings: &mut Vec<String>) -> Result<usize, String> {
    let sites = root
        .get("sites")
        .and_then(Value::as_array)
        .ok_or_else(|| "WhatsMyName list has no sites array".to_string())?;
    let mut count = 0;
    for site in sites {
        if site.as_object().is_none() {
            continue;
        }
        let name = match str_field(site, "name") {
            Some(n) => n,
            None => continue,
        };
        if is_disabled(site) {
            continue;
        }
        let Some(uri) = str_field(site, "uri_check") else {
            warnings.push(format!("{name}: missing uri_check"));
            continue;
        };
        let Some(e_code) = int_field(site, "e_code") else {
            warnings.push(format!("{name}: missing e_code"));
            continue;
        };
        let Some(m_code) = int_field(site, "m_code") else {
            warnings.push(format!("{name}: missing m_code"));
            continue;
        };
        let e_string = str_field(site, "e_string").unwrap_or_default();
        let m_string = str_field(site, "m_string").unwrap_or_default();
        let body = str_field(site, "post_body");
        let method = if body.is_some() { "POST" } else { "GET" };
        let display = str_field(site, "uri_pretty").unwrap_or_else(|| uri.clone());
        let category = str_field(site, "cat").unwrap_or_default();
        let nsfw = category.eq_ignore_ascii_case("xx NSFW xx");
        let headers = headers_of(site.get("headers"));
        let protection = string_list(site.get("protection"));
        let strip_chars = str_field(site, "strip_bad_char").unwrap_or_default();
        bag.upsert(Site {
            aliases: vec![name.clone()],
            name,
            category,
            nsfw,
            homepage: String::new(),
            display_template: token(&display),
            probe_template: token(&uri),
            method: method.into(),
            headers,
            body,
            regex: None,
            strip_chars,
            input_op: None,
            pre_check: None,
            rules: vec![Rule::Wmn {
                e_code,
                e_string,
                m_code,
                m_string,
            }],
            sources: vec!["wmn".into()],
            meta: vec![],
            protection,
        });
        count += 1;
    }
    Ok(count)
}

fn load_sherlock(root: &Value, bag: &mut Bag, warnings: &mut Vec<String>) -> Result<usize, String> {
    let pairs = root
        .as_object()
        .ok_or_else(|| "Sherlock list is not an object".to_string())?;
    let mut count = 0;
    for (name, site) in pairs {
        if name.starts_with('$') {
            continue;
        }
        if site.as_object().is_none() {
            warnings.push(format!("{name}: not an object"));
            continue;
        }
        let Some(url) = str_field(site, "url") else {
            warnings.push(format!("{name}: missing url"));
            continue;
        };
        let Some(kind_value) = site.get("errorType") else {
            warnings.push(format!("{name}: missing errorType"));
            continue;
        };
        let Ok(kinds) = kinds_of(kind_value) else {
            warnings.push(format!("{name}: unknown errorType"));
            continue;
        };
        if kinds.is_empty() {
            warnings.push(format!("{name}: empty errorType"));
            continue;
        }
        let messages = string_list(site.get("errorMsg"));
        let codes = int_list(site.get("errorCode"));
        let body = site.get("request_payload").map(json::stringify);
        let method = str_field(site, "request_method")
            .unwrap_or_else(|| {
                if body.is_some() {
                    "POST".into()
                } else {
                    "GET".into()
                }
            })
            .to_ascii_uppercase();
        let probe = str_field(site, "urlProbe").unwrap_or_else(|| url.clone());
        let homepage = str_field(site, "urlMain").unwrap_or_default();
        let nsfw = site.get("isNSFW").and_then(Value::as_bool).unwrap_or(false);
        let category = string_list(site.get("tags")).join(",");
        let headers = headers_of(site.get("headers"));
        let regex = match str_field(site, "regexCheck") {
            Some(src) if !src.is_empty() => match re::compile(&src) {
                Ok(p) => Some(p),
                Err(e) => {
                    warnings.push(format!("{name}: regex skipped ({e})"));
                    None
                }
            },
            _ => None,
        };
        bag.upsert(Site {
            aliases: vec![name.clone()],
            name: name.clone(),
            category,
            nsfw,
            homepage,
            display_template: token(&url),
            probe_template: token(&probe),
            method,
            headers,
            body,
            regex,
            strip_chars: String::new(),
            input_op: None,
            pre_check: None,
            rules: vec![Rule::Sherlock {
                kinds,
                messages,
                codes,
            }],
            sources: vec!["sherlock".into()],
            meta: vec![],
            protection: vec![],
        });
        count += 1;
    }
    Ok(count)
}

fn load_email(root: &Value, warnings: &mut Vec<String>) -> Result<Vec<Site>, String> {
    let sites = root
        .get("sites")
        .and_then(Value::as_array)
        .ok_or_else(|| "email list has no sites array".to_string())?;
    let mut out = Vec::new();
    for site in sites {
        let Some(name) = str_field(site, "name") else {
            continue;
        };
        let Some(uri) = str_field(site, "uri_check") else {
            warnings.push(format!("{name}: missing uri_check"));
            continue;
        };
        let Some(e_code) = int_field(site, "e_code") else {
            warnings.push(format!("{name}: missing e_code"));
            continue;
        };
        let Some(m_code) = int_field(site, "m_code") else {
            warnings.push(format!("{name}: missing m_code"));
            continue;
        };
        let method = str_field(site, "method")
            .unwrap_or_else(|| "GET".into())
            .to_ascii_uppercase();
        let body = str_field(site, "data");
        let category = str_field(site, "cat").unwrap_or_default();
        let op = str_field(site, "input_operation");
        if let Some(op) = &op {
            if op != "hash-sha256" {
                warnings.push(format!("{name}: unsupported input operation {op}"));
            }
        }
        let pre_check = pre_check_of(site.get("pre_check"));
        let meta = meta_of(site.get("metadata"));
        out.push(Site {
            aliases: vec![name.clone()],
            name,
            category: category.clone(),
            nsfw: category.eq_ignore_ascii_case("xx NSFW xx"),
            homepage: String::new(),
            display_template: token(&uri),
            probe_template: token(&uri),
            method,
            headers: headers_of(site.get("headers")),
            body,
            regex: None,
            strip_chars: String::new(),
            input_op: op,
            pre_check,
            rules: vec![Rule::Wmn {
                e_code,
                e_string: str_field(site, "e_string").unwrap_or_default(),
                m_code,
                m_string: str_field(site, "m_string").unwrap_or_default(),
            }],
            sources: vec!["email".into()],
            meta,
            protection: vec![],
        });
    }
    Ok(out)
}

fn pre_check_of(value: Option<&Value>) -> Option<PreCheck> {
    let value = value?;
    if matches!(value, Value::Null) {
        return None;
    }
    let url = str_field(value, "endpoint")?;
    let cookie = str_field(value, "cookie_name")?;
    let method = str_field(value, "method").unwrap_or_else(|| "GET".into());
    Some(PreCheck {
        method: method.to_ascii_uppercase(),
        url,
        cookie,
    })
}

fn meta_of(value: Option<&Value>) -> Vec<MetaField> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(name) = str_field(item, "name") else {
            continue;
        };
        let schema = str_field(item, "schema").unwrap_or_default();
        if !schema.eq_ignore_ascii_case("json") {
            continue;
        }
        let Some(Value::Array(path)) = item.get("path") else {
            continue;
        };
        let mut segs = Vec::new();
        for seg in path {
            if let Some(s) = seg.as_str() {
                segs.push(PathSeg::Key(s.to_string()));
            } else if let Some(n) = seg.as_i64() {
                if n >= 0 {
                    segs.push(PathSeg::Index(n as usize));
                }
            }
        }
        out.push(MetaField { name, path: segs });
    }
    out
}

fn is_disabled(site: &Value) -> bool {
    match site.get("valid") {
        Some(Value::Bool(false)) => true,
        Some(Value::String(s)) if s == "false" || s == "0" => true,
        _ => false,
    }
}

fn token(url: &str) -> String {
    url.replace("{}", "{account}")
}

fn str_field(site: &Value, key: &str) -> Option<String> {
    match site.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) | None => None,
        Some(other) => Some(json::stringify(other)),
    }
}

fn int_field(site: &Value, key: &str) -> Option<i64> {
    site.get(key).and_then(Value::as_i64)
}

fn headers_of(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(pairs) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    pairs
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn int_list(value: Option<&Value>) -> Vec<i64> {
    match value {
        Some(v) => {
            if let Some(n) = v.as_i64() {
                vec![n]
            } else if let Some(items) = v.as_array() {
                items.iter().filter_map(Value::as_i64).collect()
            } else {
                Vec::new()
            }
        }
        None => Vec::new(),
    }
}

fn kinds_of(value: &Value) -> Result<Vec<SherlockKind>, ()> {
    let mut names = Vec::new();
    match value {
        Value::String(s) => names.push(s.as_str()),
        Value::Array(items) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    names.push(s);
                } else {
                    return Err(());
                }
            }
        }
        _ => return Err(()),
    }
    let mut out = Vec::new();
    for name in names {
        match name {
            "message" => out.push(SherlockKind::Message),
            "status_code" => out.push(SherlockKind::StatusCode),
            "response_url" => out.push(SherlockKind::ResponseUrl),
            _ => return Err(()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        Paths {
            wmn: Some(root.join("wmn-data.json")),
            sherlock: Some(root.join("sherlock.json")),
            email: Some(root.join("email-data.json")),
            load_wmn: true,
            load_sherlock: true,
        }
    }

    #[test]
    fn loads_and_merges_real_lists() {
        let cat = load(&paths()).unwrap();
        assert_eq!(cat.wmn_count, 691);
        assert_eq!(cat.sherlock_count, 481);
        assert_eq!(cat.email_sites.len(), 16);
        assert!(cat.merged_count > 50, "merged {}", cat.merged_count);
        assert!(
            cat.warnings.iter().all(|w| !w.contains("regex skipped")),
            "{:?}",
            cat.warnings
        );
        assert!(cat
            .user_sites
            .iter()
            .any(|s| s.aliases.iter().any(|a| a == "GitHub (User)")));
        assert!(!cat
            .user_sites
            .iter()
            .any(|s| s.name == "7cup" || s.aliases.iter().any(|a| a == "7cup")));
        let gravatar = cat
            .email_sites
            .iter()
            .find(|s| s.name == "Gravatar")
            .unwrap();
        assert_eq!(gravatar.input_op.as_deref(), Some("hash-sha256"));
        assert!(!gravatar.meta.is_empty());
        let eventbrite = cat
            .email_sites
            .iter()
            .find(|s| s.name == "Eventbrite")
            .unwrap();
        assert!(eventbrite.pre_check.is_some());
    }
}
