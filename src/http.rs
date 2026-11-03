//! HTTP through the system `curl` binary.
//! TLS verification stays on. That is deliberate: Blackbird disabled it.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const MAX_BODY: usize = 2 * 1024 * 1024;

static SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub proxy: Option<String>,
    pub redirects: bool,
    pub user_agent: String,
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub final_url: String,
    pub headers: Vec<(String, String)>,
    pub cookies: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}

pub trait HttpClient: Sync {
    fn execute(&self, request: &Request) -> Result<Response, String>;
}

#[derive(Clone, Debug, Default)]
pub struct CurlClient;

impl HttpClient for CurlClient {
    fn execute(&self, request: &Request) -> Result<Response, String> {
        if !is_http_url(&request.url) {
            return Err(format!("refusing non-http url {}", request.url));
        }
        if request.url.chars().any(|c| c == '\n' || c == '\r') {
            return Err("refusing url with a newline".into());
        }
        for (k, v) in &request.headers {
            if k.chars().any(|c| c.is_control()) || v.chars().any(|c| c == '\n' || c == '\r') {
                return Err(format!("refusing header {k}"));
            }
        }

        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir();
        let body_path = dir.join(format!("birdlock-{pid}-{id}.body"));
        let hdr_path = dir.join(format!("birdlock-{pid}-{id}.hdr"));
        let post_path = dir.join(format!("birdlock-{pid}-{id}.post"));
        let cleanup = TempGuard {
            paths: vec![body_path.clone(), hdr_path.clone(), post_path.clone()],
        };

        if let Some(body) = &request.body {
            fs::write(&post_path, body).map_err(|e| format!("temp body: {e}"))?;
        }

        let secs = request.timeout.as_secs().max(1).to_string();
        let mut cmd = Command::new("curl");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("--silent")
            .arg("--show-error")
            .arg("--globoff")
            .arg("--compressed")
            .arg("--http1.1")
            .arg("--max-time")
            .arg(&secs)
            .arg("--connect-timeout")
            .arg(connect_timeout(&secs))
            .arg("--user-agent")
            .arg(&request.user_agent)
            .arg("--dump-header")
            .arg(&hdr_path)
            .arg("--output")
            .arg(&body_path)
            .arg("--write-out")
            .arg("%{http_code}\n%{url_effective}\n")
            .arg("-H")
            .arg("Expect:");
        if request.redirects {
            cmd.arg("--location").arg("--max-redirs").arg("10");
        }
        if let Some(proxy) = &request.proxy {
            cmd.arg("--proxy").arg(proxy);
        }
        for (k, v) in &request.headers {
            cmd.arg("-H").arg(format!("{k}: {v}"));
        }
        cmd.arg("-X").arg(request.method.to_ascii_uppercase());
        if request.body.is_some() {
            cmd.arg("--data-binary")
                .arg(format!("@{}", post_path.display()));
        }
        cmd.arg(&request.url);

        let child = cmd
            .output()
            .map_err(|e| format!("curl failed to start ({e}). Install curl and retry."))?;
        let stdout = String::from_utf8_lossy(&child.stdout);
        let stderr = String::from_utf8_lossy(&child.stderr);
        let mut lines = stdout.lines();
        let code_line = lines.next().unwrap_or("000");
        let final_url = lines.next().unwrap_or("").trim().to_string();
        let status: u16 = code_line.trim().parse().unwrap_or(0);
        if status == 0 {
            let detail = stderr.trim();
            let detail = if detail.len() > 300 {
                &detail[..300]
            } else {
                detail
            };
            return Err(if detail.is_empty() {
                format!("no response from {}", request.url)
            } else {
                format!("{detail}")
            });
        }

        let header_text = fs::read_to_string(&hdr_path).unwrap_or_default();
        let (headers, cookies) = parse_header_dump(&header_text);
        let body = read_limited(&body_path, MAX_BODY).unwrap_or_default();
        drop(cleanup);
        let _ = child.status;
        Ok(Response {
            status,
            final_url: if final_url.is_empty() {
                request.url.clone()
            } else {
                final_url
            },
            headers,
            cookies,
            body,
        })
    }
}

fn connect_timeout(total: &str) -> String {
    match total.parse::<u64>() {
        Ok(n) if n < 10 => n.to_string(),
        _ => "10".into(),
    }
}

fn is_http_url(url: &str) -> bool {
    let u = url.trim();
    let lower = u.get(..8).unwrap_or(u).to_ascii_lowercase();
    lower.starts_with("https://") || u.to_ascii_lowercase().starts_with("http://")
}

fn read_limited(path: &PathBuf, max: usize) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while buf.len() < max {
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let take = n.min(max - buf.len());
        buf.extend_from_slice(&chunk[..take]);
    }
    Ok(buf)
}

struct TempGuard {
    paths: Vec<PathBuf>,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn parse_header_dump(raw: &str) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let normalized = raw.replace("\r\n", "\n");
    let mut headers = Vec::new();
    let mut cookies = Vec::new();
    for block in normalized.split("\n\n") {
        let mut lines = block.lines().filter(|l| !l.is_empty());
        let Some(first) = lines.next() else {
            continue;
        };
        if !first.starts_with("HTTP/") {
            continue;
        }
        let mut block_headers = Vec::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                continue;
            }
            if name.eq_ignore_ascii_case("set-cookie") {
                if let Some((cookie, cookie_value)) = split_cookie(value) {
                    cookies.push((cookie, cookie_value));
                }
            }
            block_headers.push((name.to_string(), value.to_string()));
        }
        if !block_headers.is_empty() {
            headers = block_headers;
        }
    }
    (headers, cookies)
}

fn split_cookie(value: &str) -> Option<(String, String)> {
    let pair = value.split(';').next()?.trim();
    let (name, raw) = pair.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), raw.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_redirect_cookies_and_last_headers() {
        let raw = "\
HTTP/1.1 302 Found\r\n\
Set-Cookie: csrftoken=abc123; Path=/\r\n\
Location: /next\r\n\
\r\n\
HTTP/1.1 200 OK\r\n\
Content-Type: application/json\r\n\
Set-Cookie: other=1\r\n\
\r\n";
        let (headers, cookies) = parse_header_dump(raw);
        assert_eq!(headers[0].0, "Content-Type");
        assert_eq!(cookies[0], ("csrftoken".into(), "abc123".into()));
        let response = Response {
            status: 200,
            final_url: String::new(),
            headers,
            cookies,
            body: Vec::new(),
        };
        assert_eq!(response.cookie("csrftoken").as_deref(), Some("abc123"));
    }
}
