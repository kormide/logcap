//! # Overview
//! This crate contains a [`log::Log`] implementation that captures logs from the logging
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
//! use logcap::CaptureScope;
//! use log::{info,LevelFilter};
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
//!
//!     // make assertions on logs
//!     logcap::consume(|logs| {
//!         assert_eq!(1, logs.len());
//!         assert_eq!("foobar", logs[0].body);
//!     });
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
//! use logcap::CaptureScope;
//! use log::{LevelFilter,warn};
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
//! Call [`consume`] to process the captured logs and run assertions. The logs will be cleared after the closure runs.
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
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use log::{Level, LevelFilter, Log};

static LOGGER: OnceLock<Logger> = OnceLock::new();
thread_local! {static THREAD_RECORDS: RefCell<Vec<CapturedLog>> = RefCell::new(Vec::new())}
static PROCESS_RECORDS: Mutex<Vec<CapturedLog>> = Mutex::new(Vec::new());

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

#[derive(Debug)]
struct Logger {
    scope: CaptureScope,
    max_level: LevelFilter,
    output: Option<LogOutput>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= self.max_level
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
            CaptureScope::Process => PROCESS_RECORDS
                .lock()
                .expect("failed to lock log records")
                .push(captured_log),
            CaptureScope::Thread => THREAD_RECORDS.with(|records| {
                records.borrow_mut().push(captured_log);
            }),
        }

        if let Some(output) = self.output {
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
        let logger = Logger {
            scope: self.scope,
            max_level: self.max_level,
            output: self.output,
        };

        match LOGGER.set(logger) {
            Ok(_) => {
                log::set_logger(LOGGER.get().unwrap()).expect(
                    "cannot set logcap because another logger has already been initialized",
                );
                log::set_max_level(self.max_level);
            }
            Err(_) => {
                if LOGGER.get().unwrap().scope != self.scope {
                    panic!("logcap has already been set up with a different scope");
                }
            }
        }
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
/// Threaded capture will capture logs in order. Logging between threads
/// synchronized and captured first come, first serve.
pub fn consume(f: impl FnOnce(Vec<CapturedLog>)) {
    match LOGGER.get() {
        Some(logger) => match logger.scope {
            CaptureScope::Process => {
                let mut records = PROCESS_RECORDS.lock().unwrap();
                let mut moved: Vec<CapturedLog> = Vec::new();
                moved.extend(records.drain(..));
                f(moved);
            }
            CaptureScope::Thread => {
                THREAD_RECORDS.with(|records| {
                    let mut records = records.borrow_mut();
                    let mut moved: Vec<CapturedLog> = Vec::new();
                    moved.extend(records.drain(..));
                    f(moved);
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
                let mut records = PROCESS_RECORDS.lock().unwrap();
                records.clear();
            }
            CaptureScope::Thread => {
                THREAD_RECORDS.with(|records| {
                    let mut records = records.borrow_mut();
                    records.clear();
                });
            }
        },
        None => panic!("logcap has not been initialized"),
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use log::{debug, info, warn};

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
}
