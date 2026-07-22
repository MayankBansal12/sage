//! Recording duration decision (Hex RecordingDecisionEngine).

use crate::hotkey::HotKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingDecision {
    ProceedToTranscription,
    DiscardShortRecording,
}

pub fn decide(
    hotkey: &HotKey,
    minimum_key_time: f64,
    duration_secs: f64,
) -> RecordingDecision {
    let min = if hotkey.key.is_none() {
        // Modifier-only: enforce at least 0.3s
        minimum_key_time.max(0.3)
    } else {
        minimum_key_time
    };
    if duration_secs + f64::EPSILON < min {
        RecordingDecision::DiscardShortRecording
    } else {
        RecordingDecision::ProceedToTranscription
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discards_short() {
        let hk = HotKey::parse("ctrl+r").unwrap();
        assert_eq!(
            decide(&hk, 0.2, 0.05),
            RecordingDecision::DiscardShortRecording
        );
        assert_eq!(
            decide(&hk, 0.2, 0.5),
            RecordingDecision::ProceedToTranscription
        );
    }

    #[test]
    fn modifier_only_min_0_3() {
        let hk = HotKey::parse("ctrl").unwrap();
        assert_eq!(
            decide(&hk, 0.1, 0.2),
            RecordingDecision::DiscardShortRecording
        );
        assert_eq!(
            decide(&hk, 0.1, 0.35),
            RecordingDecision::ProceedToTranscription
        );
    }
}
