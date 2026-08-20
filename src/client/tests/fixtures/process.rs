use std::io::Write;
use std::time::Duration;

fn main() {
    #[cfg(unix)]
    {
        unsafe {
            let previous = libc::signal(libc::SIGTERM, libc::SIG_IGN);
            assert_ne!(previous, libc::SIG_ERR, "failed to install SIGTERM handler");
        }
    }

    println!("ready");
    std::io::stdout()
        .flush()
        .expect("failed to flush readiness signal");

    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
