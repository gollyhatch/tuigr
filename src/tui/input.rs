#[derive(Clone, Debug, Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.chars().count();
    }

    pub fn insert(&mut self, ch: char) {
        let byte = self.byte_index(self.cursor);
        self.value.insert(byte, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte = self.byte_index(self.cursor - 1);
        self.value.remove(byte);
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.value.chars().count() {
            return;
        }
        let byte = self.byte_index(self.cursor);
        self.value.remove(byte);
    }

    /// Delete from the cursor back to the start of the previous
    /// whitespace-delimited word, including any whitespace just before the
    /// cursor.
    pub fn delete_word(&mut self) {
        let chars: Vec<char> = self.value.chars().collect();
        let mut start = self.cursor.min(chars.len());
        if start == 0 {
            return;
        }
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let from = self.byte_index(start);
        let to = self.byte_index(self.cursor.min(chars.len()));
        self.value.replace_range(from..to, "");
        self.cursor = start;
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    fn byte_index(&self, char_index: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_index)
            .map(|(i, _)| i)
            .unwrap_or(self.value.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_input(value: &str) -> TextInput {
        let mut input = TextInput::default();
        input.set(value);
        input
    }

    #[test]
    fn deletes_previous_word() {
        let mut input = text_input("hello world");
        input.delete_word();
        assert_eq!(input.value, "hello ");
        assert_eq!(input.cursor, 6);

        input.delete_word();
        assert_eq!(input.value, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn deletes_trailing_whitespace_with_word() {
        let mut input = text_input("hello world   ");
        input.delete_word();
        assert_eq!(input.value, "hello ");
        assert_eq!(input.cursor, 6);
    }

    #[test]
    fn deletes_word_before_cursor_and_keeps_suffix() {
        let mut input = text_input("one two three");
        input.cursor = 7;
        input.delete_word();
        assert_eq!(input.value, "one  three");
        assert_eq!(input.cursor, 4);
    }

    #[test]
    fn handles_unicode_and_only_whitespace() {
        let mut input = text_input("über alles");
        input.delete_word();
        assert_eq!(input.value, "über ");

        let mut input = text_input("    ");
        input.delete_word();
        assert_eq!(input.value, "");
        assert_eq!(input.cursor, 0);
    }
}
