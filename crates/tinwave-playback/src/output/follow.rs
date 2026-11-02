//! Device following: when does a change in the system's audio devices mean "this stream must be
//! re-opened"? Pure logic fed with already-decoded notifications, so it is tested with fake
//! events and shared by every backend that learns about devices.
//!
//! The follower never emits signals (they live in `tinwave-context`, ADR 0004) and never acts: it
//! only answers yes or no, and the stream reports `is_lost()` so the host can `switch_device`.

/// What the stream was opened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowTarget {
    /// The system default render device. Follow it when the default moves.
    Default,
    /// A device the user chose by id. Ignore default changes, follow only its own fate.
    Device(String),
}

/// A device went into a state that cannot play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStateChange {
    /// Usable.
    Active,
    /// Disabled by the user.
    Disabled,
    /// The adapter is gone.
    NotPresent,
    /// The jack or cable was pulled.
    Unplugged,
}

/// A decoded notification (Windows `IMMNotificationClient`; other platforms map theirs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The default device for a flow and role changed. `new_id` is `None` when there is no
    /// default device any more.
    DefaultChanged {
        /// Playback (true) or capture (false).
        render: bool,
        /// The role that people hear by default (console or multimedia), not "communications".
        general_role: bool,
        /// The new default.
        new_id: Option<String>,
    },
    /// A device changed state.
    StateChanged {
        /// Endpoint id.
        id: String,
        /// New state.
        state: DeviceStateChange,
    },
    /// A device was removed from the system.
    Removed {
        /// Endpoint id.
        id: String,
    },
    /// A device appeared. Never a reason to leave a working stream.
    Added {
        /// Endpoint id.
        id: String,
    },
}

/// Decides whether the stream on `current_id` has to be re-opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFollower {
    target: FollowTarget,
    current_id: String,
}

impl DeviceFollower {
    /// A follower for a stream open on `current_id`, which was chosen as `target`.
    #[must_use]
    pub fn new(target: FollowTarget, current_id: impl Into<String>) -> Self {
        Self {
            target,
            current_id: current_id.into(),
        }
    }

    /// `true` when `event` means the stream must be re-opened.
    #[must_use]
    pub fn needs_reopen(&self, event: &DeviceEvent) -> bool {
        match event {
            DeviceEvent::DefaultChanged {
                render,
                general_role,
                new_id,
            } => {
                *render
                    && *general_role
                    && self.target == FollowTarget::Default
                    && new_id.as_deref() != Some(self.current_id.as_str())
            }
            DeviceEvent::StateChanged { id, state } => {
                *id == self.current_id && *state != DeviceStateChange::Active
            }
            DeviceEvent::Removed { id } => *id == self.current_id,
            DeviceEvent::Added { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_changed(new_id: Option<&str>) -> DeviceEvent {
        DeviceEvent::DefaultChanged {
            render: true,
            general_role: true,
            new_id: new_id.map(str::to_owned),
        }
    }

    #[test]
    fn following_the_default_moves_with_it() {
        let f = DeviceFollower::new(FollowTarget::Default, "speakers");
        assert!(f.needs_reopen(&default_changed(Some("headphones"))));
        assert!(f.needs_reopen(&default_changed(None)), "no default left");
        // The notification that repeats the device we are already on is not a change.
        assert!(!f.needs_reopen(&default_changed(Some("speakers"))));
    }

    #[test]
    fn only_the_general_playback_default_counts() {
        let f = DeviceFollower::new(FollowTarget::Default, "speakers");
        let capture = DeviceEvent::DefaultChanged {
            render: false,
            general_role: true,
            new_id: Some("mic".into()),
        };
        let communications = DeviceEvent::DefaultChanged {
            render: true,
            general_role: false,
            new_id: Some("headset".into()),
        };
        assert!(!f.needs_reopen(&capture));
        assert!(!f.needs_reopen(&communications));
    }

    #[test]
    fn a_chosen_device_ignores_default_changes() {
        let f = DeviceFollower::new(FollowTarget::Device("dac".into()), "dac");
        assert!(!f.needs_reopen(&default_changed(Some("headphones"))));
    }

    #[test]
    fn losing_the_current_device_always_counts() {
        for target in [FollowTarget::Default, FollowTarget::Device("dac".into())] {
            let f = DeviceFollower::new(target, "dac");
            for state in [
                DeviceStateChange::Disabled,
                DeviceStateChange::NotPresent,
                DeviceStateChange::Unplugged,
            ] {
                assert!(
                    f.needs_reopen(&DeviceEvent::StateChanged {
                        id: "dac".into(),
                        state
                    }),
                    "{state:?}"
                );
            }
            assert!(f.needs_reopen(&DeviceEvent::Removed { id: "dac".into() }));
        }
    }

    #[test]
    fn other_devices_and_new_devices_do_not_matter() {
        let f = DeviceFollower::new(FollowTarget::Default, "speakers");
        assert!(!f.needs_reopen(&DeviceEvent::StateChanged {
            id: "usb".into(),
            state: DeviceStateChange::Unplugged
        }));
        assert!(!f.needs_reopen(&DeviceEvent::Removed { id: "usb".into() }));
        assert!(!f.needs_reopen(&DeviceEvent::Added { id: "usb".into() }));
        assert!(!f.needs_reopen(&DeviceEvent::StateChanged {
            id: "speakers".into(),
            state: DeviceStateChange::Active
        }));
    }
}
