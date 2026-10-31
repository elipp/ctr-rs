//! What a panic leaves behind: with no console window (a release build on Windows) it would go
//! unseen, so the message and a backtrace are written to `crash.log` (see `paths.rs`), and a
//! panic of the main thread is also shown in a dialog on Windows. A panic of another thread
//! (the network's, say) only goes to the log; the program may well go on without it.
//!
//! In a browser there are no files, and a panic ends the program whichever task it is in: the
//! message goes to the browser's console, and to the page (`web.rs`), which says the game has
//! crashed and offers to start it again.

#[cfg(target_arch = "wasm32")]
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        crate::web::crashed(&info.to_string());
    }));
}

#[cfg(not(target_arch = "wasm32"))]
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let path = crate::paths::crash_log();
        let text = format!(
            "{info}\n\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, &text);
        #[cfg(all(windows, not(debug_assertions)))]
        if std::thread::current().name() == Some("main") {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("ctr-rs")
                .set_description(format!("ctr-rs crashed: {info}\n\nDetails: {}", path.display()))
                .show();
        }
    }));
}
