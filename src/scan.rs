//! Run probes. One site is one HTTP request, judged by every rule it carries.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::catalog::{MetaField, PathSeg, Site};
use crate::detect::{self, Verdict};
use crate::http::{HttpClient, Request};
use crate::json::{self, Value};
use crate::re;
use crate::sha256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Found,
    Missing,
    Disputed,
    Illegal,
    Waf,
    Error,
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Found => "found",
            Status::Missing => "missing",
            Status::Disputed => "disputed",
            Status::Illegal => "illegal",
            Status::Waf => "waf",
            Status::Error => "error",
            Status::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub account: String,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub status: Status,
    pub category: String,
    pub sources: Vec<String>,
    pub http_status: Option<u16>,
    pub detail: Option<String>,
    pub meta: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub timeout: Duration,
    pub jobs: usize,
    pub proxy: Option<String>,
    pub user_agent: String,
    pub verbose: bool,
    pub print_all: bool,
    pub color: bool,
    pub dump_dir: Option<PathBuf>,
    pub print: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            jobs: 20,
            proxy: None,
            user_agent: String::new(),
            verbose: false,
            print_all: false,
            color: false,
            dump_dir: None,
            print: false,
        }
    }
}

pub fn run(
    sites: &[Site],
    account: &str,
    kind: &str,
    http: &dyn HttpClient,
    opts: &Options,
) -> Vec<Hit> {
    if sites.is_empty() {
        return Vec::new();
    }
    let dump_root = opts.dump_dir.as_ref().map(|dir| {
        let sub = dir.join(safe(account));
        let _ = fs::create_dir_all(&sub);
        sub
    });
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let next = &next;
    let dump_root = dump_root.as_ref();
    let (tx, rx) = mpsc::channel();
    let jobs = opts.jobs.max(1).min(64).min(sites.len());
    let tty = opts.print && std::io::stderr().is_terminal();
    let total = sites.len();

    thread::scope(|scope| {
        for _ in 0..jobs {
            let tx = tx.clone();
            scope.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= sites.len() {
                    break;
                }
                let hit = probe_site(&sites[i], account, kind, http, opts, dump_root);
                let _ = tx.send(hit);
            });
        }
        drop(tx);
        let mut hits = Vec::with_capacity(total);
        let mut found = 0usize;
        while let Ok(hit) = rx.recv() {
            if hit.status == Status::Found {
                found += 1;
            }
            if opts.print {
                print_hit(&hit, opts, tty);
                if tty {
                    eprint!(
                        "\r\x1b[2Kchecking {}/{total}  found {found}",
                        hits.len() + 1
                    );
                    let _ = std::io::stderr().flush();
                } else if (hits.len() + 1) % 50 == 0 {
                    eprintln!("checking {}/{total}  found {found}", hits.len() + 1);
                }
            }
            hits.push(hit);
        }
        if opts.print && tty {
            eprintln!();
        }
        if opts.print {
            summarize(account, &hits, started.elapsed());
        }
        hits.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.url.cmp(&b.url))
        });
        hits
    })
}

pub fn probe_site(
    site: &Site,
    account: &str,
    kind: &str,
    http: &dyn HttpClient,
    opts: &Options,
    dump_root: Option<&PathBuf>,
) -> Hit {
    let base = |status, url: String, http_status, detail: Option<String>, meta| Hit {
        account: account.to_string(),
        kind: kind.to_string(),
        name: site.name.clone(),
        url,
        status,
        category: site.category.clone(),
        sources: site.sources.clone(),
        http_status,
        detail,
        meta,
    };

    let prepared = match prepare_account(site, account) {
        Prep::Ready(value) => value,
        Prep::Illegal => {
            return base(Status::Illegal, String::new(), None, None, Vec::new());
        }
        Prep::Error(msg) => {
            return base(Status::Error, String::new(), None, Some(msg), Vec::new());
        }
    };

    let mut headers = site.headers.clone();
    let mut body = site.body.clone();
    if let Some(pre) = &site.pre_check {
        match run_precheck(pre, http, opts) {
            Ok(cookie) => {
                let placeholder = format!("{{{}_value}}", pre.cookie);
                for (_, value) in &mut headers {
                    *value = value.replace(&placeholder, &cookie);
                }
                if let Some(text) = &mut body {
                    *text = text.replace(&placeholder, &cookie);
                }
            }
            Err(msg) => {
                return base(Status::Error, String::new(), None, Some(msg), Vec::new());
            }
        }
    }
    for (_, value) in &mut headers {
        *value = subst(value, &prepared, false);
    }
    if let Some(text) = &mut body {
        *text = subst(text, &prepared, false);
    }

    let probe = subst(&site.probe_template, &prepared, true);
    let display = subst(&site.display_template, &prepared, true);
    let shown = if site.display_template.contains("{account}") {
        display
    } else {
        probe.clone()
    };
    if !is_web(&probe) {
        return base(
            Status::Error,
            shown,
            None,
            Some(format!("refusing non-http probe {probe}")),
            Vec::new(),
        );
    }

    let request = Request {
        method: site.method.clone(),
        url: probe,
        headers,
        body: body.map(|text| text.into_bytes()),
        timeout: opts.timeout,
        proxy: opts.proxy.clone(),
        redirects: !detect::wants_raw_status(&site.rules),
        user_agent: opts.user_agent.clone(),
    };
    let response = match http.execute(&request) {
        Ok(response) => response,
        Err(msg) => return base(Status::Error, shown, None, Some(msg), Vec::new()),
    };
    let text = response.text();
    let verdict = detect::judge(&site.rules, response.status as i64, &text);
    let status = match verdict {
        Verdict::Found => Status::Found,
        Verdict::Missing => Status::Missing,
        Verdict::Disputed => Status::Disputed,
        Verdict::Waf => Status::Waf,
        Verdict::Unknown => Status::Unknown,
    };
    let meta = if status == Status::Found {
        extract_meta(&text, &site.meta)
    } else {
        Vec::new()
    };
    if status == Status::Found {
        if let Some(root) = dump_root {
            let path = root.join(format!("{}.body", safe(&site.name)));
            let _ = fs::write(path, &response.body);
        }
    }
    base(status, shown, Some(response.status), None, meta)
}

enum Prep {
    Ready(String),
    Illegal,
    Error(String),
}

fn prepare_account(site: &Site, raw: &str) -> Prep {
    let mut account = raw.to_string();
    if let Some(op) = &site.input_op {
        if op == "hash-sha256" {
            account = sha256::sha256_hex(account.as_bytes());
        } else {
            return Prep::Error(format!("unsupported input operation {op}"));
        }
    }
    if !site.strip_chars.is_empty() {
        let strip = site.strip_chars.clone();
        account.retain(|c| !strip.contains(c));
    }
    if account.is_empty() {
        return Prep::Illegal;
    }
    if let Some(pattern) = &site.regex {
        if !re::is_match(pattern, &account) {
            return Prep::Illegal;
        }
    }
    Prep::Ready(account)
}

fn subst(template: &str, account: &str, encode_space: bool) -> String {
    let value = if encode_space {
        account.replace(' ', "%20")
    } else {
        account.to_string()
    };
    // `{account}` first so a later `{}` pass does not eat the longer token.
    template.replace("{account}", &value).replace("{}", &value)
}

fn run_precheck(
    pre: &crate::catalog::PreCheck,
    http: &dyn HttpClient,
    opts: &Options,
) -> Result<String, String> {
    if !is_web(&pre.url) {
        return Err(format!("refusing precheck url {}", pre.url));
    }
    let response = http.execute(&Request {
        method: pre.method.clone(),
        url: pre.url.clone(),
        headers: Vec::new(),
        body: None,
        timeout: opts.timeout,
        proxy: opts.proxy.clone(),
        redirects: true,
        user_agent: opts.user_agent.clone(),
    })?;
    response
        .cookie(&pre.cookie)
        .ok_or_else(|| format!("precheck did not set cookie {}", pre.cookie))
}

fn extract_meta(body: &str, fields: &[MetaField]) -> Vec<(String, String)> {
    if fields.is_empty() {
        return Vec::new();
    }
    let trimmed = body.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return Vec::new();
    }
    let Ok(value) = json::parse(trimmed) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for field in fields {
        let Some(found) = walk(&value, &field.path) else {
            continue;
        };
        let Some(text) = scalar(found) else {
            continue;
        };
        if !text.is_empty() {
            out.push((field.name.clone(), text));
        }
    }
    out
}

fn walk<'a>(mut value: &'a Value, path: &[PathSeg]) -> Option<&'a Value> {
    for seg in path {
        value = match seg {
            PathSeg::Key(key) => value.get(key)?,
            PathSeg::Index(index) => value.as_array()?.get(*index)?,
        };
    }
    Some(value)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.clone()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn is_web(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

fn safe(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.len() > 80 {
        out.truncate(80);
    }
    if out.is_empty() {
        out.push_str("site");
    }
    out
}

fn print_hit(hit: &Hit, opts: &Options, tty: bool) {
    let show = match hit.status {
        Status::Found => true,
        Status::Missing => opts.print_all,
        Status::Illegal => opts.verbose,
        Status::Disputed | Status::Waf | Status::Error | Status::Unknown => {
            opts.verbose || opts.print_all
        }
    };
    if !show {
        return;
    }
    if tty {
        eprint!("\r\x1b[2K");
    }
    let (mark, color) = match hit.status {
        Status::Found => ("+", "32"),
        Status::Missing => ("-", "2"),
        Status::Disputed => ("~", "33"),
        Status::Waf => ("~", "33"),
        Status::Illegal => (".", "2"),
        Status::Error => ("!", "31"),
        Status::Unknown => ("?", "33"),
    };
    let mark = paint(opts.color, color, &format!("[{mark}]"));
    let mut line = format!("{mark} {}", hit.name);
    if !hit.url.is_empty() {
        line.push_str("  ");
        line.push_str(&hit.url);
    }
    if opts.verbose {
        if let Some(code) = hit.http_status {
            line.push_str(&format!("  http {code}"));
        }
        if !hit.sources.is_empty() {
            line.push_str("  ");
            line.push_str(&hit.sources.join("+"));
        }
        if !hit.category.is_empty() {
            line.push_str("  ");
            line.push_str(&hit.category);
        }
        for (k, v) in &hit.meta {
            line.push_str(&format!("  {k}={v}"));
        }
        if let Some(detail) = &hit.detail {
            line.push_str("  ");
            line.push_str(detail);
        }
    }
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn summarize(account: &str, hits: &[Hit], elapsed: Duration) {
    let mut counts = [0usize; 7];
    for hit in hits {
        let idx = match hit.status {
            Status::Found => 0,
            Status::Missing => 1,
            Status::Disputed => 2,
            Status::Illegal => 3,
            Status::Waf => 4,
            Status::Error => 5,
            Status::Unknown => 6,
        };
        counts[idx] += 1;
    }
    eprintln!(
        "{account}: {} found, {} missing, {} disputed, {} illegal, {} waf, {} errors, {} unknown in {:.1}s",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        counts[5],
        counts[6],
        elapsed.as_secs_f64()
    );
}

fn paint(on: bool, code: &str, text: &str) -> String {
    if on {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Rule;
    use crate::http::{Request, Response};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MapClient {
        map: Mutex<HashMap<String, Response>>,
    }

    impl HttpClient for MapClient {
        fn execute(&self, request: &Request) -> Result<Response, String> {
            self.map
                .lock()
                .unwrap()
                .get(&request.url)
                .cloned()
                .ok_or_else(|| format!("no scripted response for {}", request.url))
        }
    }

    fn site() -> Site {
        Site {
            name: "Example".into(),
            aliases: vec!["Example".into()],
            category: "social".into(),
            nsfw: false,
            homepage: "https://example.com".into(),
            display_template: "https://example.com/{account}".into(),
            probe_template: "https://example.com/{account}".into(),
            method: "GET".into(),
            headers: vec![],
            body: None,
            regex: Some(crate::re::compile(r"^[a-z]{3,8}$").unwrap()),
            strip_chars: ".".into(),
            input_op: None,
            pre_check: None,
            rules: vec![Rule::Wmn {
                e_code: 200,
                e_string: "profile".into(),
                m_code: 404,
                m_string: "missing".into(),
            }],
            sources: vec!["wmn".into()],
            meta: vec![],
            protection: vec![],
        }
    }

    #[test]
    fn scripted_hit_strips_and_rejects_illegal() {
        let client = MapClient {
            map: Mutex::new(HashMap::from([(
                "https://example.com/ada".into(),
                Response {
                    status: 200,
                    final_url: "https://example.com/ada".into(),
                    headers: vec![],
                    cookies: vec![],
                    body: b"profile ok".to_vec(),
                },
            )])),
        };
        let opts = Options::default();
        let hit = probe_site(&site(), "a.da", "user", &client, &opts, None);
        assert_eq!(hit.status, Status::Found);
        assert_eq!(hit.url, "https://example.com/ada");
        let bad = probe_site(&site(), "NO", "user", &client, &opts, None);
        assert_eq!(bad.status, Status::Illegal);
    }
}
