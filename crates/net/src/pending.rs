//! A one-off network job the game polls each frame instead of waiting on: a realm list, a score
//! request.

use std::future::Future;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;

/// The job's thread or task ended without an answer (it panicked).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stopped;

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the network thread stopped")
    }
}

pub struct Pending<T> {
    result: Mutex<Receiver<T>>,
    #[cfg(not(target_arch = "wasm32"))]
    _thread: JoinHandle<()>,
}

impl<T: Send + 'static> Pending<T> {
    /// Runs `job` on a thread of its own with a runtime of its own (in a browser, as a task of
    /// the page).
    pub(crate) fn spawn<F>(name: &str, job: impl FnOnce() -> F + Send + 'static) -> Pending<T>
    where
        F: Future<Output = T> + 'static,
    {
        let (tx, result) = mpsc::channel();
        let run = move || async move {
            let _ = tx.send(job().await);
        };
        #[cfg(not(target_arch = "wasm32"))]
        let thread = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("the network runtime")
                    .block_on(run())
            })
            .expect("the network thread");
        #[cfg(target_arch = "wasm32")]
        {
            let _ = name;
            n0_future::task::spawn(run());
        }
        Pending {
            result: Mutex::new(result),
            #[cfg(not(target_arch = "wasm32"))]
            _thread: thread,
        }
    }

    /// The answer, once, when it is here.
    pub fn poll(&self) -> Option<Result<T, Stopped>> {
        match self.result.lock().unwrap().try_recv() {
            Ok(result) => Some(Ok(result)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Stopped)),
        }
    }
}
