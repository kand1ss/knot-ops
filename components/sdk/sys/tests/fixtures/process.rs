use std::time::Duration;

fn main() {
    // Keep the process alive long enough for integration tests
    // to bind to it and terminate it.
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
