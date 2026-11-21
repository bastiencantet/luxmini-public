//! Closed onboarding vocabulary and bounded, retry-safe local journey records.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    AppOpened,
    UnsupportedCancelled,
    OnboardingStarted,
    OnboardingCancelled,
    OnboardingCompleted,
    OnboardingFailed,
    ProfileReady,
    ProfileUnavailable,
    HelperReady,
    HelperFailed,
    LedTestFailed,
    VisualConfirmed,
    VisualRejected,
    ControlReady,
    ControlUnavailable,
    ManualControlSuccess,
    ManualControlFailed,
    AppExited,
    SleepOptionEnabled,
    SleepStarted,
    SleepApplied,
    SleepFailed,
    WakeRestored,
    WakeFailed,
}

impl Stage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppOpened => "app_opened",
            Self::UnsupportedCancelled => "unsupported_cancelled",
            Self::OnboardingStarted => "onboarding_started",
            Self::OnboardingCancelled => "onboarding_cancelled",
            Self::OnboardingCompleted => "onboarding_completed",
            Self::OnboardingFailed => "onboarding_failed",
            Self::ProfileReady => "profile_ready",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::HelperReady => "helper_ready",
            Self::HelperFailed => "helper_failed",
            Self::LedTestFailed => "led_test_failed",
            Self::VisualConfirmed => "visual_confirmed",
            Self::VisualRejected => "visual_rejected",
            Self::ControlReady => "control_ready",
            Self::ControlUnavailable => "control_unavailable",
            Self::ManualControlSuccess => "manual_control_success",
            Self::ManualControlFailed => "manual_control_failed",
            Self::AppExited => "app_exited",
            Self::SleepOptionEnabled => "sleep_option_enabled",
            Self::SleepStarted => "sleep_started",
            Self::SleepApplied => "sleep_applied",
            Self::SleepFailed => "sleep_failed",
            Self::WakeRestored => "wake_restored",
            Self::WakeFailed => "wake_failed",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub session_id: String,
    pub device_kind: String,
    pub app_version: String,
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
}

/// Each stage is recorded once per process session. A full outbox preserves
/// older unsent records and explicitly refuses new records instead of growing.
pub fn push(
    pending: &mut Vec<Record>,
    current_session: &mut Option<String>,
    seen: &mut Vec<Stage>,
    record: Record,
) -> bool {
    if current_session.as_deref() != Some(&record.session_id) {
        *current_session = Some(record.session_id.clone());
        seen.clear();
    }
    if pending.len() >= 128 {
        return false;
    }
    if record.cycle_id.is_some() {
        if pending.iter().any(|item| {
            item.session_id == record.session_id
                && item.cycle_id == record.cycle_id
                && item.stage == record.stage
        }) {
            return false;
        }
    } else {
        if seen.contains(&record.stage) {
            return false;
        }
        seen.push(record.stage);
    }
    pending.push(record);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(session: &str, stage: Stage) -> Record {
        Record {
            session_id: session.into(),
            device_kind: "rear_logo".into(),
            app_version: "0.5.2".into(),
            stage,
            cycle_id: None,
        }
    }

    #[test]
    fn retries_do_not_create_additional_openings_but_new_processes_do() {
        let (mut queue, mut session, mut seen) = (Vec::new(), None, Vec::new());
        assert!(push(
            &mut queue,
            &mut session,
            &mut seen,
            record("one", Stage::AppOpened)
        ));
        assert!(!push(
            &mut queue,
            &mut session,
            &mut seen,
            record("one", Stage::AppOpened)
        ));
        queue.clear();
        assert!(!push(
            &mut queue,
            &mut session,
            &mut seen,
            record("one", Stage::AppOpened)
        ));
        assert!(push(
            &mut queue,
            &mut session,
            &mut seen,
            record("two", Stage::AppOpened)
        ));
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn failed_and_successful_steps_are_separate_and_the_outbox_is_bounded() {
        let (mut queue, mut session, mut seen) = (Vec::new(), None, Vec::new());
        assert!(push(
            &mut queue,
            &mut session,
            &mut seen,
            record("one", Stage::HelperFailed)
        ));
        assert!(push(
            &mut queue,
            &mut session,
            &mut seen,
            record("one", Stage::HelperReady)
        ));
        for index in 0..200 {
            push(
                &mut queue,
                &mut session,
                &mut seen,
                record(&format!("session-{index}"), Stage::AppOpened),
            );
        }
        assert_eq!(queue.len(), 128);
        assert_eq!(
            queue.first().map(|record| record.stage),
            Some(Stage::HelperFailed)
        );
    }
}
