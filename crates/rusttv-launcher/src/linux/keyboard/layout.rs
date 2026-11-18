//! The on-screen keyboard's keys for the remote: four rows of ten keys and a
//! bottom row with the layer key, space, delete and done. The D-pad moves
//! over the keys, OK types one. No drawing here (`draw.rs`).

use crate::model::Input;

/// Keys per row of characters.
pub(crate) const COLUMNS: usize = 10;

/// Rows of characters; the bottom row comes after them.
pub(crate) const CHAR_ROWS: usize = 4;

/// The bottom row: layer switch, space, delete, done.
pub(crate) const BOTTOM_KEYS: usize = 4;

/// Letters (shift makes them capitals).
const LETTERS: [&str; CHAR_ROWS] = ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm._@"];

/// Symbols, for the layer key.
const SYMBOLS: [&str; CHAR_ROWS] = ["!\"#$%&'()*", "+,/:;<=>?[", "\\]^`{|}~€£", "§°´¨µ¿¡«»¤"];

/// A key of the bottom row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomKey {
    /// Letters ⇄ symbols; a second press on letters toggles capitals.
    Layer,
    Space,
    Delete,
    Done,
}

const BOTTOM: [BottomKey; BOTTOM_KEYS] = [
    BottomKey::Layer,
    BottomKey::Space,
    BottomKey::Delete,
    BottomKey::Done,
];

/// What the prompt's owner must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PromptAction {
    /// The text was confirmed.
    Done(String),
    /// Back: the prompt closes without a text.
    Cancel,
}

/// An open prompt: its title, the text so far and the focused key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextPrompt {
    title: String,
    secret: bool,
    text: String,
    row: usize,
    column: usize,
    symbols: bool,
    capitals: bool,
}

impl TextPrompt {
    /// An empty prompt with focus on the first letter.
    pub fn new(title: impl Into<String>, secret: bool) -> Self {
        Self {
            title: title.into(),
            secret,
            text: String::new(),
            row: 1,
            column: 0,
            symbols: false,
            capitals: false,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The text as shown: dots for a secret.
    pub fn shown(&self) -> String {
        if self.secret {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }

    /// The text typed so far (not masked).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The bottom row's key under focus, if focus is there.
    pub fn bottom_key(&self) -> Option<BottomKey> {
        (self.row == CHAR_ROWS).then(|| BOTTOM[self.column])
    }

    /// The focused key: row (`CHAR_ROWS` is the bottom row) and column.
    pub fn focus(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    /// Whether the symbols are shown (the layer key then says "abc").
    pub fn symbols(&self) -> bool {
        self.symbols
    }

    /// Whether letters type as capitals.
    pub fn capitals(&self) -> bool {
        self.capitals
    }

    /// The labels of the character keys, row by row.
    pub fn labels(&self) -> Vec<String> {
        let rows = if self.symbols { SYMBOLS } else { LETTERS };
        rows.iter()
            .flat_map(|row| row.chars())
            .map(|key| self.cased(key).to_string())
            .collect()
    }

    fn cased(&self, key: char) -> char {
        if self.capitals && !self.symbols {
            key.to_ascii_uppercase()
        } else {
            key
        }
    }

    /// Applies a remote key.
    pub fn handle(&mut self, input: Input) -> Option<PromptAction> {
        match input {
            Input::Up => self.move_to(self.row.saturating_sub(1), self.column),
            Input::Down => self.move_to((self.row + 1).min(CHAR_ROWS), self.column),
            Input::Left => {
                let last = self.row_len() - 1;
                self.column = if self.column == 0 {
                    last
                } else {
                    self.column - 1
                };
            }
            Input::Right => self.column = (self.column + 1) % self.row_len(),
            Input::OkReleased => return self.press(self.row, self.column),
            Input::Back => return Some(PromptAction::Cancel),
            _ => {}
        }
        None
    }

    /// Focuses a key (the pointer over it).
    pub fn focus_on(&mut self, row: usize, column: usize) {
        if row > CHAR_ROWS {
            return;
        }
        self.row = row;
        self.column = column.min(self.row_len() - 1);
    }

    /// Deletes the last character.
    pub fn delete(&mut self) {
        self.text.pop();
    }

    fn submit(&self) -> PromptAction {
        PromptAction::Done(self.text.clone())
    }

    fn row_len(&self) -> usize {
        if self.row == CHAR_ROWS {
            BOTTOM_KEYS
        } else {
            COLUMNS
        }
    }

    /// Moves between rows, keeping the key under the same place: the
    /// bottom row's four keys each sit under a stretch of the ten above.
    fn move_to(&mut self, row: usize, column: usize) {
        let from_bottom = self.row == CHAR_ROWS;
        let to_bottom = row == CHAR_ROWS;
        self.column = match (from_bottom, to_bottom) {
            (false, true) => column * BOTTOM_KEYS / COLUMNS,
            (true, false) => (column * COLUMNS / BOTTOM_KEYS + 1).min(COLUMNS - 1),
            _ => column,
        };
        self.row = row;
    }

    fn press(&mut self, row: usize, column: usize) -> Option<PromptAction> {
        if row == CHAR_ROWS {
            match BOTTOM[column] {
                BottomKey::Layer => {
                    if self.symbols {
                        self.symbols = false;
                    } else if self.capitals {
                        self.capitals = false;
                        self.symbols = true;
                    } else {
                        self.capitals = true;
                    }
                }
                BottomKey::Space => self.text.push(' '),
                BottomKey::Delete => self.delete(),
                BottomKey::Done => return Some(self.submit()),
            }
            return None;
        }
        let rows = if self.symbols { SYMBOLS } else { LETTERS };
        if let Some(key) = rows[row].chars().nth(column) {
            self.text.push(self.cased(key));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(prompt: &mut TextPrompt, inputs: &[Input]) -> Option<PromptAction> {
        let mut last = None;
        for input in inputs {
            last = prompt.handle(*input);
        }
        last
    }

    #[test]
    fn the_d_pad_types_letters_and_done_confirms() {
        let mut prompt = TextPrompt::new("Password", true);
        assert_eq!(prompt.focus(), (1, 0));
        // q, then w.
        press(
            &mut prompt,
            &[Input::OkReleased, Input::Right, Input::OkReleased],
        );
        assert_eq!(prompt.shown(), "••");
        // Down to the bottom row: Done is the last key; Left wraps there.
        press(&mut prompt, &[Input::Down, Input::Down, Input::Down]);
        assert_eq!(prompt.focus(), (CHAR_ROWS, 0));
        assert_eq!(
            press(&mut prompt, &[Input::Left, Input::OkReleased]),
            Some(PromptAction::Done("qw".into()))
        );
    }

    #[test]
    fn the_layer_key_gives_capitals_then_symbols_then_letters() {
        let mut prompt = TextPrompt::new("Name", false);
        let mut click = |row, column| {
            prompt.focus_on(row, column);
            prompt.handle(Input::OkReleased)
        };
        click(CHAR_ROWS, 0);
        click(1, 0);
        click(CHAR_ROWS, 0);
        click(0, 0);
        click(CHAR_ROWS, 0);
        click(CHAR_ROWS, 1);
        click(CHAR_ROWS, 2);
        assert_eq!(click(CHAR_ROWS, 3), Some(PromptAction::Done("Q!".into())));
        assert!(!prompt.symbols() && !prompt.capitals());
    }

    #[test]
    fn the_layer_key_shows_capitals_and_symbols() {
        let mut prompt = TextPrompt::new("Name", false);
        prompt.focus_on(CHAR_ROWS, 0);
        prompt.handle(Input::OkReleased);
        assert!(prompt.capitals());
        assert_eq!(prompt.labels()[10], "Q");
        prompt.handle(Input::OkReleased);
        assert!(prompt.symbols());
        assert_eq!(prompt.labels()[0], "!");
    }

    #[test]
    fn delete_and_back_cancels() {
        let mut prompt = TextPrompt::new("Password", true);
        press(&mut prompt, &[Input::OkReleased, Input::OkReleased]);
        prompt.delete();
        assert_eq!(prompt.text(), "q");
        assert_eq!(prompt.title(), "Password");
        assert_eq!(
            press(&mut prompt, &[Input::Back]),
            Some(PromptAction::Cancel)
        );
    }

    #[test]
    fn rows_keep_the_column_under_the_same_place() {
        let mut prompt = TextPrompt::new("x", false);
        press(&mut prompt, &[Input::Right; 9]);
        press(&mut prompt, &[Input::Down; 4]);
        assert_eq!(prompt.focus(), (CHAR_ROWS, 3));
        press(&mut prompt, &[Input::Up]);
        assert_eq!(prompt.focus(), (3, 8));
        assert_eq!(prompt.labels().len(), CHAR_ROWS * COLUMNS);
    }
}
