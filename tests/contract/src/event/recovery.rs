#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

// --- Contract Definitions ---

#[derive(Debug, PartialEq)]
enum RecoveryState {
    Normal,
    GapDetected,
    Recovering,
    #[allow(dead_code)]
    Skipped, // Poison Marker
    RebuildRequired,
}

// Mock Outbox
struct MockOutbox {
    events: Mutex<Vec<u64>>, // Stores existing seqs
}

impl MockOutbox {
    fn contains(&self, seq: u64) -> bool {
        self.events.lock().unwrap().contains(&seq)
    }
}

// Mock Recovery Logic
struct GapRecoveryController {
    state: Mutex<RecoveryState>,
    outbox: Arc<MockOutbox>,
    max_wait: Duration,
}

impl GapRecoveryController {
    fn new(outbox: Arc<MockOutbox>) -> Self {
        Self {
            state: Mutex::new(RecoveryState::Normal),
            outbox,
            max_wait: Duration::from_millis(50), // Short for test
        }
    }

    async fn on_gap_detected(&self, missing_seq: u64) {
        {
            let mut state = self.state.lock().unwrap();
            if *state == RecoveryState::Normal {
                *state = RecoveryState::GapDetected;
            }
        }

        // Wait buffer
        sleep(self.max_wait).await;

        // Check outbox
        if self.outbox.contains(missing_seq) {
            // Found -> Recovering
            {
                let mut state = self.state.lock().unwrap();
                *state = RecoveryState::Recovering;
            }
            
            // Make transition observable
            tokio::task::yield_now().await;
            sleep(Duration::from_millis(10)).await;

            // Simulate recovery completion
            let mut state = self.state.lock().unwrap();
            *state = RecoveryState::Normal;
        } else {
            // Not Found -> Rebuild
            let mut state = self.state.lock().unwrap();
            *state = RecoveryState::RebuildRequired;
        }
    }
}

#[tokio::test]
async fn test_gap_recovery_found() {
    let outbox = Arc::new(MockOutbox {
        events: Mutex::new(vec![1, 2, 3]), // 2 is present
    });
    let controller = GapRecoveryController::new(outbox);

    // Simulate receiving 1, then 3 (Gap 2)
    controller.on_gap_detected(2).await;

    let state = controller.state.lock().unwrap();
    // Should have recovered to Normal because 2 was in outbox
    assert_eq!(*state, RecoveryState::Normal);
}

#[tokio::test]
async fn test_gap_recovery_rebuild_required() {
    let outbox = Arc::new(MockOutbox {
        events: Mutex::new(vec![1, 3]), // 2 is MISSING
    });
    let controller = GapRecoveryController::new(outbox);

    // Simulate receiving 1, then 3 (Gap 2)
    controller.on_gap_detected(2).await;

    let state = controller.state.lock().unwrap();
    // Should transition to RebuildRequired because 2 was missing
    assert_eq!(*state, RecoveryState::RebuildRequired);
}
