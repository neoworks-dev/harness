use super::*;
use crate::onboarding::{Onboarding, Outcome, steps};

impl App {
    /// The agents the daemon can run, for the walkthrough's steps about them.
    pub fn set_harnesses(&mut self, harnesses: Vec<neo_protocol::HarnessInfo>) {
        self.harnesses = harnesses;
    }

    /// The daemon's settings came: with nothing chosen yet, the first start walks the user
    /// through the settings that matter, once.
    pub(super) fn maybe_ask_work(&mut self, chosen: bool) {
        if chosen || self.setup_asked || self.overlay != Overlay::None {
            return;
        }
        self.setup_asked = true;
        self.open_setup(true);
    }

    /// The walkthrough, on the first start or `/setup`.
    pub(super) fn open_setup(&mut self, first: bool) {
        self.onboarding = Some(Onboarding::new(steps(&self.harnesses)));
        self.setup_first = first;
        self.overlay = Overlay::Setup;
    }

    /// Keys in the walkthrough. Done, the picks go to the daemon as commands; left on the first
    /// start, it keeps coding so it doesn't ask again.
    pub(super) fn setup_key(&mut self, key: KeyEvent, fx: &mut Vec<Effect>) {
        let Some(onboarding) = self.onboarding.as_mut() else {
            self.overlay = Overlay::None;
            return;
        };
        let commands = match onboarding.key(key) {
            Outcome::Stay => return,
            Outcome::Done(commands) => commands,
            Outcome::Skipped if self.setup_first => vec!["/settings work coding".to_string()],
            Outcome::Skipped => Vec::new(),
        };
        self.onboarding = None;
        self.overlay = Overlay::None;
        for text in commands {
            fx.push(Effect::Input {
                session: self.session.clone(),
                text,
                images: Vec::new(),
                steer: false,
            });
        }
    }

    /// The walkthrough over the whole screen.
    pub(super) fn draw_setup(&self, frame: &mut Frame) {
        if let Some(onboarding) = &self.onboarding {
            onboarding.draw(frame, frame.area(), &self.styles);
        }
    }
}
