//! birdlock: username and email account search.
//!
//! Site lists come from WhatsMyName and Sherlock. Email sites come from
//! Blackbird. HTTP is the system curl binary, with TLS verification left on.

use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};
use std::time::Duration;

use birdlock::catalog::{self, Paths};
use birdlock::export;
use birdlock::http::CurlClient;
use birdlock::query;
use birdlock::scan::{self, Hit, Options, Status};

const WMN_URL: &str =
    "https://raw.githubusercontent.com/WebBreacher/WhatsMyName/main/wmn-data.json";
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:128.0) Gecko/20100101 Firefox/128.0";

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("birdlock: {err}");
            ExitCode::from(1)
        }
    }
}

fn real_main() -> Result<(), String> {
    let args = Args::parse(env::args().skip(1))?;
    match args.command {
        Command::Help => {
            print!("{}", HELP);
            Ok(())
        }
        Command::Update => update_wmn(&resolve("wmn-data.json", args.wmn.as_deref())),
        Command::List => {
            let catalog = load_catalog(&args)?;
            for warning in &catalog.warnings {
                eprintln!("birdlock: {warning}");
            }
            let nsfw = catalog.user_sites.iter().filter(|s| s.nsfw).count();
            println!(
                "user sites: {} (wmn {}, sherlock {}, merged {}, nsfw {})",
                catalog.user_sites.len(),
                catalog.wmn_count,
                catalog.sherlock_count,
                catalog.merged_count,
                nsfw
            );
            println!("email sites: {}", catalog.email_sites.len());
            if !catalog.warnings.is_empty() {
                println!("warnings: {}", catalog.warnings.len());
            }
            Ok(())
        }
        Command::User(ref names) => scan_accounts(&args, names, "user"),
        Command::Email(ref emails) => scan_accounts(&args, emails, "email"),
    }
}

fn scan_accounts(args: &Args, inputs: &[String], kind: &str) -> Result<(), String> {
    if inputs.is_empty() {
        return Err(format!("pass at least one {kind}"));
    }
    let catalog = load_catalog(args)?;
    for warning in &catalog.warnings {
        eprintln!("birdlock: {warning}");
    }
    let filter = match &args.filter {
        Some(text) => Some(query::parse_filter(text)?),
        None => None,
    };
    let pool = if kind == "email" {
        &catalog.email_sites
    } else {
        &catalog.user_sites
    };
    let sites = query::select_sites(pool, &args.sites, args.nsfw, filter.as_ref())?;
    if sites.is_empty() {
        return Err("no sites to search".into());
    }
    let accounts = if kind == "user" {
        let expanded: Vec<String> = inputs.iter().flat_map(|n| query::expand_brace(n)).collect();
        if args.permute || args.permute_all {
            if expanded.len() < 2 && !args.permute_all {
                return Err("permute needs at least two usernames".into());
            }
            query::permute(&expanded, args.permute_all)
        } else {
            expanded
        }
    } else {
        inputs.to_vec()
    };
    if accounts.is_empty() {
        return Err(format!("no {kind}s to search"));
    }
    eprintln!(
        "searching {} {kind}{} across {} sites",
        accounts.len(),
        if accounts.len() == 1 { "" } else { "s" },
        sites.len()
    );

    let opts = Options {
        timeout: Duration::from_secs(args.timeout),
        jobs: args.jobs,
        proxy: args.proxy.clone(),
        user_agent: args.user_agent.clone(),
        verbose: args.verbose,
        print_all: args.print_all,
        color: args.color && io::stdout().is_terminal(),
        dump_dir: args.dump.clone(),
        print: true,
    };
    let client = CurlClient;
    let mut all: Vec<Hit> = Vec::new();
    for account in &accounts {
        let hits = scan::run(&sites, account, kind, &client, &opts);
        all.extend(hits);
    }
    let exported: Vec<Hit> = if args.print_all {
        all.into_iter()
            .filter(|h| h.status != Status::Illegal)
            .collect()
    } else {
        all.into_iter()
            .filter(|h| h.status == Status::Found)
            .collect()
    };
    if let Some(path) = &args.json {
        export::write_json(path, &exported)
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    if let Some(path) = &args.csv {
        export::write_csv(path, &exported)
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    if let Some(path) = &args.txt {
        export::write_txt(path, &exported)
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }
    Ok(())
}

fn load_catalog(args: &Args) -> Result<birdlock::catalog::Catalog, String> {
    let paths = match &args.command {
        Command::Email(_) => Paths {
            wmn: None,
            sherlock: None,
            email: Some(resolve("email-data.json", args.email.as_deref())),
            load_wmn: false,
            load_sherlock: false,
        },
        Command::List => Paths {
            wmn: Some(resolve("wmn-data.json", args.wmn.as_deref())),
            sherlock: Some(resolve("sherlock.json", args.sherlock.as_deref())),
            email: Some(resolve("email-data.json", args.email.as_deref())),
            load_wmn: true,
            load_sherlock: true,
        },
        Command::User(_) => {
            if args.no_wmn && args.no_sherlock {
                return Err("both username lists are disabled".into());
            }
            Paths {
                wmn: if args.no_wmn {
                    None
                } else {
                    Some(resolve("wmn-data.json", args.wmn.as_deref()))
                },
                sherlock: if args.no_sherlock {
                    None
                } else {
                    Some(resolve("sherlock.json", args.sherlock.as_deref()))
                },
                email: None,
                load_wmn: !args.no_wmn,
                load_sherlock: !args.no_sherlock,
            }
        }
        Command::Update | Command::Help => return Err("no catalog for this command".into()),
    };
    catalog::load(&paths)
}

fn resolve(name: &str, explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    let cwd = PathBuf::from("data").join(name);
    if cwd.exists() {
        return cwd;
    }
    if let Ok(dir) = env::var("BIRDLOCK_DATA") {
        let from_env = PathBuf::from(dir).join(name);
        if from_env.exists() {
            return from_env;
        }
    }
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(name);
    if bundled.exists() {
        return bundled;
    }
    bundled
}

fn update_wmn(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let status = Process::new("curl")
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--globoff")
        .arg("--max-time")
        .arg("60")
        .arg("-o")
        .arg(path)
        .arg(WMN_URL)
        .status()
        .map_err(|e| format!("curl failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("download failed ({status})"));
    }
    eprintln!("updated {}", path.display());
    Ok(())
}

struct Args {
    command: Command,
    wmn: Option<PathBuf>,
    sherlock: Option<PathBuf>,
    email: Option<PathBuf>,
    sites: Vec<String>,
    filter: Option<String>,
    nsfw: bool,
    no_wmn: bool,
    no_sherlock: bool,
    proxy: Option<String>,
    timeout: u64,
    jobs: usize,
    user_agent: String,
    json: Option<PathBuf>,
    csv: Option<PathBuf>,
    txt: Option<PathBuf>,
    dump: Option<PathBuf>,
    verbose: bool,
    print_all: bool,
    color: bool,
    permute: bool,
    permute_all: bool,
}

enum Command {
    User(Vec<String>),
    Email(Vec<String>),
    List,
    Update,
    Help,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args {
            command: Command::Help,
            wmn: None,
            sherlock: None,
            email: None,
            sites: Vec::new(),
            filter: None,
            nsfw: false,
            no_wmn: false,
            no_sherlock: false,
            proxy: None,
            timeout: 30,
            jobs: 20,
            user_agent: USER_AGENT.into(),
            json: None,
            csv: None,
            txt: None,
            dump: None,
            verbose: false,
            print_all: false,
            color: env::var_os("NO_COLOR").is_none(),
            permute: false,
            permute_all: false,
        };
        let mut positionals = Vec::new();
        let tokens: Vec<String> = raw.collect();
        let mut i = 0;
        while i < tokens.len() {
            let token = tokens[i].clone();
            if token == "--" {
                positionals.extend(tokens[i + 1..].iter().cloned());
                break;
            }
            if let Some(rest) = token.strip_prefix("--") {
                let (flag, inline) = match rest.split_once('=') {
                    Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
                    None => (rest.to_string(), None),
                };
                let mut take = |name: &str| -> Result<String, String> {
                    if let Some(value) = inline.clone() {
                        Ok(value)
                    } else {
                        i += 1;
                        tokens
                            .get(i)
                            .cloned()
                            .ok_or_else(|| format!("--{name} needs a value"))
                    }
                };
                match flag.as_str() {
                    "help" => args.command = Command::Help,
                    "wmn" => args.wmn = Some(PathBuf::from(take("wmn")?)),
                    "sherlock" => args.sherlock = Some(PathBuf::from(take("sherlock")?)),
                    "email-list" => args.email = Some(PathBuf::from(take("email-list")?)),
                    "site" => args.sites.push(take("site")?),
                    "filter" => args.filter = Some(take("filter")?),
                    "proxy" => args.proxy = Some(take("proxy")?),
                    "timeout" => {
                        args.timeout = parse_u64(&take("timeout")?, "--timeout")?;
                        if args.timeout == 0 {
                            return Err("--timeout must be at least 1".into());
                        }
                    }
                    "jobs" => {
                        args.jobs = parse_u64(&take("jobs")?, "--jobs")? as usize;
                        if args.jobs == 0 {
                            return Err("--jobs must be at least 1".into());
                        }
                    }
                    "user-agent" => args.user_agent = take("user-agent")?,
                    "json" => args.json = Some(PathBuf::from(take("json")?)),
                    "csv" => args.csv = Some(PathBuf::from(take("csv")?)),
                    "txt" => args.txt = Some(PathBuf::from(take("txt")?)),
                    "dump" => args.dump = Some(PathBuf::from(take("dump")?)),
                    "nsfw" => args.nsfw = true,
                    "no-wmn" => args.no_wmn = true,
                    "no-sherlock" => args.no_sherlock = true,
                    "verbose" | "v" => args.verbose = true,
                    "print-all" => args.print_all = true,
                    "no-color" => args.color = false,
                    "permute" => args.permute = true,
                    "permute-all" => args.permute_all = true,
                    other => return Err(format!("unknown flag --{other}")),
                }
            } else if token == "-h" {
                args.command = Command::Help;
            } else if token == "-v" {
                args.verbose = true;
            } else if let Some(rest) = token.strip_prefix('-') {
                if rest.len() == 1 {
                    return Err(format!("unknown flag -{rest}"));
                }
                return Err(format!("unknown flag {token}"));
            } else {
                positionals.push(token);
            }
            i += 1;
        }

        if matches!(args.command, Command::Help) && positionals.is_empty() && env_args_were_empty()
        {
            args.command = Command::Help;
            return Ok(args);
        }
        if matches!(args.command, Command::Help) && positionals.is_empty() {
            // `--help` was set, or the user passed only flags.
            let only_help = tokens.iter().any(|t| t == "--help" || t == "-h");
            if only_help || tokens.is_empty() {
                return Ok(args);
            }
        }
        if positionals.is_empty() {
            if matches!(args.command, Command::Help) {
                return Ok(args);
            }
            return Err("pass a command or a username. Try `birdlock --help`.".into());
        }
        let command = match positionals[0].as_str() {
            "help" => Command::Help,
            "list" => Command::List,
            "update" => Command::Update,
            "user" => Command::User(positionals[1..].to_vec()),
            "email" => Command::Email(positionals[1..].to_vec()),
            _ => Command::User(positionals),
        };
        if matches!(args.command, Command::Help)
            && tokens.iter().any(|t| t == "--help" || t == "-h")
        {
            args.command = Command::Help;
        } else {
            args.command = command;
        }
        match &args.command {
            Command::User(names) if names.is_empty() => Err("pass at least one username".into()),
            Command::Email(emails) if emails.is_empty() => Err("pass at least one email".into()),
            _ => Ok(args),
        }
    }
}

fn env_args_were_empty() -> bool {
    env::args().skip(1).next().is_none()
}

fn parse_u64(value: &str, flag: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{flag} expects a number, got {value}"))
}

const HELP: &str = "\
birdlock — find accounts by username or email

Usage:
  birdlock [flags] user NAME [NAME...]
  birdlock [flags] email ADDRESS [ADDRESS...]
  birdlock [flags] NAME [NAME...]
  birdlock list
  birdlock update
  birdlock --help

Usernames are checked against the WhatsMyName list and the Sherlock list.
Entries that probe the same URL are merged, and a hit is kept only when both
rules agree. Emails use Blackbird's email list. A name like user{?}name is
expanded with _, -, and .

Flags:
  --site NAME         Check only this site. Repeat for more than one.
  --filter EXPR       Property filter, for example cat=social and name~git
  --nsfw              Include NSFW sites. They are off unless named with --site.
  --no-wmn            Skip the WhatsMyName list.
  --no-sherlock       Skip the Sherlock list.
  --permute           Permute two or more usernames, skipping the parts alone.
  --permute-all       Permute, and also keep each part on its own.
  --proxy URL         Send requests through this proxy.
  --timeout SEC       Per-request timeout. Default 30.
  --jobs N            Concurrent requests. Default 20.
  --user-agent TEXT   Override the default browser user agent.
  --json PATH         Write results as JSON.
  --csv PATH          Write results as CSV.
  --txt PATH          Write found URLs, one per line.
  --dump DIR          Save response bodies for found accounts.
  --print-all         Print and export misses, disputes, blocks, and errors.
  --verbose, -v       Print status codes, sources, metadata, and errors.
  --no-color          Plain output.
  --wmn PATH          WhatsMyName wmn-data.json
  --sherlock PATH     Sherlock data.json
  --email-list PATH   Blackbird email-data.json

list prints how many sites loaded. update downloads a fresh WhatsMyName file.
HTTP uses the system curl and checks TLS certificates.
";
