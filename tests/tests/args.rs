use std::sync::OnceLock;

static ARGS: OnceLock<Args> = OnceLock::new();

const USAGE: &str = "\
Usage: cargo test -p hayro-tests --release -- [OPTIONS] [FILTER]...

Runs every test whose name contains one of the filters (all tests if none are given).

Options:
      --exact        Only run tests whose name matches a filter exactly
      --list         List the selected tests without running them
  -u, --update       Replace the reference images of failing tests
      --store        Write the rendered output to `store/`
      --offline      Don't download missing files, skip the affected tests instead
  -j, --jobs <N>     Number of tests to run in parallel
  -h, --help         Print this help";

#[derive(Debug, Default)]
pub(crate) struct Args {
    pub(crate) filters: Vec<String>,
    pub(crate) exact: bool,
    pub(crate) list: bool,
    pub(crate) update: bool,
    pub(crate) store: bool,
    pub(crate) offline: bool,
    pub(crate) jobs: Option<usize>,
}

impl Args {
    pub(crate) fn matches(&self, name: &str) -> bool {
        if self.filters.is_empty() {
            return true;
        }

        self.filters.iter().any(|filter| {
            if self.exact {
                name == filter
            } else {
                name.contains(filter.as_str())
            }
        })
    }
}

pub(crate) fn args() -> &'static Args {
    ARGS.get().expect("arguments haven't been parsed yet")
}

pub(crate) fn init() -> &'static Args {
    ARGS.get_or_init(parse)
}

fn parse() -> Args {
    let mut args = Args::default();
    let mut iter = std::env::args().skip(1);

    while let Some(arg) = iter.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((flag, value)) if arg.starts_with("--") => (flag.to_string(), Some(value)),
            _ => (arg.clone(), None),
        };

        match flag.as_str() {
            "--exact" => args.exact = true,
            "--list" => args.list = true,
            "-u" | "--update" => args.update = true,
            "--store" => args.store = true,
            "--offline" => args.offline = true,
            "-j" | "--jobs" | "--test-threads" => {
                let value = inline_value.map(str::to_string).or_else(|| iter.next());
                match value.and_then(|v| v.parse::<usize>().ok()) {
                    Some(jobs) if jobs > 0 => args.jobs = Some(jobs),
                    _ => fail(&format!("`{flag}` expects a positive number")),
                }
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            // Flags that `cargo test` users commonly pass to libtest.
            "--nocapture" | "--show-output" | "-q" | "--quiet" | "--color" => {}
            _ if flag.starts_with('-') => fail(&format!("unknown option `{arg}`")),
            _ => args.filters.push(arg),
        }
    }

    args
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}\n\n{USAGE}");
    std::process::exit(2);
}
