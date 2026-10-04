use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use armonik_transport::grpc::ReadGate;
use tokio::sync::watch;

use crate::ledger::Ledger;

/// A call's turn to read: the message it read last is delivered, and the runtime's ceiling admits
/// a read. The engine waits on it before each message, so the decision is taken before the read
/// and the message is charged once it arrives decoded - the two steps level 1 has.
pub(crate) struct ReadTurn {
    ledger: Arc<Ledger>,
    /// Whether no message the call read is still to be delivered. One at a time is what bounds
    /// the calls admitted together to a message each past the first threshold.
    free: watch::Sender<bool>,
}

impl ReadTurn {
    pub(crate) fn new(ledger: Arc<Ledger>) -> Self {
        Self {
            ledger,
            free: watch::channel(true).0,
        }
    }

    /// The message read last was delivered: staged for the host, with its credit.
    pub(crate) fn delivered(&self) {
        self.free.send_replace(true);
    }
}

impl ReadTurn {
    async fn taken(&self, admitted: bool) {
        let _ = self.free.subscribe().wait_for(|free| *free).await;
        if admitted {
            self.ledger.read_admitted().await;
        }
        self.free.send_replace(false);
    }
}

impl ReadGate for ReadTurn {
    fn admitted(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.taken(true))
    }

    /// Without the ledger's admission: behind it, a status would wait for its message to be given
    /// back, and a host that gives a one-response call's message back with its status never would.
    fn turn(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.taken(false))
    }
}

impl std::fmt::Debug for ReadTurn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadTurn")
            .field("free", &*self.free.borrow())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_call_reads_its_next_message_only_once_the_last_is_delivered() {
        let turn = ReadTurn::new(Arc::new(Ledger::new(64, 0).expect("a valid ledger")));

        turn.admitted().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), turn.admitted())
                .await
                .is_err(),
            "the message read is not delivered yet"
        );

        turn.delivered();
        tokio::time::timeout(Duration::from_secs(10), turn.admitted())
            .await
            .expect("delivered, the call reads again");
    }
}
