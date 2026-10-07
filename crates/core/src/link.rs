//! The socket to the server. It reconnects by itself, waiting longer each time it fails, and
//! tells the core when it opened so the core can say hello and resend what waits.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use mail_protocol::wire::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as Frame;

pub enum LinkEvent {
    Opened,
    Message(ServerMessage),
    Closed(Option<String>),
}

pub struct Link {
    out: mpsc::UnboundedSender<ClientMessage>,
    task: JoinHandle<()>,
}

const PING_EVERY: Duration = Duration::from_secs(25);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

pub fn socket_url(server: &str) -> String {
    let server = server.trim_end_matches('/');
    let url = match server.strip_prefix("https://") {
        Some(rest) => format!("wss://{rest}"),
        None => format!("ws://{}", server.strip_prefix("http://").unwrap_or(server)),
    };
    format!("{url}/sync")
}

impl Link {
    pub fn start(server: &str, events: impl Fn(LinkEvent) + Send + Sync + 'static) -> Self {
        let url = socket_url(server);
        let (out, mut outbox) = mpsc::unbounded_channel::<ClientMessage>();
        let task = tokio::spawn(async move {
            let mut backoff = Duration::from_millis(500);
            loop {
                match tokio_tungstenite::connect_async(url.as_str()).await {
                    Ok((socket, _)) => {
                        backoff = Duration::from_millis(500);
                        while outbox.try_recv().is_ok() {}
                        events(LinkEvent::Opened);
                        let (mut sink, mut stream) = socket.split();
                        let mut ping = tokio::time::interval(PING_EVERY);
                        ping.tick().await;
                        let error = loop {
                            tokio::select! {
                                message = outbox.recv() => {
                                    let Some(message) = message else { return };
                                    let Ok(text) = serde_json::to_string(&message) else { continue };
                                    if let Err(error) = sink.send(Frame::text(text)).await {
                                        break Some(error.to_string());
                                    }
                                }
                                frame = stream.next() => match frame {
                                    Some(Ok(Frame::Text(text))) => match serde_json::from_str::<ServerMessage>(&text) {
                                        Ok(message) => events(LinkEvent::Message(message)),
                                        Err(error) => tracing::warn!("ignored a message from the server: {error}"),
                                    },
                                    Some(Ok(Frame::Close(_))) | None => break None,
                                    Some(Err(error)) => break Some(error.to_string()),
                                    Some(Ok(_)) => {}
                                },
                                _ = ping.tick() => {
                                    let text = serde_json::to_string(&ClientMessage::Ping).unwrap_or_default();
                                    if let Err(error) = sink.send(Frame::text(text)).await {
                                        break Some(error.to_string());
                                    }
                                }
                            }
                        };
                        events(LinkEvent::Closed(error));
                    }
                    Err(error) => events(LinkEvent::Closed(Some(error.to_string()))),
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });
        Self { out, task }
    }

    pub fn send(&self, message: ClientMessage) {
        let _ = self.out.send(message);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn socket_urls_follow_the_scheme() {
        assert_eq!(super::socket_url("https://mail.example.com/"), "wss://mail.example.com/sync");
        assert_eq!(super::socket_url("http://localhost:3000"), "ws://localhost:3000/sync");
    }
}
