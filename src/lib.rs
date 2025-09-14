//! # Overview
//! This crate contains a [`log::Log`] implementation that captures logs created by the logging
//! macros [`info!`](log::info!), [`warn!`](log::warn!), etc.
//! Similar to [testing_logger](https://docs.rs/testing_logger/latest/testing_logger), it is useful for making assertions about
//! log output, but adds the ability to capture logs of multithreaded applications.
//!
//! # Usage
//! To capture logs from a single thread, configure the logger with the [`CaptureScope::Thread`] scope. Logs will be stored in
//! thread_local storage which cannot be accessed by another thread, nor, by extension, any other test.
//!
//! To capture logs across threads, use the [`CaptureScope::Process`] scope. This comes with the caveat that
//! tests should be executed serially to avoid clobbering the logs. Rust runs tests concurrently by default.
//! This can be overridden with `--test-threads=1` or by serializing test execution using a mutex.
//!
//! ## Capture logs within a single thread
//!
//! Setup the logger in each test. If it has already been setup, it will be a no-op:
//! ```
//! use log::{info,LevelFilter};
//! use logcap::{assert_logs, CaptureScope};
//!
//! #[test]
//! fn test_logs() {
//!     logcap::builder()
//!         .scope(CaptureScope::Thread)
//!         .max_level(LevelFilter::Debug)
//!         .setup();
//!
//!     // test logic outputting logs
//!     info!("foobar");
//!     info!("moocow");
//!     info!("hello, world!");
//!
//!     // make assertions on logs
//!     assert_logs!(
//!         "foobar",
//!         "^m.*w$",
//!         (Level::Info, "hello, world!")
//!     );
//! }
//! ```
//!
//! Or just call [`setup`] which is shorthand for capturing all levels with a threaded scope.
//! ```
//! #[test]
//! fn test_logs() {
//!     logcap::setup();
//!
//!     // ...
//! }
//! ```
//!
//! ## Capture logs across multiple threads
//!
//! Run tests with `--test_threads=1` to avoid clobbering logs between tests.
//! ```
//! use log::{LevelFilter,warn};
//! use logcap::CaptureScope;
//!
//! #[test]
//! fn test_logs() {
//!     logcap::builder()
//!         .scope(CaptureScope::Process)
//!         .max_level(LevelFilter::Trace)
//!         .setup();
//!
//!     // test multi-threaded logic outputting logs
//!     warn!("warning from main thread");
//!     let _ = thread::spawn(|| warn!("warning from thread 1")).join();
//!     let _ = thread::spawn(|| warn!("warning from thread 2")).join();

//!
//!     // make assertions on logs
//!     logcap::consume(|logs| {
//!         assert_eq!(3, logs.len());
//!         assert!(logs.iter.find(|log| log.body.contains("thread 1")).is_some());
//!         assert!(logs.iter.find(|log| log.body.contains("thread 2")).is_some());
//!         assert!(logs.iter.find(|log| log.body.contains("main thread")).is_some());
//!     });
//! }
//! ```
//!
//! ## Processing logs
//!
//! Use the [`assert_logs!`] macro to assert a sequence of logs using an expressive format.
//!
//! Call [`consume`] or [`examine`] to process the captured logs and run assertions over the full
//! collection of logs.
//!
//! Call [`clear`] to remove existing logs without processing them.
//!
//! ## Ouptutting logs
//!
//! Logs can be output to stderr (default), stdout, or not at all.
//!
//! To silently capture logs:
//! ```
//! logcap::builder().output(None).setup();
//! ```
//!
//! To output logs to stdout:
//! ```
//! use logcap::LogOutput;
//!
//! logcap::builder().output(Some(LogOutput::Stdout)).setup();
//! ```

use std::{
    cell::RefCell,
    sync::{LazyLock, Mutex, OnceLock},
    time::SystemTime,
};

use log::{Level, LevelFilter, Log};
use regex::Regex;

static LOGGER: OnceLock<Logger> = OnceLock::new();
static PROCESS_LOGGER_DATA: LazyLock<Mutex<LoggerData>> =
    LazyLock::new(|| Mutex::new(LoggerData::new()));
thread_local! {static THREAD_LOGGER_DATA: RefCell<LoggerData> = RefCell::new(LoggerData::new())}

/// A log captured by calls to the logging macros ([info!](log::info), [warn!](log::warn), etc.).
#[derive(Debug)]
pub struct CapturedLog {
    /// The error level of the log.
    pub level: Level,
    /// The log's "target". Defaults to the name of the module.
    pub target: String,
    /// The log's body formatted as a string.
    pub body: String,
}

/// The scope of logs to capture.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum CaptureScope {
    /// Capture logs from the current process
    Process,
    /// Capture logs from the current thread
    Thread,
}

/// Where to output logs printed with logging macros.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LogOutput {
    /// Output logs to stderr
    Stderr,
    /// Output logs to stdout
    Stdout,
}

/// Properties of the logger. These need to be stored separately from the
/// logger because they can exist per thread or for the process, while the
/// logger itself must be static based on the design of the [`log`] facade.
struct LoggerData {
    records: Vec<CapturedLog>,
    max_level: LevelFilter,
    output: Option<LogOutput>,
}

impl LoggerData {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_level: LevelFilter::Trace,
            output: Some(LogOutput::Stderr),
        }
    }
}

#[derive(Debug)]
struct Logger {
    scope: CaptureScope,
}

impl Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        match self.scope {
            CaptureScope::Process => metadata.level() <= log::max_level(),
            CaptureScope::Thread => THREAD_LOGGER_DATA
                .with(|logger_data| metadata.level() <= logger_data.borrow().max_level),
        }
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let captured_log = CapturedLog {
            level: record.level(),
            target: record.target().to_string(),
            body: format!("{}", record.args()),
        };

        match self.scope {
            CaptureScope::Process => PROCESS_LOGGER_DATA
                .lock()
                .expect("failed to lock process logger data")
                .records
                .push(captured_log),
            CaptureScope::Thread => THREAD_LOGGER_DATA.with(|logger_data| {
                logger_data.borrow_mut().records.push(captured_log);
            }),
        }

        if let Some(output) = match self.scope {
            CaptureScope::Process => {
                PROCESS_LOGGER_DATA
                    .lock()
                    .expect("failed to lock process logger data")
                    .output
            }
            CaptureScope::Thread => {
                THREAD_LOGGER_DATA.with(|logger_data| logger_data.borrow().output)
            }
        } {
            match output {
                LogOutput::Stderr => {
                    eprintln!(
                        "[{} {} {}] {}",
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        record.level(),
                        record.target(),
                        record.args()
                    );
                }
                LogOutput::Stdout => {
                    println!(
                        "[{} {} {}] {}",
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                        record.level(),
                        record.target(),
                        record.args()
                    );
                }
            }
        }
    }

    fn flush(&self) {}
}

/// A builder to customize behaviour of the capture logger.
#[derive(Debug)]
pub struct Builder {
    scope: CaptureScope,
    max_level: LevelFilter,
    output: Option<LogOutput>,
}

impl Builder {
    /// Set the scope of logging capture to be thread-only or process-wide.
    pub fn scope(&mut self, scope: CaptureScope) -> &mut Self {
        self.scope = scope;
        self
    }

    /// Set the max level of logs to capture. Set LevelFilter::Trace to
    /// capture all logs.
    pub fn max_level(&mut self, max_level: LevelFilter) -> &mut Self {
        self.max_level = max_level;
        self
    }

    /// Configure whether captured logs are output to stdout or stderr.
    pub fn output(&mut self, output: Option<LogOutput>) -> &mut Self {
        self.output = output;
        self
    }

    pub fn setup(&self) {
        let logger = Logger { scope: self.scope };

        match LOGGER.set(logger) {
            Ok(_) => {
                log::set_logger(LOGGER.get().unwrap()).expect(
                    "cannot set logcap because another logger has already been initialized",
                );
            }
            Err(_) => {
                if LOGGER.get().unwrap().scope != self.scope {
                    panic!("logcap has already been set up with a different scope");
                }
            }
        }

        // Reset the max level or set it on a per-thread basis if the logger already exists
        match self.scope {
            CaptureScope::Process => {
                log::set_max_level(self.max_level);
                let mut logger_data = PROCESS_LOGGER_DATA.lock().unwrap();
                logger_data.max_level = self.max_level;
                logger_data.output = self.output;
            }
            CaptureScope::Thread => {
                log::set_max_level(LevelFilter::Trace);
                THREAD_LOGGER_DATA.set(LoggerData {
                    records: Vec::new(),
                    max_level: self.max_level,
                    output: self.output,
                });
            }
        };
    }
}

/// Create a configuration builder for setting up capture logging.
pub fn builder() -> Builder {
    Builder {
        scope: CaptureScope::Thread,
        max_level: LevelFilter::Trace,
        output: Some(LogOutput::Stderr),
    }
}

/// A shortcut for setting up the capture logger with default settings:
/// threaded capture, captures all levels, and outputs logs to stderr.
pub fn setup() {
    builder().setup();
}

/// Consume captured logs and perform and processing or assertions.
/// Threaded capture will capture logs in order. Logging between threads is
/// synchronized and captured first come, first serve.
pub fn consume(f: impl FnOnce(Vec<CapturedLog>)) {
    match LOGGER.get() {
        Some(logger) => match logger.scope {
            CaptureScope::Process => {
                let mut moved: Vec<CapturedLog> = Vec::new();
                moved.extend(PROCESS_LOGGER_DATA.lock().unwrap().records.drain(..));
                f(moved);
            }
            CaptureScope::Thread => {
                THREAD_LOGGER_DATA.with(|logger_data| {
                    let mut moved: Vec<CapturedLog> = Vec::new();
                    moved.extend(logger_data.borrow_mut().records.drain(..));
                    f(moved);
                });
            }
        },
        None => panic!("logcap has not been initialized"),
    }
}

/// Examine captured logs and perform and processing or assertions.
/// Threaded capture will capture logs in order. Logging between threads is
/// synchronized and captured first come, first serve.
pub fn examine(f: impl FnOnce(&Vec<CapturedLog>)) {
    match LOGGER.get() {
        Some(logger) => match logger.scope {
            CaptureScope::Process => {
                f(&PROCESS_LOGGER_DATA.lock().unwrap().records);
            }
            CaptureScope::Thread => {
                THREAD_LOGGER_DATA.with(|logger_data| {
                    f(&logger_data.borrow_mut().records);
                });
            }
        },
        None => panic!("logcap has not been initialized"),
    }
}

/// Clear any captured logs.
pub fn clear() {
    match LOGGER.get() {
        Some(logger) => match logger.scope {
            CaptureScope::Process => {
                PROCESS_LOGGER_DATA.lock().unwrap().records.clear();
            }
            CaptureScope::Thread => {
                THREAD_LOGGER_DATA.with(|logger_data| {
                    logger_data.borrow_mut().records.clear();
                });
            }
        },
        None => panic!("logcap has not been initialized"),
    }
}

/// A pattern to match against captured logs.
#[derive(Debug)]
pub struct LogPattern {
    /// The log [`Level`](`log::Level`) to match against, or `None` to disregard
    pub level: Option<Level>,
    /// A regular expression to match against the log body.
    pub body: Regex,
    /// The log target to match against, or `None` to disregard
    pub target: Option<String>,
}

impl LogPattern {
    pub fn matches(&self, log: &CapturedLog) -> bool {
        self.body.is_match(&log.body)
            && self.level.as_ref().is_none_or(|level| *level == log.level)
            && self
                .target
                .as_ref()
                .is_none_or(|target| *target == log.target)
    }
}

impl From<&str> for LogPattern {
    fn from(value: &str) -> Self {
        Self {
            body: Regex::new(value).expect("bad regex"),
            level: None,
            target: None,
        }
    }
}

impl From<(Level, &str)> for LogPattern {
    fn from(value: (Level, &str)) -> Self {
        Self {
            body: Regex::new(value.1).expect("bad regex"),
            level: Some(value.0),
            target: None,
        }
    }
}

impl From<(&str, &str)> for LogPattern {
    fn from(value: (&str, &str)) -> Self {
        Self {
            body: Regex::new(value.1).expect("bad regex"),
            level: None,
            target: Some(String::from(value.0)),
        }
    }
}

impl From<(Level, &str, &str)> for LogPattern {
    fn from(value: (Level, &str, &str)) -> Self {
        Self {
            body: Regex::new(value.2).expect("bad regex"),
            level: Some(value.0),
            target: Some(String::from(value.1)),
        }
    }
}

/// Assert a sequence of matching log patterns over captured logs.
///
/// In the simplest form, log patterns can be specified using string slices which
/// get converted into [`Regex`](regex::Regex)s and match against the log body.
/// Finer-grained assertions can be made with tuples containing additional metadata
/// to match. See the snippet below for all of the possible variants.
///
/// The sequence of patterns need not be exhaustive—they can describe a subset of the
/// captured logs but the ordering must be correct.
///
/// The macro panics if any of the log patterns fail to match, or if they fail to match in the
/// given order.
///
/// ```
/// use log::{info, Level};
/// use logcap::{assert_logs};
///
/// logcap::setup();
///
/// for i in 0..5 {
///     info!(target: "target", "foobar");
/// }
///
/// assert_logs!(
///     // body as regex (implicit)
///     "foobar",
///     // body as regex (explicit)
///     "^foobar$",
///     // level and body
///     (Level::Info, "foobar"),
///     // target and body
///     ("target", "^foobar$"),
///     // level, target, and body
///     (Level::Info, "target", "foobar")
/// );
/// ```
#[macro_export]
macro_rules! assert_logs {
    ( $( $x:expr ),* ) => {
        {
            let mut patterns: Vec<$crate::LogPattern> = Vec::new();
            $(
                patterns.push($x.into());
            )*

            $crate::examine(|logs| {
                let mut i = 0;
                'outer: for pattern in patterns {
                    while i < logs.len() {
                        if pattern.matches(&logs[i]) {
                            continue 'outer;
                        }
                        i += 1;
                    }
                    panic!("failed to match log pattern {:?}", pattern);
                }
            });
        }
    };
}

#[cfg(test)]
mod tests {
    use std::thread;

    use log::{debug, error, info, warn};

    use super::*;

    #[test]
    fn builder_defaults_to_threaded_capture() {
        let builder = super::builder();

        let CaptureScope::Thread = builder.scope else {
            panic!("expected builder to default scope to CaptureScope::Thread")
        };
    }

    #[test]
    fn setup_twice_is_noop_and_doesnt_panic() {
        super::setup();
        super::setup();
    }

    #[test]
    #[should_panic]
    fn setup_with_different_scope_panics() {
        super::builder().scope(CaptureScope::Thread).setup();
        super::builder().scope(CaptureScope::Process).setup();
    }

    #[test]
    fn captures_log() {
        super::setup();

        info!("foobar");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foobar", logs[0].body);
            assert_eq!(Level::Info, logs[0].level);
            assert_eq!("logcap::tests", logs[0].target);
        });
    }

    #[test]
    fn captures_log_with_target() {
        super::setup();

        info!(target: "foo", "bar");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("bar", logs[0].body);
            assert_eq!(Level::Info, logs[0].level);
            assert_eq!("foo", logs[0].target);
        });
    }

    #[test]
    fn captures_log_with_args() {
        super::setup();

        let greeting = "Foobar";
        info!("Hello, {greeting}!");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("Hello, Foobar!", logs[0].body);
        });
    }

    #[test]
    fn captures_logs_in_order() {
        super::setup();

        info!("foo");
        warn!("bar");
        info!("moo");
        debug!("cow");

        super::consume(|logs| {
            assert_eq!(4, logs.len());
            assert_eq!("foo", logs[0].body);
            assert_eq!("bar", logs[1].body);
            assert_eq!("moo", logs[2].body);
            assert_eq!("cow", logs[3].body);
        });
    }

    #[test]
    fn consume_logs_does_consume() {
        super::setup();

        info!("foo");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foo", logs[0].body);
        });

        super::consume(|logs| {
            assert!(logs.is_empty());
        });
    }

    #[test]
    fn examine_logs_does_not_consume() {
        super::setup();

        info!("foo");

        super::examine(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foo", logs[0].body);
        });

        super::examine(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foo", logs[0].body);
        });
    }

    #[test]
    fn consume_empty_logs() {
        super::setup();

        super::consume(|logs| {
            assert_eq!(0, logs.len());
        });
    }

    #[test]
    fn does_not_capture_log_in_other_thread() {
        super::setup();

        let _ = thread::spawn(|| info!("foobar")).join();

        super::consume(|logs| {
            assert!(logs.is_empty());
        })
    }

    #[test]
    fn captures_at_specified_level() {
        super::builder().max_level(LevelFilter::Warn).setup();

        warn!("foobar");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foobar", logs[0].body);
        });
    }

    #[test]
    fn captures_below_specified_level() {
        super::builder().max_level(LevelFilter::Warn).setup();

        error!("foobar");

        super::consume(|logs| {
            assert_eq!(1, logs.len());
            assert_eq!("foobar", logs[0].body);
        });
    }

    #[test]
    fn does_not_capture_above_specified_level() {
        super::builder().max_level(LevelFilter::Info).setup();

        debug!("foobar");

        super::consume(|logs| {
            assert!(logs.is_empty());
        });
    }

    #[test]
    fn assert_logs_single_log() {
        super::setup();

        info!("foobar");

        assert_logs!("foobar");
    }

    #[test]
    fn assert_logs_mulitple_logs() {
        super::setup();

        info!("foobar");
        info!("abc");
        info!("moocow");
        info!("hello world");
        info!("Stop. Who would cross the Bridge of Death must answer me these questions three, ere the other side he see");

        assert_logs!(
            "foobar",
            "abc",
            "moocow",
            "hello world",
            "Stop. Who would cross the Bridge of Death must answer me these questions three, ere the other side he see"
        );
    }

    #[test]
    fn assert_logs_non_exhaustive_match() {
        super::setup();

        info!("foobar");
        info!("abc");
        info!("moocow");
        info!("hello world");
        info!("Stop. Who would cross the Bridge of Death must answer me these questions three, ere the other side he see");

        assert_logs!("foobar", "moocow", "hello world");
    }

    #[test]
    fn assert_logs_does_not_consume() {
        super::setup();

        info!("foobar");

        assert_logs!("foobar");
        assert_logs!("foobar");
    }

    #[test]
    #[should_panic]
    fn assert_logs_panics_on_no_match() {
        super::setup();

        info!("foobar");

        assert_logs!("moocow");
    }

    #[test]
    #[should_panic]
    fn assert_logs_panics_on_wrong_order() {
        super::setup();

        info!("foobar");
        info!("moocow");

        assert_logs!("moocow", "foobar");
    }

    #[test]
    fn assert_logs_regex_matching() {
        super::setup();

        info!("foobar");
        info!("moocow");
        info!("Stop. Who would cross the Bridge of Death must answer me these questions three, ere the other side he see");

        assert_logs!("^foobar$", "ooco", r".*?Bridge.*?he\s");
    }

    #[test]
    fn assert_logs_input_variants() {
        super::setup();

        info!("foobar");
        warn!("abc");
        error!(target: "farm", "moocow");
        debug!(target: "programmers", "hello world");
        info!("Stop. Who would cross the Bridge of Death must answer me these questions three, ere the other side he see");

        assert_logs!(
            "^foo",
            (Level::Warn, "abc"),
            ("farm", "moocow"),
            (Level::Debug, "programmers", "hello world"),
            LogPattern {
                level: None,
                body: Regex::new(r".*?Bridge.*?he\s").expect("bad regex"),
                target: None
            }
        );
    }
}
