use crate::harness::live::Live;
use crate::harness::runner::{Outcome, TestResult};
use console::{Style, measure_text_width, style, truncate_str};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::{Duration, Instant};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const MAX_RUNNING_LINES: usize = 5;
const SLOW: Duration = Duration::from_secs(10);
const PLAIN_PROGRESS_EVERY: usize = 250;

pub(crate) struct Ui {
    live: Option<Live>,
    total: usize,
    done: usize,
    failed: usize,
    start: Instant,
    ticks: usize,
}

impl Ui {
    pub(crate) fn new() -> Self {
        Self {
            live: Live::new(),
            total: 0,
            done: 0,
            failed: 0,
            start: Instant::now(),
            ticks: 0,
        }
    }

    pub(crate) fn line(&mut self, line: impl AsRef<str>) {
        match &mut self.live {
            Some(live) => live.println(line.as_ref()),
            None => eprintln!("{}", line.as_ref()),
        }
    }

    pub(crate) fn header(&mut self, suites: &BTreeMap<String, usize>, total: usize, jobs: usize) {
        let mut suites: Vec<_> = suites.iter().collect();
        suites.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));

        let mut details = format!("{} tests", count(total));
        if suites.len() > 1 {
            for (suite, n) in suites {
                write!(details, " · {} {suite}", count(*n)).unwrap();
            }
        }
        write!(details, " · {jobs} threads").unwrap();

        self.line(format!("{} {}", style("hayro-tests").bold(), dim(details)));
    }

    pub(crate) fn download_started(&mut self, total: usize) {
        self.total = total;
        self.done = 0;
        self.start = Instant::now();
        if self.live.is_none() {
            self.line(format!("downloading {total} files"));
        }
        self.draw_download("");
    }

    pub(crate) fn download_finished(&mut self, file: &str, error: Option<&str>) {
        self.done += 1;
        match error {
            Some(error) => self.line(format!(
                "{} {}  {}",
                style("✗").red(),
                file,
                dim(format!("download failed: {error}"))
            )),
            None => self.draw_download(file),
        }
    }

    pub(crate) fn download_done(&mut self, downloaded: usize, bytes: u64) {
        self.clear();
        if downloaded > 0 {
            self.line(format!(
                "{} {}",
                style("✓").green(),
                dim(format!(
                    "downloaded {} files ({}) in {}",
                    count(downloaded),
                    size(bytes),
                    secs(self.start.elapsed())
                ))
            ));
        }
    }

    fn draw_download(&mut self, file: &str) {
        if let Some(live) = &mut self.live {
            live.set(format!(
                "{} {}  {}{}  {}",
                style("Downloading").bold(),
                progress_bar(self.done, self.total, 30),
                style(self.done).bold(),
                dim(format!("/{}", self.total)),
                dim(file),
            ));
        }
    }

    pub(crate) fn tests_started(&mut self, total: usize) {
        self.total = total;
        self.done = 0;
        self.failed = 0;
        self.start = Instant::now();
    }

    pub(crate) fn finished(&mut self, result: &TestResult) {
        self.done += 1;

        let name = pad(&result.name, 34);
        match &result.outcome {
            Outcome::Passed | Outcome::Skipped(_) => {}
            Outcome::Failed(message) => {
                self.failed += 1;
                let (summary, location) = short_failure(message);
                let mut line = format!("{} {name}  {summary}", style("✗").red());
                if let Some(location) = location {
                    write!(line, "  {}", dim(location)).unwrap();
                }
                let line = self.fit(line);
                self.line(line);
            }
            Outcome::Created(pages) => self.line(format!(
                "{} {name}  {}",
                style("+").yellow(),
                dim(format!("created {}", plural(*pages, "reference")))
            )),
            Outcome::Updated(pages) => self.line(format!(
                "{} {name}  {}",
                style("↻").yellow(),
                dim(format!("updated {}", plural(*pages, "reference")))
            )),
        }

        if self.live.is_none()
            && self.done.is_multiple_of(PLAIN_PROGRESS_EVERY)
            && self.done < self.total
        {
            let width = self.total.to_string().len();
            self.line(format!(
                "[{:>width$}/{}] {} failed, {}",
                self.done,
                self.total,
                self.failed,
                secs(self.start.elapsed())
            ));
        }
    }

    pub(crate) fn tick(&mut self, running: &[(String, Instant)]) {
        let Some(live) = &mut self.live else { return };
        self.ticks += 1;

        let mut block = format!(
            "{}  {}{}",
            progress_bar(self.done, self.total, 40),
            style(count(self.done)).bold(),
            dim(format!("/{}", count(self.total))),
        );
        if self.failed > 0 {
            write!(
                block,
                "  {}",
                style(format!("{} failed", count(self.failed))).red()
            )
            .unwrap();
        }
        write!(block, "  {}", dim(secs(self.start.elapsed()))).unwrap();

        let mut running: Vec<_> = running
            .iter()
            .map(|(name, start)| (name, start.elapsed()))
            .collect();
        running.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

        for (i, (name, elapsed)) in running.iter().take(MAX_RUNNING_LINES).enumerate() {
            let spinner = SPINNER[(self.ticks + i * 3) % SPINNER.len()];
            let time_style = if *elapsed >= SLOW {
                Style::new().yellow()
            } else {
                Style::new().dim()
            };
            write!(
                block,
                "\n  {} {}  {}",
                time_style.apply_to(spinner),
                pad(name, 34),
                time_style.apply_to(secs(*elapsed))
            )
            .unwrap();
        }
        if running.len() > MAX_RUNNING_LINES {
            write!(
                block,
                "\n  {}",
                dim(format!("… {} more", running.len() - MAX_RUNNING_LINES))
            )
            .unwrap();
        }

        live.set(block);
    }

    pub(crate) fn clear(&mut self) {
        if let Some(live) = &mut self.live {
            live.clear();
        }
    }

    pub(crate) fn summary(&mut self, results: &[TestResult], ignored: usize) {
        self.clear();

        let mut failed: Vec<_> = results
            .iter()
            .filter_map(|r| match &r.outcome {
                Outcome::Failed(message) => Some((&r.name, message)),
                _ => None,
            })
            .collect();
        failed.sort();

        let mut out = String::new();
        if !failed.is_empty() {
            writeln!(out, "\n{}", style("Failures").red().bold()).unwrap();
            for (name, message) in &failed {
                writeln!(out, "\n{}", style(format!("✗ {name}")).red()).unwrap();
                for line in message.lines() {
                    if line.is_empty() {
                        writeln!(out).unwrap();
                    } else {
                        writeln!(out, "  {}", dim(line)).unwrap();
                    }
                }
            }
        }

        let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
        for result in results {
            if let Outcome::Skipped(reason) = &result.outcome {
                *skipped.entry(reason).or_default() += 1;
            }
        }
        if !skipped.is_empty() {
            writeln!(out).unwrap();
            for (reason, n) in &skipped {
                writeln!(out, "{}", dim(format!("{} skipped: {reason}", count(*n)))).unwrap();
            }
        }

        let mut slowest: Vec<_> = results
            .iter()
            .filter(|r| r.duration >= Duration::from_secs(1))
            .collect();
        slowest.sort_by_key(|r| std::cmp::Reverse(r.duration));
        if !slowest.is_empty() {
            let list = slowest
                .iter()
                .take(3)
                .map(|r| format!("{} {}", r.name, secs(r.duration)))
                .collect::<Vec<_>>()
                .join(" · ");
            writeln!(out, "\n{}  {}", style("Slowest").dim(), dim(list)).unwrap();
        }

        let tally = |f: fn(&Outcome) -> Option<usize>| -> usize {
            results.iter().filter_map(|r| f(&r.outcome)).sum()
        };
        let passed = tally(|o| matches!(o, Outcome::Passed).then_some(1));
        let created = tally(|o| matches!(o, Outcome::Created(_)).then_some(1));
        let updated = tally(|o| matches!(o, Outcome::Updated(_)).then_some(1));
        let skipped = tally(|o| matches!(o, Outcome::Skipped(_)).then_some(1));

        let mut parts = vec![
            style(format!("{} passed", count(passed)))
                .green()
                .to_string(),
        ];
        if !failed.is_empty() {
            parts.push(
                style(format!("{} failed", count(failed.len())))
                    .red()
                    .to_string(),
            );
        }
        if created > 0 {
            parts.push(
                style(format!("{} new", count(created)))
                    .yellow()
                    .to_string(),
            );
        }
        if updated > 0 {
            parts.push(
                style(format!("{} updated", count(updated)))
                    .yellow()
                    .to_string(),
            );
        }
        if skipped > 0 {
            parts.push(dim(format!("{} skipped", count(skipped))));
        }
        if ignored > 0 {
            parts.push(dim(format!("{} ignored", count(ignored))));
        }
        writeln!(
            out,
            "\n{}  {}",
            parts.join(&dim(" · ")),
            dim(format!("in {}", secs(self.start.elapsed())))
        )
        .unwrap();

        if !failed.is_empty() && failed.len() <= 20 {
            let release = if cfg!(debug_assertions) {
                ""
            } else {
                " --release"
            };
            let names = failed
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(
                out,
                "{} cargo test -p hayro-tests{release} -- --exact {names}",
                dim("rerun failures:")
            )
            .unwrap();
        }

        eprint!("{out}");
    }

    fn fit(&self, line: String) -> String {
        match &self.live {
            Some(live) => truncate_str(&line, live.width() - 1, "…").into_owned(),
            None => line,
        }
    }
}

fn short_failure(message: &str) -> (String, Option<&str>) {
    let mut lines = message.lines();
    let first = lines.next().unwrap_or_default();

    match first.strip_prefix("panicked at ") {
        Some(location) => {
            let reason = lines.next().unwrap_or_default();
            (format!("panicked: {reason}"), Some(location))
        }
        None => (first.to_string(), None),
    }
}

fn progress_bar(done: usize, total: usize, width: usize) -> String {
    let filled = (done * width).checked_div(total).unwrap_or(width);
    format!(
        "{}{}",
        style("━".repeat(filled)).cyan(),
        dim("─".repeat(width - filled))
    )
}

fn pad(name: &str, width: usize) -> String {
    let len = measure_text_width(name);
    format!("{name}{}", " ".repeat(width.saturating_sub(len)))
}

fn dim(text: impl std::fmt::Display) -> String {
    style(text).dim().to_string()
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn secs(duration: Duration) -> String {
    format!("{:.1}s", duration.as_secs_f64())
}

fn size(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

pub(crate) fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
