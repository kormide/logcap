use std::{
    sync::{Mutex, MutexGuard},
    thread,
};

use log::{info, Level};
use logcap::CaptureScope;

extern crate logcap;

static SERIAL: Mutex<()> = Mutex::new(());

/// This test suite tests capturing logs across the entire
/// process. In order to isolate each test, and not rely on
/// the suite being run with --test-threads=1, use a mutex to
/// serialize each test's execution.
fn before_each<'a>() -> MutexGuard<'a, ()> {
    SERIAL.lock().unwrap()
}

/// Clear out any captured logs at the end of each test.
fn after_each() {
    logcap::clear();
}

#[test]
fn captures_log_from_single_thread() {
    let __ = before_each();

    logcap::builder()
        .scope(CaptureScope::Process)
        .setup();

    info!("foobar");

    logcap::consume(|logs| {
        assert_eq!(1, logs.len());
        assert_eq!("foobar", logs[0].body);
        assert_eq!(Level::Info, logs[0].level);
        assert_eq!("process", logs[0].target);
    });

    after_each();
}

#[test]
fn captures_logs_across_threads() {
    let __ = before_each();

    logcap::builder()
        .scope(CaptureScope::Process)
        .setup();

    info!("main thread");

    let a = thread::spawn(|| info!("thread a"));
    let b = thread::spawn(|| info!("thread b"));
    let c = thread::spawn(|| info!("thread c"));

    let _ = a.join();
    let _ = b.join();
    let _ = c.join();

    logcap::consume(|logs| {
        assert_eq!(4, logs.len());
        assert!(logs.iter().find(|log| log.body == "main thread").is_some());
        assert!(logs.iter().find(|log| log.body == "thread a").is_some());
        assert!(logs.iter().find(|log| log.body == "thread b").is_some());
        assert!(logs.iter().find(|log| log.body == "thread c").is_some());
    });

    after_each();
}
