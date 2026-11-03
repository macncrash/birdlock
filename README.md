# birdlock

Find a username or an email address across the social, coding, gaming, and mail sites that [Sherlock](https://github.com/sherlock-project/sherlock) and [Blackbird](https://github.com/p1ngul1n0/blackbird) already know.

One Rust binary. No crate dependencies. Requests go out through the `curl` on your machine, and TLS certificates are checked.

```text
$ birdlock --site Asciinema user blue
searching 1 user across 1 sites
[+] asciinema  https://asciinema.org/~blue
blue: 1 found, 0 missing, 0 disputed, 0 illegal, 0 waf, 0 errors, 0 unknown in 0.8s
```

## Install

You need Rust and `curl`.

```bash
git clone https://github.com/macncrash/birdlock.git
cd birdlock
cargo test
cargo build --release
./target/release/birdlock list
```

`list` loads the three files in `data/`. A `data/` directory in the current working directory is used first, then `BIRDLOCK_DATA`, then the copies shipped with the source. `birdlock update` downloads a fresh [WhatsMyName](https://github.com/WebBreacher/WhatsMyName) list.

## Usage

```bash
birdlock user torvalds
birdlock torvalds
birdlock --site GitHub --site "GitHub (User)" user torvalds
birdlock --filter 'cat=coding' user ada
birdlock --permute user jane doe
birdlock email ada@example.com
birdlock --json out.json --csv out.csv user blue
```

Found accounts print to stdout. The tally prints to stderr. NSFW sites stay out of a search unless you pass `--nsfw` or name that site with `--site`.

`user{?}name` tries the username with `_`, `-`, and `.`. `--permute` builds the separator combinations of two or more names. `--permute-all` also keeps each name on its own.

| Flag | What it does |
| --- | --- |
| `--site NAME` | Check only this site. Repeat the flag for more than one. |
| `--filter EXPR` | Keep sites matching a property test, such as `cat=social and name~git`. Values may contain spaces. |
| `--nsfw` | Include NSFW sites. |
| `--no-wmn` / `--no-sherlock` | Drop one of the username lists. |
| `--proxy URL` | Send requests through a proxy. |
| `--timeout SEC` | Per-request timeout. Default 30. |
| `--jobs N` | Concurrent requests. Default 20, capped at 64. |
| `--json PATH` `--csv PATH` `--txt PATH` | Write found rows. With `--print-all`, the files also include misses, disputes, blocks, and errors. |
| `--dump DIR` | Save response bodies for found accounts. |
| `--print-all` | Show every result, not only hits. |
| `--verbose`, `-v` | Add the HTTP status, which lists matched, and email metadata. |
| `--wmn PATH` `--sherlock PATH` `--email-list PATH` | Use a different site list. |

`birdlock --help` prints the same flags.

## How a hit is decided

A username is checked against both lists.

- Sherlock rows match the way Sherlock does: an error message in the body, an HTTP status, a response that must not redirect, or a combination of those. Each site's username pattern is applied first, so an illegal name is never requested.
- WhatsMyName rows match on an exists code and an exists string, and they reject a hit when the miss string is present. An empty miss string means the site has no miss marker. When the exists code and the miss code are the same, the body string is what separates them.
- If both lists describe the same probe (same method, URL, and body), both rules have to agree. A disagreement is a dispute, not a hit.
- Redirects are followed, except when a rule needs the original 3xx status.
- Status-code sites are fetched with GET unless the row asks for POST, PUT, or HEAD.
- Email sites keep Blackbird's methods, headers, SHA-256 rewriting, and the Eventbrite cookie pre-check.

Challenge pages from Cloudflare, CloudFront, and PerimeterX are reported as blocks rather than accounts.

## Credits

birdlock is a new program. It is not a fork of either project below, and it would not exist without them.

[Sherlock](https://github.com/sherlock-project/sherlock), by Siddharth Dushantha and the Sherlock maintainers, is the username hunt this leans on. The site manifest in `data/sherlock.json`, the message / status-code / response-url checks, and the per-site username patterns all come from that work. Thank you for keeping a careful public list of how to ask each site whether a name is taken.

[Blackbird](https://github.com/p1ngul1n0/blackbird), by Lucas Antoniaci, is the other half. It is where the email search comes from, and it is how the [WhatsMyName](https://github.com/WebBreacher/WhatsMyName) list, kept by Micah Hoffman and contributors, shows up here as a second opinion on usernames. Thank you for the email sites, the categories, and the exists/miss strings.

Reading those two codebases is also why birdlock has its own checker. A few behaviors in the originals fight their own site lists: TLS verification turned off, username POST bodies that are never sent, an email test that cannot succeed when both status codes match, empty miss strings that can never hit, and HEAD requests to sites that only answer GET. The rules above are the corrected ones. The site data is still theirs.

## Site data

| File | From | License |
| --- | --- | --- |
| `data/sherlock.json` | [sherlock-project/sherlock](https://github.com/sherlock-project/sherlock) | MIT |
| `data/wmn-data.json` | [WebBreacher/WhatsMyName](https://github.com/WebBreacher/WhatsMyName) | CC BY-SA 4.0 |
| `data/email-data.json` | [p1ngul1n0/blackbird](https://github.com/p1ngul1n0/blackbird) | GPL-3.0, as data shipped beside this program |

`NOTICE` has the longer attribution. The Rust source is MIT, copyright macncrash.

## License

[MIT](LICENSE) for the program. Copyright (c) 2026 macncrash.

The three JSON files stay under the upstream licenses in the table above.
