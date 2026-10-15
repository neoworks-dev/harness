//! Where the agent is, from this app's point of view. The daemon sends `agent.focus` after
//! every hello and on each change; this turns that stream into "open the pane on session X" /
//! "close the pane on session Y". No GPUI in here.

use suite_protocol::AgentFocus;

/// What the pane has to do after a focus message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Show the chat pane on this session (a fresh subscription from seq 1).
    Open(String),
    /// Collapse the pane and stop following this session.
    Close(String),
}

#[derive(Debug)]
pub struct Follower {
    app: String,
    /// The session the pane is open on.
    open: Option<String>,
}

impl Follower {
    pub fn new(app: &str) -> Follower {
        Follower {
            app: app.into(),
            open: None,
        }
    }

    /// The session the pane is open on, if it is open.
    pub fn pane(&self) -> Option<&str> {
        self.open.as_deref()
    }

    /// Applies one `agent.focus`. A repeat of what the pane already shows (the daemon re-sends
    /// the focus after a reconnect) changes nothing; the SDK has resumed the subscription.
    pub fn on_focus(&mut self, focus: &AgentFocus) -> Vec<Change> {
        let target = (focus.app.as_deref() == Some(self.app.as_str())).then_some(&focus.session);
        if target == self.open.as_ref() {
            return Vec::new();
        }
        let mut changes = Vec::new();
        if let Some(old) = self.open.take() {
            changes.push(Change::Close(old));
        }
        if let Some(session) = target {
            self.open = Some(session.clone());
            changes.push(Change::Open(session.clone()));
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suite_protocol::FocusReason;

    fn focus(session: &str, app: Option<&str>) -> AgentFocus {
        AgentFocus {
            session: session.into(),
            app: app.map(Into::into),
            from: None,
            reason: FocusReason::User,
        }
    }

    fn open(session: &str) -> Change {
        Change::Open(session.into())
    }

    fn close(session: &str) -> Change {
        Change::Close(session.into())
    }

    #[test]
    fn opens_when_the_agent_arrives_and_collapses_when_it_leaves() {
        let mut follower = Follower::new("pad");
        assert!(follower.on_focus(&focus("s1", Some("overlay"))).is_empty());
        assert_eq!(follower.pane(), None);

        assert_eq!(follower.on_focus(&focus("s1", Some("pad"))), [open("s1")]);
        assert_eq!(follower.pane(), Some("s1"));

        assert_eq!(
            follower.on_focus(&focus("s1", Some("overlay"))),
            [close("s1")]
        );
        assert_eq!(follower.pane(), None);
    }

    #[test]
    fn a_repeated_focus_changes_nothing() {
        let mut follower = Follower::new("pad");
        follower.on_focus(&focus("s1", Some("pad")));
        assert!(follower.on_focus(&focus("s1", Some("pad"))).is_empty());
        assert_eq!(follower.pane(), Some("s1"));
    }

    #[test]
    fn a_new_session_resubscribes() {
        let mut follower = Follower::new("pad");
        follower.on_focus(&focus("s1", Some("pad")));
        assert_eq!(
            follower.on_focus(&focus("s2", Some("pad"))),
            [close("s1"), open("s2")]
        );
        assert_eq!(follower.pane(), Some("s2"));
    }

    #[test]
    fn session_changes_elsewhere_are_ignored() {
        let mut follower = Follower::new("pad");
        assert!(follower.on_focus(&focus("s1", Some("overlay"))).is_empty());
        assert!(follower.on_focus(&focus("s2", Some("overlay"))).is_empty());
        assert_eq!(follower.pane(), None);
    }

    #[test]
    fn no_hosting_app_closes_the_pane() {
        let mut follower = Follower::new("pad");
        follower.on_focus(&focus("s1", Some("pad")));
        assert_eq!(follower.on_focus(&focus("s1", None)), [close("s1")]);
        assert_eq!(follower.pane(), None);
    }
}
