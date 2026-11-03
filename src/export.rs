//! CSV, JSON, and plain-text writers for scan hits.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use crate::scan::Hit;

pub fn write_json(path: &Path, hits: &[Hit]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(file, "[")?;
    for (i, hit) in hits.iter().enumerate() {
        let suffix = if i + 1 == hits.len() { "" } else { "," };
        writeln!(file, "  {}{suffix}", object(hit))?;
    }
    writeln!(file, "]")?;
    Ok(())
}

fn object(hit: &Hit) -> String {
    let mut meta = String::from("{");
    for (i, (k, v)) in hit.meta.iter().enumerate() {
        if i > 0 {
            meta.push(',');
        }
        meta.push_str(&format!("{}:{}", js_string(k), js_string(v)));
    }
    meta.push('}');
    format!(
        "{{\"account\":{},\"kind\":{},\"name\":{},\"url\":{},\"status\":{},\"category\":{},\"sources\":{},\"http_status\":{},\"detail\":{},\"meta\":{meta}}}",
        js_string(&hit.account),
        js_string(&hit.kind),
        js_string(&hit.name),
        js_string(&hit.url),
        js_string(hit.status.as_str()),
        js_string(&hit.category),
        js_array(&hit.sources),
        hit.http_status
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into()),
        hit.detail
            .as_ref()
            .map(|s| js_string(s))
            .unwrap_or_else(|| "null".into()),
    )
}

fn js_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&js_string(item));
    }
    out.push(']');
    out
}

fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn write_csv(path: &Path, hits: &[Hit]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(
        file,
        "account,kind,name,url,status,category,sources,http_status"
    )?;
    for hit in hits {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{}",
            csv(&hit.account),
            csv(&hit.kind),
            csv(&hit.name),
            csv(&hit.url),
            csv(hit.status.as_str()),
            csv(&hit.category),
            csv(&hit.sources.join("+")),
            hit.http_status.map(|n| n.to_string()).unwrap_or_default()
        )?;
    }
    Ok(())
}

fn csv(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn write_txt(path: &Path, hits: &[Hit]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    for hit in hits {
        if hit.status.as_str() == "found" {
            writeln!(file, "{}", hit.url)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Hit, Status};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn csv_quotes_commas() {
        let hit = Hit {
            account: "a,b".into(),
            kind: "user".into(),
            name: "Quote \"Site\"".into(),
            url: "https://example.com/a".into(),
            status: Status::Found,
            category: "social".into(),
            sources: vec!["wmn".into(), "sherlock".into()],
            http_status: Some(200),
            detail: None,
            meta: vec![("Name".into(), "Ada".into())],
        };
        let path = std::env::temp_dir().join(format!(
            "birdlock-export-{}-{}.csv",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_csv(&path, &[hit.clone()]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert!(text.contains("\"a,b\""));
        assert!(text.contains("\"Quote \"\"Site\"\"\""));
        let json_path = path.with_extension("json");
        write_json(&json_path, &[hit]).unwrap();
        let json = fs::read_to_string(&json_path).unwrap();
        let _ = fs::remove_file(&json_path);
        assert!(json.contains("\"status\":\"found\""));
        assert!(json.contains("\"Name\":\"Ada\""));
    }
}
