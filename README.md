# logcap

A Rust crate for capturing application log output for integration testing.

The logger can have a _threaded_ scope to capture logs from a thread, or with a _process_ scope to capture across all threads.

See the [documentation](https://docs.rs/logcap/latest/logcap).

## Examples

### Capture logs within a thread

```rust
use logcap::CaptureScope;
use log::info;

#[test]
fn test_logs() {
    logcap::setup();

    // test logic outputting logs
    info!("foobar");

    // make assertions on logs
    logcap::consume(|logs| {
        assert_eq!(1, logs.len());
        assert_eq!("foobar", logs[0].body);
    });
}
```

### Capture logs across threads
Run tests with `--test_threads=1` to avoid clobbering logs between tests, or use a mutex for synchronization.

```rust
use logcap::CaptureScope;
use log::{LevelFilter,warn};

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
