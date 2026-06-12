//! Line editor for the status-line input (visidata's _input.py): editing,
//! history, tab-completion.

#[derive(Clone)]
pub struct LineEditor {
    pub prompt: String,
    pub buffer: String,
    /// Cursor position in chars.
    pub cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    saved_buffer: String,
    completions: Vec<String>,
    completion_pos: Option<usize>,
}

impl LineEditor {
    pub fn new(prompt: impl Into<String>, initial: impl Into<String>) -> Self {
        let buffer: String = initial.into();
        let cursor = buffer.chars().count();
        LineEditor {
            prompt: prompt.into(),
            buffer,
            cursor,
            history: Vec::new(),
            history_pos: None,
            saved_buffer: String::new(),
            completions: Vec::new(),
            completion_pos: None,
        }
    }

    pub fn with_history(mut self, history: Vec<String>) -> Self {
        self.history = history;
        self
    }

    pub fn with_completions(mut self, completions: Vec<String>) -> Self {
        self.completions = completions;
        self
    }

    fn byte_pos(&self, char_pos: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.buffer.len())
    }

    pub fn insert(&mut self, c: char) {
        let p = self.byte_pos(self.cursor);
        self.buffer.insert(p, c);
        self.cursor += 1;
        self.completion_pos = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let p = self.byte_pos(self.cursor - 1);
            self.buffer.remove(p);
            self.cursor -= 1;
        }
        self.completion_pos = None;
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buffer.chars().count() {
            let p = self.byte_pos(self.cursor);
            self.buffer.remove(p);
        }
        self.completion_pos = None;
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.chars().count();
    }

    pub fn clear_to_end(&mut self) {
        let p = self.byte_pos(self.cursor);
        self.buffer.truncate(p);
    }

    pub fn clear_all(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_pos {
            None => {
                self.saved_buffer = self.buffer.clone();
                self.history_pos = Some(self.history.len() - 1);
            }
            Some(0) => {}
            Some(p) => self.history_pos = Some(p - 1),
        }
        if let Some(p) = self.history_pos {
            self.buffer = self.history[p].clone();
            self.end();
        }
    }

    pub fn history_next(&mut self) {
        match self.history_pos {
            None => {}
            Some(p) if p + 1 >= self.history.len() => {
                self.history_pos = None;
                self.buffer = self.saved_buffer.clone();
                self.end();
            }
            Some(p) => {
                self.history_pos = Some(p + 1);
                self.buffer = self.history[p + 1].clone();
                self.end();
            }
        }
    }

    /// Tab: cycle through completions matching the current prefix.
    pub fn complete(&mut self) {
        if self.completions.is_empty() {
            return;
        }
        match self.completion_pos {
            None => {
                let prefix = self.buffer.clone();
                let matches: Vec<usize> = self
                    .completions
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.starts_with(&prefix))
                    .map(|(i, _)| i)
                    .collect();
                if let Some(&first) = matches.first() {
                    self.saved_buffer = prefix;
                    self.completion_pos = Some(first);
                    self.buffer = self.completions[first].clone();
                    self.end();
                }
            }
            Some(p) => {
                // next match after p with the original prefix
                let prefix = self.saved_buffer.clone();
                let next = self
                    .completions
                    .iter()
                    .enumerate()
                    .skip(p + 1)
                    .find(|(_, c)| c.starts_with(&prefix))
                    .map(|(i, _)| i)
                    .or_else(|| {
                        self.completions
                            .iter()
                            .enumerate()
                            .find(|(_, c)| c.starts_with(&prefix))
                            .map(|(i, _)| i)
                    });
                if let Some(i) = next {
                    self.completion_pos = Some(i);
                    self.buffer = self.completions[i].clone();
                    self.end();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing() {
        let mut ed = LineEditor::new("> ", "");
        for c in "hello".chars() {
            ed.insert(c);
        }
        ed.left();
        ed.left();
        ed.insert('X');
        assert_eq!(ed.buffer, "helXlo");
        ed.backspace();
        assert_eq!(ed.buffer, "hello");
        ed.delete();
        assert_eq!(ed.buffer, "helo");
    }

    #[test]
    fn history_navigation() {
        let mut ed =
            LineEditor::new("> ", "").with_history(vec!["one".into(), "two".into()]);
        ed.insert('x');
        ed.history_prev();
        assert_eq!(ed.buffer, "two");
        ed.history_prev();
        assert_eq!(ed.buffer, "one");
        ed.history_next();
        assert_eq!(ed.buffer, "two");
        ed.history_next();
        assert_eq!(ed.buffer, "x"); // restored
    }

    #[test]
    fn completion_cycles() {
        let mut ed = LineEditor::new("> ", "").with_completions(vec![
            "sort-asc".into(),
            "sort-desc".into(),
            "search-col".into(),
        ]);
        ed.insert('s');
        ed.insert('o');
        ed.complete();
        assert_eq!(ed.buffer, "sort-asc");
        ed.complete();
        assert_eq!(ed.buffer, "sort-desc");
        ed.complete();
        assert_eq!(ed.buffer, "sort-asc"); // wraps
    }

    #[test]
    fn unicode_editing() {
        let mut ed = LineEditor::new("> ", "日本");
        ed.insert('語');
        assert_eq!(ed.buffer, "日本語");
        ed.backspace();
        assert_eq!(ed.buffer, "日本");
    }
}
