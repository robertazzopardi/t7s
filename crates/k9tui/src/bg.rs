//! Background tasks that report back to the UI thread over a channel.

use core::future::Future;

use tokio::sync::mpsc::UnboundedSender;

/// Background event that reports a failed keyed task, releasing its key.
pub trait KeyedFailure {
    fn failed(key: String, message: String) -> Self;
}

/// `tokio::spawn` for work holding an in-flight `key`: if the task panics, a
/// [`KeyedFailure`] still releases the key so the resource is not left busy.
pub fn spawn_keyed<E, F>(tx: UnboundedSender<E>, key: String, task: F)
where
    E: KeyedFailure + Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let handle = tokio::spawn(task);
    tokio::spawn(async move {
        if handle.await.is_err() {
            let _ = tx.send(E::failed(
                key,
                "Background task failed unexpectedly".into(),
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    struct Failed(String);

    impl KeyedFailure for Failed {
        fn failed(key: String, _message: String) -> Self {
            Self(key)
        }
    }

    #[tokio::test]
    async fn spawn_keyed_releases_key_when_task_panics() {
        let (tx, mut rx) = unbounded_channel::<Failed>();
        spawn_keyed(tx, "k".into(), async { panic!("boom") });
        assert_eq!(rx.recv().await.map(|f| f.0).as_deref(), Some("k"));
    }
}
