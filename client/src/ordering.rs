//! Frame ordering across pipe connections.
//!
//! Schema 2 frames carry a plugin `session` and a `sequence` that increases strictly within
//! that session, also across reconnects. A frame is accepted when it starts a new session or
//! advances the sequence of the current one; repeated or older frames are dropped.
//!
//! Schema 1 frames have no session and the v1 plugin opened one connection per frame, so a
//! game restart could not be told apart from reordering. For them the check applies only
//! within one connection, as in the first release.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameOrder {
    last: Option<(Option<u64>, u64)>,
}

impl FrameOrder {
    /// Returns `true` if the frame is new and should be published.
    pub fn accept(&mut self, session: Option<u64>, sequence: u64) -> bool {
        let newer = match self.last {
            Some((last_session, last_sequence)) if last_session == session => {
                sequence > last_sequence
            }
            _ => true,
        };
        if newer {
            self.last = Some((session, sequence));
        }
        newer
    }

    /// Call when a pipe connection ends.
    pub fn connection_closed(&mut self) {
        if matches!(self.last, Some((None, _))) {
            self.last = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_increasing_and_drops_repeated_or_older_frames() {
        let mut o = FrameOrder::default();
        assert!(o.accept(Some(1), 1));
        assert!(o.accept(Some(1), 2));
        assert!(!o.accept(Some(1), 2));
        assert!(!o.accept(Some(1), 1));
        assert!(o.accept(Some(1), 10));
    }

    #[test]
    fn keeps_ordering_across_reconnects_of_the_same_session() {
        let mut o = FrameOrder::default();
        assert!(o.accept(Some(5), 100));
        o.connection_closed();
        assert!(
            !o.accept(Some(5), 99),
            "an old frame after reconnect must be dropped"
        );
        assert!(o.accept(Some(5), 101));
    }

    #[test]
    fn a_new_session_starts_over() {
        let mut o = FrameOrder::default();
        assert!(o.accept(Some(5), 100));
        assert!(o.accept(Some(6), 1));
        assert!(!o.accept(Some(6), 1));
    }

    #[test]
    fn v1_frames_are_ordered_per_connection_only() {
        let mut o = FrameOrder::default();
        assert!(o.accept(None, 50));
        assert!(!o.accept(None, 49));
        o.connection_closed();
        assert!(o.accept(None, 1), "v1 game restart resets the sequence");
    }
}
