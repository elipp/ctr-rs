//! Alerts to a Telegram chat, for what happens in the realm.
//!
//! Behind the `telegram` feature, which brings the http client. Without it, [`from_env`] gives no
//! [`Alerts`] and the realm runs as it did.
//!
//! Set `TG_BOT_TOKEN` and `TG_CHAT_ID` to turn them on. Without them the realm runs as it did, and
//! says so at the start. A message is never waited for: the realm hands it to [`Alerts`], and a
//! task sends it, so that a slow or unreachable Telegram cannot slow a request down.

use tokio::sync::mpsc;
use tracing::error;

/// Where the alerts go. Cheap to clone.
#[derive(Clone)]
#[cfg_attr(not(feature = "telegram"), allow(dead_code))]
pub struct Alerts(mpsc::Sender<String>);

impl Alerts {
    /// Queues an alert. It is dropped if the queue is full.
    pub fn send(&self, text: impl Into<String>) {
        if self.0.try_send(text.into()).is_err() {
            error!("telegram alert dropped: the queue is full");
        }
    }
}

/// Starts the task that sends the alerts, if the environment has the bot and the chat. Must be
/// called inside the runtime.
#[cfg(not(feature = "telegram"))]
pub fn from_env() -> Option<Alerts> {
    tracing::info!("built without the `telegram` feature: no telegram alerts");
    None
}

#[cfg(feature = "telegram")]
pub use sender::from_env;

#[cfg(feature = "telegram")]
mod sender {
    use std::sync::LazyLock;
    use std::time::Duration;

    use serde::Serialize;
    use tokio::sync::mpsc;
    use tracing::{error, info};

    use super::Alerts;

    /// How many alerts wait to be sent. When Telegram is slower than the realm, the rest are
    /// dropped.
    const QUEUE: usize = 32;

    /// The pause between messages: Telegram allows a chat about one a second.
    const SPACING: Duration = Duration::from_secs(1);

    const TIMEOUT: Duration = Duration::from_secs(10);

    /// The url of the bot, which has the token in it, so it is never logged.
    static SEND_URL: LazyLock<Option<String>> = LazyLock::new(|| {
        let token = std::env::var("TG_BOT_TOKEN").ok()?;
        Some(format!("https://api.telegram.org/bot{token}/sendMessage"))
    });

    static CHAT_ID: LazyLock<Option<i64>> = LazyLock::new(|| {
        let chat = std::env::var("TG_CHAT_ID").ok()?;
        match chat.trim().parse() {
            Ok(id) => Some(id),
            Err(e) => {
                error!("TG_CHAT_ID is not a number: {e}");
                None
            }
        }
    });

    static HTTP_CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
        // reqwest brings no crypto of its own here: it uses ring, like iroh does. Failing only
        // means that one is already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();
        match reqwest::Client::builder().timeout(TIMEOUT).build() {
            Ok(client) => Some(client),
            Err(e) => {
                error!("no http client: {e}");
                None
            }
        }
    });

    #[derive(Serialize)]
    struct Message<'a> {
        chat_id: i64,
        text: &'a str,
    }

    pub fn from_env() -> Option<Alerts> {
        let (Some(url), Some(chat_id)) = (SEND_URL.as_deref(), *CHAT_ID) else {
            info!("TG_BOT_TOKEN or TG_CHAT_ID is not set: no telegram alerts");
            return None;
        };
        let client = HTTP_CLIENT.as_ref()?;
        let (tx, mut rx) = mpsc::channel::<String>(QUEUE);
        tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                let sent = client
                    .post(url)
                    .json(&Message { chat_id, text: &text })
                    .send()
                    .await;
                match sent {
                    Ok(res) if res.status().is_success() => info!("telegram alert sent"),
                    Ok(res) => error!("telegram answered {}", res.status()),
                    // The error can hold the url, so only its kind is shown.
                    Err(e) => error!(
                        timeout = e.is_timeout(),
                        connect = e.is_connect(),
                        "telegram alert failed"
                    ),
                }
                tokio::time::sleep(SPACING).await;
            }
        });
        info!("telegram alerts on");
        Some(Alerts(tx))
    }
}
