mod cli;
mod config;
mod cover;
mod library;
mod online;
mod organize;
mod tags;
mod theme;
mod tui;

fn main() {
    // Restore the default SIGPIPE behavior so piping into `head` exits
    // quietly instead of panicking on a failed stdout write.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    if let Err(err) = cli::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
