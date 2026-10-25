//! Command runui keeps a folder's commands at hand and runs them: the
//! project's own (a Taskfile's tasks, package.json's scripts, a Makefile's
//! targets), the ones starred as favorites, and commands of your own. What
//! a command prints shows as it's printed; it keeps running once runui
//! quits, and the next runui finds it, its output and all, and stops it.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod cli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // An interrupt or a SIGTERM cancels the interface, as Go's
    // signal.NotifyContext cancels the context it's given.
    let canceled = Arc::new(AtomicBool::new(false));
    if args.first().is_none_or(|arg| arg != "mcp") {
        let flag = canceled.clone();
        let _ = runui_runs::on_signals(&[libc::SIGINT, libc::SIGTERM], move |_| {
            flag.store(true, Ordering::SeqCst);
            true
        });
    }
    let code = cli::run(&canceled, &args, &mut io::stdout(), &mut io::stderr());
    std::process::exit(code);
}
