//! Every Sherlock username pattern, checked against Python's `re.search`.

use birdlock::re;

#[test]
fn sherlock_patterns_match_python() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/regex_vectors.tsv");
    let text = std::fs::read_to_string(path).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0usize;
    for (line_no, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let pattern = parts.next().unwrap();
        let name = parts.next().unwrap();
        let expect = parts.next().unwrap() == "1";
        let compiled = match re::compile(pattern) {
            Ok(p) => p,
            Err(err) => {
                failures.push(format!("{}: compile {pattern}: {err}", line_no + 1));
                continue;
            }
        };
        let got = re::is_match(&compiled, name);
        checked += 1;
        if got != expect {
            failures.push(format!(
                "{}: /{pattern}/ on {name:?} got {got} expected {expect}",
                line_no + 1
            ));
            if failures.len() >= 12 {
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "checked {checked}, mismatches:\n{}",
        failures.join("\n")
    );
}
