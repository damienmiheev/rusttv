//! The review and confirmation screen (HISTORY.md § 6.2, § 8.5): read the
//! changes, choose Approve, press the 4-arrow code shown on screen, then hold
//! OK for 2 s. No UI or platform code.

use std::time::Duration;

use crate::model::Input;
use crate::settings::Draft;

/// How long OK must be held to apply.
pub(crate) const HOLD: Duration = Duration::from_secs(2);

/// OK presses earlier than this after the screen opened don't count, so an
/// OK still held from the page before can't confirm.
pub(crate) const ARM_AFTER: Duration = Duration::from_secs(1);

/// One arrow of the code. Physical keys: never mirrored for right-to-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

impl Arrow {
    /// The arrow for a random number (only the low two bits count).
    pub fn from_random(value: u32) -> Self {
        match value & 3 {
            0 => Self::Up,
            1 => Self::Down,
            2 => Self::Left,
            _ => Self::Right,
        }
    }

    fn of(input: Input) -> Option<Self> {
        match input {
            Input::Up => Some(Self::Up),
            Input::Down => Some(Self::Down),
            Input::Left => Some(Self::Left),
            Input::Right => Some(Self::Right),
            _ => None,
        }
    }
}

/// Where the screen is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Reading: `row` is the focused line; the last row is Approve.
    List { row: usize },
    /// `entered` arrows of the code pressed right so far.
    Code { entered: usize },
    /// The code is right; OK held since `since`, if it is.
    Hold { since: Option<Duration> },
}

/// How a key ended the review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Back: nothing changes.
    Cancelled,
    /// A wrong arrow: nothing changes, and it counts toward the lockout.
    WrongCode,
    /// Held long enough: apply.
    Confirmed,
}

/// The review screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Review {
    draft: Draft,
    lines: usize,
    code: [Arrow; 4],
    phase: Phase,
    opened: Duration,
}

impl Review {
    /// Opens at time `now` for `draft`, shown as `lines` lines, with `code`
    /// (from the OS random source).
    pub fn new(draft: Draft, lines: usize, code: [Arrow; 4], now: Duration) -> Self {
        Self {
            draft,
            lines,
            code,
            phase: Phase::List { row: 0 },
            opened: now,
        }
    }

    /// What is being approved.
    pub fn draft(&self) -> &Draft {
        &self.draft
    }

    /// Where the screen is.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Applies one key at `now`.
    pub fn handle(&mut self, input: Input, now: Duration) -> Option<Outcome> {
        if input == Input::Back {
            return Some(Outcome::Cancelled);
        }
        match self.phase {
            Phase::List { row } => match input {
                Input::Up => {
                    self.phase = Phase::List {
                        row: row.saturating_sub(1),
                    }
                }
                Input::Down => {
                    self.phase = Phase::List {
                        row: (row + 1).min(self.lines),
                    }
                }
                Input::OkReleased if row == self.lines => self.phase = Phase::Code { entered: 0 },
                _ => {}
            },
            Phase::Code { entered } => {
                if let Some(arrow) = Arrow::of(input) {
                    if arrow != self.code[entered] {
                        return Some(Outcome::WrongCode);
                    }
                    self.phase = if entered + 1 == self.code.len() {
                        Phase::Hold { since: None }
                    } else {
                        Phase::Code {
                            entered: entered + 1,
                        }
                    };
                }
            }
            Phase::Hold { since } => match input {
                Input::OkPressed if since.is_none() && now >= self.opened + ARM_AFTER => {
                    self.phase = Phase::Hold { since: Some(now) };
                }
                Input::OkReleased => self.phase = Phase::Hold { since: None },
                Input::Tick if since.is_some_and(|since| now.saturating_sub(since) >= HOLD) => {
                    return Some(Outcome::Confirmed);
                }
                _ => {}
            },
        }
        None
    }

    /// When the hold completes, so ui.rs can send a [`Input::Tick`] then.
    pub fn hold_ends(&self) -> Option<Duration> {
        match self.phase {
            Phase::Hold { since: Some(since) } => Some(since + HOLD),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: [Arrow; 4] = [Arrow::Left, Arrow::Up, Arrow::Right, Arrow::Down];

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn to_code(review: &mut Review) {
        review.handle(Input::Down, at(0));
        review.handle(Input::Down, at(0));
        assert_eq!(review.phase(), Phase::List { row: 2 });
        review.handle(Input::OkReleased, at(0));
        assert_eq!(review.phase(), Phase::Code { entered: 0 });
    }

    #[test]
    fn approve_then_code_then_hold_confirms() {
        let mut review = Review::new(Draft::default(), 2, CODE, at(0));
        to_code(&mut review);
        for input in [Input::Left, Input::Up, Input::Right, Input::Down] {
            assert_eq!(review.handle(input, at(100)), None);
        }
        assert_eq!(review.phase(), Phase::Hold { since: None });
        review.handle(Input::OkPressed, at(1_500));
        assert_eq!(review.handle(Input::Tick, at(3_000)), None);
        assert_eq!(review.hold_ends(), Some(at(3_500)));
        assert_eq!(
            review.handle(Input::Tick, at(3_500)),
            Some(Outcome::Confirmed)
        );
    }

    #[test]
    fn arrows_scroll_until_approve_then_count_as_the_code() {
        let mut review = Review::new(Draft::default(), 5, CODE, at(0));
        for _ in 0..3 {
            review.handle(Input::Down, at(0));
        }
        review.handle(Input::Up, at(0));
        assert_eq!(review.phase(), Phase::List { row: 2 });
        // OK on a line (not Approve) does nothing.
        review.handle(Input::OkReleased, at(0));
        assert_eq!(review.phase(), Phase::List { row: 2 });
    }

    #[test]
    fn a_wrong_arrow_ends_the_review() {
        let mut review = Review::new(Draft::default(), 2, CODE, at(0));
        to_code(&mut review);
        review.handle(Input::Left, at(0));
        assert_eq!(review.handle(Input::Left, at(0)), Some(Outcome::WrongCode));
    }

    #[test]
    fn an_early_or_released_hold_does_not_confirm() {
        let mut review = Review::new(Draft::default(), 2, CODE, at(0));
        to_code(&mut review);
        for input in [Input::Left, Input::Up, Input::Right, Input::Down] {
            review.handle(input, at(0));
        }
        // Before the screen is armed.
        review.handle(Input::OkPressed, at(500));
        assert_eq!(review.handle(Input::Tick, at(5_000)), None);
        // Released early, then held again in full.
        review.handle(Input::OkPressed, at(5_000));
        review.handle(Input::OkReleased, at(6_000));
        assert_eq!(review.handle(Input::Tick, at(7_500)), None);
        review.handle(Input::OkPressed, at(8_000));
        assert_eq!(
            review.handle(Input::Tick, at(10_000)),
            Some(Outcome::Confirmed)
        );
    }

    #[test]
    fn back_cancels_at_any_time() {
        let mut review = Review::new(Draft::default(), 2, CODE, at(0));
        to_code(&mut review);
        assert_eq!(review.handle(Input::Back, at(0)), Some(Outcome::Cancelled));
    }
}
