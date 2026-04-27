use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextAreaState;

#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextArea {
    text: String,
    cursor_byte: usize,
}

#[allow(dead_code)]
impl TextArea {
    pub(crate) fn lines(&self) -> Vec<String> {
        self.text.split('\n').map(ToString::to_string).collect()
    }

    pub(crate) fn cursor(&self) -> (usize, usize) {
        let mut row = 0usize;
        let mut remaining = self.cursor_byte.min(self.text.len());
        for line in self.text.split('\n') {
            if remaining <= line.len() {
                let col = line[..remaining].chars().count();
                return (row, col);
            }
            remaining = remaining.saturating_sub(line.len() + 1);
            row += 1;
        }
        (row, 0)
    }

    pub(crate) fn input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_char(ch)
            }
            KeyCode::Enter => self.insert_newline(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => self.move_vertical(-1),
            KeyCode::Down => self.move_vertical(1),
            KeyCode::Home => self.move_line_home(),
            KeyCode::End => self.move_line_end(),
            _ => {}
        }
    }

    pub(crate) fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor_byte, ch);
        self.cursor_byte += ch.len_utf8();
    }

    pub(crate) fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub(crate) fn insert_str(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        self.text.insert_str(self.cursor_byte, text);
        self.cursor_byte += text.len();
    }

    pub(crate) fn move_cursor_jump(&mut self, row: usize, col: usize) {
        self.cursor_byte = cursor_byte_from_row_col(&self.text, row, col);
    }

    pub(crate) fn desired_height(&self, width: u16) -> u16 {
        layout_metrics(&self.text, self.cursor_byte, width).visual_lines
    }

    pub(crate) fn cursor_pos_with_state(&self, width: u16, _state: TextAreaState) -> (u16, u16) {
        let metrics = layout_metrics(&self.text, self.cursor_byte, width);
        (metrics.cursor_col, metrics.cursor_row)
    }

    fn backspace(&mut self) {
        if self.cursor_byte == 0 {
            return;
        }
        let prev = previous_char_boundary(&self.text, self.cursor_byte);
        self.text.drain(prev..self.cursor_byte);
        self.cursor_byte = prev;
    }

    fn delete(&mut self) {
        if self.cursor_byte >= self.text.len() {
            return;
        }
        let next = next_char_boundary(&self.text, self.cursor_byte);
        self.text.drain(self.cursor_byte..next);
    }

    fn move_left(&mut self) {
        self.cursor_byte = previous_char_boundary(&self.text, self.cursor_byte);
    }

    fn move_right(&mut self) {
        self.cursor_byte = next_char_boundary(&self.text, self.cursor_byte);
    }

    fn move_vertical(&mut self, delta_rows: isize) {
        let (row, col) = self.cursor();
        let target_row = if delta_rows.is_negative() {
            row.saturating_sub(delta_rows.unsigned_abs())
        } else {
            row.saturating_add(delta_rows as usize)
        };
        self.cursor_byte = cursor_byte_from_row_col(&self.text, target_row, col);
    }

    fn move_line_home(&mut self) {
        let (row, _) = self.cursor();
        self.cursor_byte = cursor_byte_from_row_col(&self.text, row, 0);
    }

    fn move_line_end(&mut self) {
        let (row, _) = self.cursor();
        let line = self.text.split('\n').nth(row).unwrap_or("");
        self.cursor_byte = cursor_byte_from_row_col(&self.text, row, line.chars().count());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
struct LayoutMetrics {
    visual_lines: u16,
    cursor_row: u16,
    cursor_col: u16,
}

#[allow(dead_code)]
fn layout_metrics(text: &str, cursor_byte: usize, width: u16) -> LayoutMetrics {
    let width = width.max(1);
    let cursor_byte = clamp_to_char_boundary(text, cursor_byte);
    let mut visual_lines = 1u16;
    let mut cursor_row = 0u16;
    let mut cursor_col = 0u16;
    let mut row = 0u16;
    let mut col = 0u16;

    for (byte_idx, ch) in text.char_indices() {
        if byte_idx == cursor_byte {
            cursor_row = row;
            cursor_col = col;
        }

        if ch == '\n' {
            row = row.saturating_add(1);
            visual_lines = visual_lines.max(row.saturating_add(1));
            col = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1) as u16;
        if col.saturating_add(ch_width) > width {
            row = row.saturating_add(1);
            visual_lines = visual_lines.max(row.saturating_add(1));
            col = 0;
        }
        col = col.saturating_add(ch_width);
    }

    if cursor_byte == text.len() {
        cursor_row = row;
        cursor_col = col;
    }

    visual_lines = visual_lines.max(cursor_row.saturating_add(1));

    LayoutMetrics {
        visual_lines,
        cursor_row,
        cursor_col,
    }
}

#[allow(dead_code)]
pub(crate) fn clamp_to_char_boundary(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    if p < text.len() && !text.is_char_boundary(p) {
        p = text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= p)
            .last()
            .unwrap_or(0);
    }
    p
}

#[allow(dead_code)]
fn previous_char_boundary(text: &str, pos: usize) -> usize {
    let pos = clamp_to_char_boundary(text, pos);
    text[..pos]
        .char_indices()
        .last()
        .map(|(idx, _)| idx)
        .unwrap_or(0)
}

#[allow(dead_code)]
fn next_char_boundary(text: &str, pos: usize) -> usize {
    let pos = clamp_to_char_boundary(text, pos);
    if pos >= text.len() {
        return text.len();
    }
    text[pos..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| pos + idx)
        .unwrap_or(text.len())
}

#[allow(dead_code)]
fn cursor_byte_from_row_col(text: &str, target_row: usize, target_col: usize) -> usize {
    let mut row = 0usize;
    let mut byte = 0usize;
    for line in text.split('\n') {
        if row == target_row {
            let col = target_col.min(line.chars().count());
            return byte
                + line
                    .char_indices()
                    .nth(col)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
        }
        byte += line.len() + 1;
        row += 1;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn cursor_tracks_multiline_row_and_col() {
        let mut area = TextArea::default();
        area.insert_str("hello\nwor");
        assert_eq!(area.cursor(), (1, 3));
    }

    #[test]
    fn jump_clamps_to_available_columns() {
        let mut area = TextArea::default();
        area.insert_str("a\nxyz");
        area.move_cursor_jump(0, 5);
        assert_eq!(area.cursor(), (0, 1));
        area.move_cursor_jump(5, 2);
        assert_eq!(area.cursor(), (1, 3));
    }

    #[test]
    fn input_backspace_removes_previous_char_boundary() {
        let mut area = TextArea::default();
        area.insert_str("a界");
        area.input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(area.lines(), vec!["a".to_string()]);
        assert_eq!(area.cursor(), (0, 1));
    }

    #[test]
    fn input_arrows_move_across_lines() {
        let mut area = TextArea::default();
        area.insert_str("ab\ncd");
        area.move_cursor_jump(1, 1);
        area.input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(area.cursor(), (0, 1));
        area.input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(area.cursor(), (1, 1));
    }

    #[test]
    fn desired_height_wraps_long_lines() {
        let mut area = TextArea::default();
        area.insert_str("abcdef");
        assert_eq!(area.desired_height(4), 2);
        assert_eq!(area.cursor_pos_with_state(4, TextAreaState), (2, 1));
    }

    #[test]
    fn desired_height_handles_explicit_newline() {
        let mut area = TextArea::default();
        area.insert_str("hello\n");
        assert_eq!(area.desired_height(20), 2);
        assert_eq!(area.cursor_pos_with_state(20, TextAreaState), (0, 1));
    }

    #[test]
    fn clamp_to_char_boundary_snaps_back_to_utf8_boundary() {
        let text = "a界b";
        let inside_wide = 2;
        assert_eq!(clamp_to_char_boundary(text, inside_wide), 1);
    }
}
