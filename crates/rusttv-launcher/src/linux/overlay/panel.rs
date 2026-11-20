//! What the app overlay holds and how the keys move through it: the app's
//! size, the volume and closing the app, one row each, in a column.

use crate::device::{Volume, VolumeChange};
use crate::model::SizeStep;

/// A row of the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// The app's size: Left and Right step it, OK goes to the next size.
    Size,
    /// The volume: Left and Right step it, OK mutes or unmutes.
    Volume,
    /// OK closes the app.
    Close,
}

/// What an input did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing changed.
    None,
    /// Something shown changed: draw again.
    Redraw,
    /// Change the volume (then draw again with the new one).
    Volume(VolumeChange),
    /// Close the overlay and the app.
    CloseApp,
    /// Close the overlay.
    Dismiss,
}

/// The overlay's state.
#[derive(Debug, Clone)]
pub struct Panel {
    /// The app's size in percent; `None` where apps can't be resized.
    pub size: Option<u32>,
    /// The size when the overlay opened.
    start_size: Option<u32>,
    /// The volume; `None` without volume control.
    pub volume: Option<Volume>,
    focus: usize,
}

impl Panel {
    /// The first row has the focus.
    pub fn new(size: Option<u32>, volume: Option<Volume>) -> Self {
        Self {
            size,
            start_size: size,
            volume,
            focus: 0,
        }
    }

    /// The rows there are, top to bottom.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.size.is_some() {
            rows.push(Row::Size);
        }
        if self.volume.is_some() {
            rows.push(Row::Volume);
        }
        rows.push(Row::Close);
        rows
    }

    /// The focused row's index in [`Panel::rows`].
    pub fn focus(&self) -> usize {
        self.focus.min(self.rows().len().saturating_sub(1))
    }

    /// Focuses row `index`; `true` if that changed the focus.
    pub fn focus_on(&mut self, index: usize) -> bool {
        let index = index.min(self.rows().len().saturating_sub(1));
        let changed = index != self.focus();
        self.focus = index;
        changed
    }

    /// Whether the size differs from when the overlay opened (the app
    /// restarts for it).
    pub fn resized(&self) -> bool {
        self.size != self.start_size
    }

    /// The focus moves up (`-1`) or down (`1`).
    pub fn move_focus(&mut self, by: isize) -> Effect {
        let target = self.focus().saturating_add_signed(by);
        if self.focus_on(target) {
            Effect::Redraw
        } else {
            Effect::None
        }
    }

    /// Row `index` one step bigger or louder (`up`), or smaller or quieter.
    pub fn step(&mut self, index: usize, up: bool) -> Effect {
        match self.rows().get(index) {
            Some(Row::Size) => self.resize(if up {
                SizeStep::Bigger
            } else {
                SizeStep::Smaller
            }),
            Some(Row::Volume) => Effect::Volume(if up {
                VolumeChange::Up
            } else {
                VolumeChange::Down
            }),
            Some(Row::Close) | None => Effect::None,
        }
    }

    /// OK on row `index`.
    pub fn activate(&mut self, index: usize) -> Effect {
        match self.rows().get(index) {
            Some(Row::Size) => self.resize(SizeStep::Cycle),
            Some(Row::Volume) => Effect::Volume(VolumeChange::Mute),
            Some(Row::Close) => Effect::CloseApp,
            None => Effect::None,
        }
    }

    fn resize(&mut self, step: SizeStep) -> Effect {
        let Some(size) = self.size else {
            return Effect::None;
        };
        let new = step.apply(size);
        if new == size {
            return Effect::None;
        }
        self.size = Some(new);
        Effect::Redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(level: f32) -> Option<Volume> {
        Some(Volume {
            level,
            muted: false,
        })
    }

    #[test]
    fn rows_follow_what_the_app_and_device_offer() {
        assert_eq!(
            Panel::new(Some(100), volume(0.5)).rows(),
            [Row::Size, Row::Volume, Row::Close]
        );
        assert_eq!(Panel::new(None, None).rows(), [Row::Close]);
    }

    #[test]
    fn the_focus_moves_and_stays_inside() {
        let mut panel = Panel::new(Some(100), volume(0.5));
        assert_eq!(panel.move_focus(-1), Effect::None);
        assert_eq!(panel.move_focus(1), Effect::Redraw);
        assert_eq!(panel.move_focus(1), Effect::Redraw);
        assert_eq!(panel.move_focus(1), Effect::None);
        assert_eq!(panel.focus(), 2);
        assert!(!panel.focus_on(2));
        // Past the last row is the last row.
        assert!(panel.focus_on(0));
        assert!(panel.focus_on(9));
        assert_eq!(panel.focus(), 2);
    }

    #[test]
    fn size_steps_cycle_and_count_as_resized() {
        let mut panel = Panel::new(Some(100), volume(0.5));
        assert_eq!(panel.step(0, true), Effect::Redraw);
        assert_eq!(panel.size, Some(110));
        assert!(panel.resized());
        assert_eq!(panel.step(0, false), Effect::Redraw);
        assert!(!panel.resized());
        panel.size = Some(200);
        assert_eq!(panel.step(0, true), Effect::None);
        assert_eq!(panel.activate(0), Effect::Redraw);
        assert_eq!(panel.size, Some(75));
    }

    #[test]
    fn volume_and_close_rows_ask_for_their_change() {
        let mut panel = Panel::new(None, volume(0.5));
        assert_eq!(panel.step(0, true), Effect::Volume(VolumeChange::Up));
        assert_eq!(panel.step(0, false), Effect::Volume(VolumeChange::Down));
        assert_eq!(panel.activate(0), Effect::Volume(VolumeChange::Mute));
        assert_eq!(panel.step(1, true), Effect::None);
        assert_eq!(panel.activate(1), Effect::CloseApp);
    }
}
