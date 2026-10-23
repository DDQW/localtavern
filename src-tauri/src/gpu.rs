//! One graphics card shared by Ollama's chat model and a local image server. When Settings unload
//! Ollama's model before a picture, the render holds the card until it's done, and Ollama requests
//! wait for it. Otherwise the next message loads the model straight back next to the image model,
//! both spill into system RAM and crawl - a Wan video takes minutes, plenty of time to send one.

use std::sync::Arc;

use tokio::sync::watch;

/// How many renders hold the card. Clones share it; a new one is never held.
#[derive(Clone)]
pub struct GpuGate(Arc<watch::Sender<usize>>);

impl Default for GpuGate {
    fn default() -> Self {
        Self(Arc::new(watch::channel(0).0))
    }
}

impl GpuGate {
    /// Holds the card until the returned guard is dropped.
    pub fn hold(&self) -> GpuHold {
        self.0.send_modify(|held| *held += 1);
        GpuHold(self.0.clone())
    }

    pub fn is_held(&self) -> bool {
        *self.0.borrow() > 0
    }

    /// Returns once no render holds the card.
    pub async fn free(&self) {
        let _ = self.0.subscribe().wait_for(|held| *held == 0).await;
    }
}

pub struct GpuHold(Arc<watch::Sender<usize>>);

impl Drop for GpuHold {
    fn drop(&mut self) {
        self.0.send_modify(|held| *held -= 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn waits_until_every_hold_is_dropped() {
        let gate = GpuGate::default();
        assert!(!gate.is_held());
        gate.free().await;

        let (first, second) = (gate.hold(), gate.hold());
        assert!(gate.is_held());
        let waiting = tokio::spawn({
            let gate = gate.clone();
            async move { gate.free().await }
        });
        drop(first);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished());

        drop(second);
        tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap().unwrap();
        assert!(!gate.is_held());
    }
}
