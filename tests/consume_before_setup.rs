extern crate logcap;

// This case must be isolated in its own integration test because
// it must run in its own process to not be affected by calls to
// `setup()` in other tests.
#[test]
#[should_panic]
fn consume_before_setup_panics() {
    logcap::consume(|_| {});
}
