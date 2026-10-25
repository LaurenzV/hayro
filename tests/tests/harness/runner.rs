use crate::harness::ui::Ui;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, PanicHookInfo};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

type TestFn = Box<dyn Fn() -> Outcome + Send + Sync>;

pub(crate) struct Test {
    pub(crate) name: String,
    pub(crate) files: Vec<String>,
    run: TestFn,
}

impl Test {
    pub(crate) fn new(
        name: impl Into<String>,
        files: Vec<String>,
        run: impl Fn() -> Outcome + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            files,
            run: Box::new(run),
        }
    }

    pub(crate) fn unit(name: &str, run: fn()) -> Self {
        Self::new(name, vec![], move || {
            run();
            Outcome::Passed
        })
    }

    pub(crate) fn suite(&self) -> &str {
        self.name
            .split_once("::")
            .map_or("other", |(suite, _)| suite)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Outcome {
    Passed,
    New(usize),
    Changed(usize),
    Skipped(String),
    Failed(String),
}

pub(crate) struct TestResult {
    pub(crate) name: String,
    pub(crate) outcome: Outcome,
    pub(crate) duration: Duration,
}

thread_local! {
    static IS_WORKER: Cell<bool> = const { Cell::new(false) };
    static CURRENT_TEST: RefCell<Option<String>> = const { RefCell::new(None) };
    static PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(crate) fn current_test() -> Option<String> {
    CURRENT_TEST.with(|test| test.borrow().clone())
}

// Panics on threads spawned by a test itself. They resurface on the worker as a generic
// "a scoped thread panicked", so the original message is kept here.
static THREAD_PANIC: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn run(tests: Vec<Test>, jobs: usize, ui: &mut Ui) -> Vec<TestResult> {
    let total = tests.len();
    let jobs = jobs.min(total).max(1);
    let next = AtomicUsize::new(0);
    let running: Vec<Mutex<Option<(String, Instant)>>> =
        (0..jobs).map(|_| Mutex::new(None)).collect();
    let mut results = Vec::with_capacity(total);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            default_hook(info);
        } else {
            record_panic(info);
        }
    }));

    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();

        for slot in &running {
            let sender = sender.clone();
            let (next, tests) = (&next, &tests);
            scope.spawn(move || {
                IS_WORKER.set(true);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(test) = tests.get(index) else { break };

                    let start = Instant::now();
                    *lock(slot) = Some((test.name.clone(), start));
                    let outcome = run_one(test);
                    *lock(slot) = None;

                    let result = TestResult {
                        name: test.name.clone(),
                        outcome,
                        duration: start.elapsed(),
                    };
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        loop {
            match receiver.recv_timeout(Duration::from_millis(80)) {
                Ok(result) => {
                    ui.finished(&result);
                    results.push(result);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            let in_flight: Vec<_> = running
                .iter()
                .filter_map(|slot| lock(slot).clone())
                .collect();
            ui.tick(&in_flight);
        }
    });

    drop(std::panic::take_hook());

    results
}

fn run_one(test: &Test) -> Outcome {
    CURRENT_TEST.with(|current| *current.borrow_mut() = Some(test.name.clone()));
    PANIC.with(|panic| panic.borrow_mut().take());
    lock(&THREAD_PANIC).take();

    match std::panic::catch_unwind(AssertUnwindSafe(|| (test.run)())) {
        Ok(outcome) => outcome,
        Err(payload) => {
            let message = PANIC
                .with(|panic| panic.borrow_mut().take())
                .unwrap_or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "panicked".to_string())
                });
            match lock(&THREAD_PANIC).take() {
                Some(cause) => Outcome::Failed(format!("{message}\n\nspawned thread {cause}")),
                None => Outcome::Failed(message),
            }
        }
    }
}

fn record_panic(info: &PanicHookInfo<'_>) {
    let message = info.payload_as_str().unwrap_or("panicked");
    let message = match info.location() {
        Some(location) => format!("panicked at {location}\n{message}"),
        None => format!("panicked\n{message}"),
    };
    if IS_WORKER.get() {
        PANIC.with(|panic| *panic.borrow_mut() = Some(message));
    } else {
        *lock(&THREAD_PANIC) = Some(message);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
