use kernel_core::event::{GapRecoveryState, progress_gap_recovery};

#[tokio::test]
async fn test_gap_recovery_found() {
    let state = progress_gap_recovery(GapRecoveryState::GapDetected, true);
    assert_eq!(state, GapRecoveryState::Recovering);
    let state = progress_gap_recovery(state, true);
    assert_eq!(state, GapRecoveryState::Recovering);
    let state = progress_gap_recovery(state, false);
    assert_eq!(state, GapRecoveryState::Normal);
}

#[tokio::test]
async fn test_gap_recovery_rebuild_required() {
    let state = progress_gap_recovery(GapRecoveryState::GapDetected, false);
    assert_eq!(state, GapRecoveryState::RebuildRequired);
}
