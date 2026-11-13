//! Frame ordering across pipe connections.
//!
//! Schema 2 frames carry a plugin `session` and a `sequence` that increases strictly within
//! that session, also across reconnects. A frame is accepted when it starts a new session or
//! advances the sequence of the current one; repeated or older frames are dropped.
//!
//! A session that was replaced by another one is **retired** and never accepted again: a
//! late frame of a finished game or plugin instance (for example still buffered somewhere)
//! must not be taken for a new session. The last [`RETIRED_SESSIONS`] retired sessions are
//! remembered; session ids are random 53-bit values, so an older one coming back is not a
//! realistic case. (Two games running at once and writing alternately would therefore only
//! deliver the first of them; RoadPilot supports one game at a time.)
//!
//! Schema 1 frames have no session and the v1 plugin opened one connection per frame, so a
//! game restart could not be told apart from reordering. For them the check applies only
//! within one connection, as in the first release.

/// How many replaced sessions are remembered and refused.
pub const RETIRED_SESSIONS: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameOrder {
    last: Option<(Option<u64>, u64)>,
    /// Replaced schema 2 sessions, most recent at `next_retired - 1` (ring).
    retired: [Option<u64>; RETIRED_SESSIONS],
    next_retired: usize,
}

impl FrameOrder {
    /// Returns `true` if the frame is new and should be published.
    pub fn accept(&mut self, session: Option<u64>, sequence: u64) -> bool {
        if session.is_some() && self.retired.contains(&session) {
            return false;
        }
        let newer = match self.last {
            Some((last_session, last_sequence)) if last_session == session => {
                sequence > last_sequence
            }
            _ => true,
        };
        if newer {
            if let Some((Some(previous), _)) = self.last
                && session != Some(previous)
            {
                self.retire(previous);
            }
            self.last = Some((session, sequence));
        }
        newer
    }

    /// True if `session` was replaced by another session and is refused.
    pub fn is_retired(&self, session: u64) -> bool {
        self.retired.contains(&Some(session))
    }

    fn retire(&mut self, session: u64) {
        self.retired[self.next_retired] = Some(session);
        self.next_retired = (self.next_retired + 1) % RETIRED_SESSIONS;
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
    fn a_replaced_session_is_never_accepted_again() {
        let mut o = FrameOrder::default();
        assert!(o.accept(Some(5), 100));
        assert!(o.accept(Some(6), 1));
        assert!(o.is_retired(5) && !o.is_retired(6));
        // A late frame of session 5, even with a higher sequence, is refused...
        assert!(!o.accept(Some(5), 101));
        // ...also after a reconnect, and also once a third session followed.
        o.connection_closed();
        assert!(!o.accept(Some(5), 102));
        assert!(o.accept(Some(7), 1));
        assert!(!o.accept(Some(5), 103));
        assert!(!o.accept(Some(6), 2));
        // The current session continues normally.
        assert!(o.accept(Some(7), 2));
    }

    #[test]
    fn the_retired_list_is_bounded() {
        let mut o = FrameOrder::default();
        for session in 1..=(RETIRED_SESSIONS as u64 + 2) {
            assert!(o.accept(Some(session), 1));
        }
        let current = RETIRED_SESSIONS as u64 + 2;
        // The newest RETIRED_SESSIONS replaced sessions are refused; the oldest is forgotten.
        for session in 2..current {
            assert!(o.is_retired(session), "{session}");
        }
        assert!(!o.is_retired(1));
        assert!(!o.is_retired(current));
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
