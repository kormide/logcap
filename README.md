# logcap

A Rust crate for capturing application log output for integration testing.

The logger can have a _threaded_ scope to capture logs from a thread, or with a _process_ scope to capture across all threads.

See the [documentation](https://docs.rs/logcap/latest/logcap) and [crate](https://crates.io/crates/logcap).

## Examples

### Capture logs within a thread

```rust
use log::{info, Level};
use logcap::assert_logs;

#[test]
fn test_logs() {
    logcap::setup();

    // test logic outputting logs
    info!("foobar");
    info!("moocow");
    info!("hello, world!");

    // make assertions on logs
    assert_logs!(
        "foobar",
        "^m.*w$",
        (Level::Info, "hello, world!")
    );
}
```

### Capture logs across threads
Run tests with `--test_threads=1` to avoid clobbering logs between tests, or use a mutex for synchronization.

```rust
use log::{LevelFilter,warn};
use logcap::{assert_logs, CaptureScope};

#[test]
fn test_logs() {
    logcap::builder()
        .scope(CaptureScope::Process)
        .max_level(LevelFilter::Trace)
        .setup();

    // test multi-threaded logic outputting logs
    warn!("warning from main thread");
    let _ = thread::spawn(|| warn!("warning from thread 1")).join();
    let _ = thread::spawn(|| warn!("warning from thread 2")).join();

    // make assertions on logs
    logcap::consume(|logs| {
        assert_eq!(3, logs.len());
        assert!(logs.iter.find(|log| log.body.contains("thread 1")).is_some());
        assert!(logs.iter.find(|log| log.body.contains("thread 2")).is_some());
        assert!(logs.iter.find(|log| log.body.contains("main thread")).is_some());
    });
}
```
