use console::{Term, measure_text_width, truncate_str};
use std::fmt::Write;
use std::time::{Duration, Instant};

const MIN_REDRAW_INTERVAL: Duration = Duration::from_millis(30);

/// A block of lines at the bottom of the terminal that is redrawn in place, with permanent
/// lines being printed above it.
pub(crate) struct Live {
    term: Term,
    block: Vec<String>,
    drawn_widths: Vec<usize>,
    last_draw: Option<Instant>,
}

impl Live {
    pub(crate) fn new() -> Option<Self> {
        let term = Term::stderr();
        let supports_ansi = cfg!(windows)
            || std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb");

        (term.is_term() && supports_ansi).then(|| Self {
            term,
            block: vec![],
            drawn_widths: vec![],
            last_draw: None,
        })
    }

    pub(crate) fn width(&self) -> usize {
        (self.term.size().1 as usize).max(20)
    }

    pub(crate) fn set(&mut self, block: String) {
        self.block = block.lines().map(str::to_string).collect();

        let throttled = self
            .last_draw
            .is_some_and(|last| last.elapsed() < MIN_REDRAW_INTERVAL);
        if !throttled {
            let frame = self.erase() + &self.render();
            self.write(frame);
        }
    }

    pub(crate) fn println(&mut self, line: &str) {
        let frame = self.erase() + line + "\n" + &self.render();
        self.write(frame);
    }

    pub(crate) fn clear(&mut self) {
        self.block.clear();
        let frame = self.erase();
        self.write(frame);
    }

    /// Moves the cursor to the start of the drawn block and clears everything from there on.
    fn erase(&mut self) -> String {
        // When the window got narrower, the terminal rewraps lines that don't fit anymore,
        // so the height of the block needs to be computed for the current width.
        let width = self.width();
        let rows: usize = self
            .drawn_widths
            .drain(..)
            .map(|line_width| line_width.max(1).div_ceil(width))
            .sum();

        let mut out = String::from("\r");
        if rows > 1 {
            write!(out, "\x1b[{}A", rows - 1).unwrap();
        }
        out.push_str("\x1b[J");
        out
    }

    fn render(&mut self) -> String {
        let width = self.width();
        let mut out = String::new();

        for (i, line) in self.block.iter().enumerate() {
            // Never fill the last column, since terminals treat a completely filled line
            // inconsistently when it comes to wrapping.
            let line = truncate_str(line, width - 1, "…");
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&line);
            self.drawn_widths.push(measure_text_width(&line));
        }

        out
    }

    fn write(&mut self, frame: String) {
        // Synchronized output, so that supporting terminals never show a half-drawn frame.
        let _ = self
            .term
            .write_str(&format!("\x1b[?2026h{frame}\x1b[?2026l"));
        self.last_draw = Some(Instant::now());
    }
}
