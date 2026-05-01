use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextAreaState {
    scroll: u16,
}

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
        wrapped_lines(&self.text, width).len() as u16
    }

    pub(crate) fn cursor_pos_with_state(
        &self,
        area: Rect,
        state: TextAreaState,
    ) -> Option<(u16, u16)> {
        let lines = wrapped_lines(&self.text, area.width);
        if lines.is_empty() {
            return Some((area.x, area.y));
        }
        let effective_scroll =
            effective_scroll(self.cursor_byte, area.height, &lines, state.scroll);
        let cursor_line_idx =
            wrapped_line_index_by_start(&lines, self.cursor_byte).unwrap_or(0) as u16;
        let line = &lines[cursor_line_idx as usize];
        let col = display_width(&self.text[line.start..self.cursor_byte]);
        let screen_row = cursor_line_idx.saturating_sub(effective_scroll);
        Some((area.x + col, area.y + screen_row))
    }

    pub(crate) fn overflow_state(
        &self,
        width: u16,
        area_height: u16,
        state: TextAreaState,
    ) -> OverflowState {
        let lines = wrapped_lines(&self.text, width);
        overflow_state_for_lines(self.cursor_byte, area_height, &lines, state.scroll)
    }

    pub(crate) fn render_with_state(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &mut TextAreaState,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let lines = wrapped_lines(&self.text, area.width);
        if lines.is_empty() {
            return;
        }
        let scroll = effective_scroll(self.cursor_byte, area.height, &lines, state.scroll);
        state.scroll = scroll;
        let start = scroll as usize;
        let end = (scroll + area.height).min(lines.len() as u16) as usize;

        for (screen_row, line_range) in lines[start..end].iter().enumerate() {
            let mut col = 0u16;
            for ch in self.text[line_range.clone()].chars() {
                if col >= area.width {
                    break;
                }
                let width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1) as u16;
                if col + width > area.width {
                    break;
                }
                buf[(area.x + col, area.y + screen_row as u16)].set_char(ch);
                for extra in 1..width {
                    if col + extra < area.width {
                        buf[(area.x + col + extra, area.y + screen_row as u16)].set_char(' ');
                    }
                }
                col += width;
            }
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverflowState {
    pub(crate) hidden_above: bool,
    pub(crate) hidden_below: bool,
}

fn display_width(text: &str) -> u16 {
    let mut col = 0u16;
    for ch in text.chars() {
        col = col.saturating_add(UnicodeWidthChar::width(ch).unwrap_or(0).max(1) as u16);
    }
    col
}

fn wrapped_lines(text: &str, width: u16) -> Vec<std::ops::Range<usize>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line_start = 0usize;
    let mut col = 0u16;

    for (byte_idx, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push(line_start..byte_idx);
            line_start = byte_idx + ch.len_utf8();
            col = 0;
            continue;
        }

        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1) as u16;
        if col > 0 && col.saturating_add(ch_width) > width {
            lines.push(line_start..byte_idx);
            line_start = byte_idx;
            col = 0;
        }
        col = col.saturating_add(ch_width);
    }

    lines.push(line_start..text.len());
    lines
}

fn wrapped_line_index_by_start(lines: &[std::ops::Range<usize>], pos: usize) -> Option<usize> {
    let idx = lines.partition_point(|r| r.start <= pos);
    if idx == 0 {
        None
    } else {
        Some(idx - 1)
    }
}

fn effective_scroll(
    cursor_byte: usize,
    area_height: u16,
    lines: &[std::ops::Range<usize>],
    current_scroll: u16,
) -> u16 {
    if area_height == 0 {
        return current_scroll;
    }
    let total_lines = lines.len() as u16;
    if area_height >= total_lines {
        return 0;
    }

    let cursor_line_idx = wrapped_line_index_by_start(lines, cursor_byte).unwrap_or(0) as u16;
    let max_scroll = total_lines.saturating_sub(area_height);
    let mut scroll = current_scroll.min(max_scroll);

    if cursor_line_idx < scroll {
        scroll = cursor_line_idx;
    } else if cursor_line_idx >= scroll + area_height {
        scroll = cursor_line_idx + 1 - area_height;
    }
    scroll
}

fn overflow_state_for_lines(
    cursor_byte: usize,
    area_height: u16,
    lines: &[std::ops::Range<usize>],
    current_scroll: u16,
) -> OverflowState {
    if area_height == 0 || lines.is_empty() {
        return OverflowState::default();
    }
    let scroll = effective_scroll(cursor_byte, area_height, lines, current_scroll);
    let total_lines = lines.len() as u16;
    OverflowState {
        hidden_above: scroll > 0,
        hidden_below: scroll.saturating_add(area_height) < total_lines,
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
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

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
        let pos = area.cursor_pos_with_state(Rect::new(0, 0, 4, 2), TextAreaState::default());
        assert_eq!(pos, Some((2, 1)));
    }

    #[test]
    fn desired_height_handles_explicit_newline() {
        let mut area = TextArea::default();
        area.insert_str("hello\n");
        assert_eq!(area.desired_height(20), 2);
        let pos = area.cursor_pos_with_state(Rect::new(0, 0, 20, 2), TextAreaState::default());
        assert_eq!(pos, Some((0, 1)));
    }

    #[test]
    fn clamp_to_char_boundary_snaps_back_to_utf8_boundary() {
        let text = "a界b";
        let inside_wide = 2;
        assert_eq!(clamp_to_char_boundary(text, inside_wide), 1);
    }

    #[test]
    fn cursor_pos_with_state_scrolls_to_keep_cursor_visible() {
        let mut area = TextArea::default();
        area.insert_str("line1\nline2\nline3\nline4");
        let pos = area.cursor_pos_with_state(Rect::new(0, 0, 20, 2), TextAreaState::default());
        assert_eq!(pos, Some((5, 1)));
    }

    #[test]
    fn render_with_state_shows_bottom_slice_when_scrolled() {
        let mut area = TextArea::default();
        area.insert_str("a\nb\nc\nd");
        let rect = Rect::new(0, 0, 4, 2);
        let mut buf = Buffer::empty(rect);
        let mut state = TextAreaState::default();
        area.render_with_state(rect, &mut buf, &mut state);
        assert_eq!(buf[(0, 0)].symbol(), "c");
        assert_eq!(buf[(0, 1)].symbol(), "d");
    }
}
