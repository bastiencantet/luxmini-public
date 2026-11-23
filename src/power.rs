//! Shared idle-event policy. This module never accesses hardware or preferences.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    SystemSleep,
    DisplaySleep,
}

impl Trigger {
    pub const fn from_stored(value: i64) -> Self {
        if value == 1 {
            Self::DisplaySleep
        } else {
            Self::SystemSleep
        }
    }

    pub const fn stored(self) -> i64 {
        match self {
            Self::SystemSleep => 0,
            Self::DisplaySleep => 1,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::SystemSleep => "Mac system sleep",
            Self::DisplaySleep => "Display sleep",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SystemSleep,
    SystemWake,
    DisplaysSleep,
    DisplaysWake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Applied(bool),
    Restored(bool),
}

#[derive(Default)]
pub struct State {
    pub system_asleep: bool,
    displays_asleep: bool,
}

impl State {
    pub fn observe(&mut self, event: Event) {
        match event {
            Event::SystemSleep => {
                // Prepare the light before hardware is suspended, even if the
                // display notification arrives later. Display wake releases it.
                if !self.system_asleep {
                    self.displays_asleep = true;
                }
                self.system_asleep = true;
            }
            Event::SystemWake => self.system_asleep = false,
            Event::DisplaysSleep => self.displays_asleep = true,
            Event::DisplaysWake => self.displays_asleep = false,
        }
    }

    pub const fn matches(&self, trigger: Trigger) -> bool {
        match trigger {
            Trigger::SystemSleep => self.system_asleep,
            Trigger::DisplaySleep => self.system_asleep || self.displays_asleep,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, State, Trigger};

    #[test]
    fn missing_and_unknown_preferences_keep_system_sleep() {
        for value in [0, -1, 2, i64::MAX] {
            assert_eq!(Trigger::from_stored(value), Trigger::SystemSleep);
        }
        assert_eq!(Trigger::from_stored(1), Trigger::DisplaySleep);
        assert_eq!(
            Trigger::from_stored(Trigger::DisplaySleep.stored()),
            Trigger::DisplaySleep
        );
    }

    #[test]
    fn display_sleep_does_not_trigger_system_sleep_mode() {
        let mut state = State::default();
        state.observe(Event::DisplaysSleep);
        assert!(state.matches(Trigger::DisplaySleep));
        assert!(!state.matches(Trigger::SystemSleep));
    }

    #[test]
    fn system_wake_keeps_display_mode_active_until_the_display_wakes() {
        let mut state = State::default();
        state.observe(Event::SystemSleep);
        state.observe(Event::SystemWake);
        assert!(!state.matches(Trigger::SystemSleep));
        assert!(state.matches(Trigger::DisplaySleep));
        state.observe(Event::DisplaysWake);
        assert!(!state.matches(Trigger::DisplaySleep));
    }

    #[test]
    fn an_early_display_wake_cannot_release_a_sleeping_system() {
        let mut state = State::default();
        state.observe(Event::SystemSleep);
        state.observe(Event::DisplaysWake);
        assert!(state.matches(Trigger::DisplaySleep));
        state.observe(Event::SystemWake);
        assert!(!state.matches(Trigger::DisplaySleep));
    }

    #[test]
    fn duplicate_system_sleep_does_not_undo_an_early_display_wake() {
        let mut state = State::default();
        state.observe(Event::SystemSleep);
        state.observe(Event::DisplaysWake);
        state.observe(Event::SystemSleep);
        state.observe(Event::SystemWake);
        assert!(!state.matches(Trigger::DisplaySleep));
    }
}
