use crate::paste_burst::{CharDecision, FlushResult, PasteBurst};
use crate::textarea::{clamp_to_char_boundary, TextArea, TextAreaState};
use crossterm::event::{self, KeyCode, KeyModifiers};
use std::time::Instant;

#[derive(Default)]
pub(crate) struct ChatComposer {
    pub(crate) input: TextArea,
    pub(crate) input_state: TextAreaState,
    pub(crate) paste_burst: PasteBurst,
    pub(crate) history: Vec<String>,
    pub(crate) history_pos: Option<usize>,
    pub(crate) history_draft: String,
}

pub(crate) enum InputAction {
    None,
    RequestDraw,
    Submit,
    Quit,
    Interrupt,
    OpenTranscriptReview,
    OpenWatchdogReview,
    CloseReview,
    ScrollUp,
    ScrollDown,
    ReturnToLatest,
    JumpToTop,
    PageUp,
    PageDown,
}

impl ChatComposer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn submit_input_text(&mut self) -> Option<String> {
        let text = self.input.lines().join("\n");
        self.submit_text(text)
    }

    pub(crate) fn submit_text(&mut self, text: String) -> Option<String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }

        if self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        self.history_pos = None;
        self.history_draft.clear();
        self.input = TextArea::default();
        self.input_state = TextAreaState::default();
        Some(text)
    }

    pub(crate) fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let new_pos = match self.history_pos {
            None => {
                self.history_draft = self.input.lines().join("\n");
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_pos = Some(new_pos);
        set_input_text(
            &mut self.input,
            &mut self.input_state,
            &self.history[new_pos].clone(),
        );
    }

    pub(crate) fn history_down(&mut self) {
        match self.history_pos {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.history_pos = Some(i + 1);
                let text = self.history[i + 1].clone();
                set_input_text(&mut self.input, &mut self.input_state, &text);
            }
            Some(_) => {
                self.history_pos = None;
                let draft = self.history_draft.clone();
                set_input_text(&mut self.input, &mut self.input_state, &draft);
            }
        }
    }

    pub(crate) fn handle_paste_event(&mut self, text: String) {
        commit_pasted_input(
            &mut self.input,
            &mut self.input_state,
            &mut self.paste_burst,
            text,
        );
    }

    pub(crate) fn handle_key_event(
        &mut self,
        key: event::KeyEvent,
        review_open: bool,
        agent_busy: bool,
    ) -> InputAction {
        let now = Instant::now();
        match self.paste_burst.flush_if_due(now) {
            FlushResult::Paste(text) => {
                commit_pasted_input(
                    &mut self.input,
                    &mut self.input_state,
                    &mut self.paste_burst,
                    text,
                );
                return InputAction::RequestDraw;
            }
            FlushResult::Typed(ch) => {
                self.input.insert_char(ch);
                return InputAction::RequestDraw;
            }
            FlushResult::None => {}
        }

        if matches!(key.code, KeyCode::Enter)
            && self.paste_burst.is_active()
            && self.paste_burst.append_newline_if_active(now)
        {
            return InputAction::None;
        }

        if let KeyCode::Char(ch) = key.code {
            let has_ctrl_or_alt = key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT);
            if !has_ctrl_or_alt {
                if !ch.is_ascii() {
                    self.handle_non_ascii_char(key);
                    return InputAction::RequestDraw;
                }

                if let Some(decision) = self.paste_burst.on_plain_char_no_hold(now) {
                    match decision {
                        CharDecision::BufferAppend => {
                            self.paste_burst.append_char_to_buffer(ch, now);
                            return InputAction::None;
                        }
                        CharDecision::BeginBuffer { retro_chars } => {
                            let (text, byte_pos) = input_text_and_cursor_byte(&self.input);
                            let safe_cursor = clamp_to_char_boundary(&text, byte_pos);
                            let before = &text[..safe_cursor];
                            if let Some(grab) = self.paste_burst.decide_begin_buffer(
                                now,
                                before,
                                retro_chars as usize,
                            ) {
                                let kept =
                                    format!("{}{}", &text[..grab.start_byte], &text[safe_cursor..]);
                                set_input_text_and_cursor(
                                    &mut self.input,
                                    &mut self.input_state,
                                    &kept,
                                    grab.start_byte,
                                );
                                self.paste_burst.append_char_to_buffer(ch, now);
                                return InputAction::None;
                            }
                        }
                    }
                }
            }

            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                commit_pasted_input(
                    &mut self.input,
                    &mut self.input_state,
                    &mut self.paste_burst,
                    pasted,
                );
            }
        }

        if !matches!(key.code, KeyCode::Char(_) | KeyCode::Enter) {
            if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                commit_pasted_input(
                    &mut self.input,
                    &mut self.input_state,
                    &mut self.paste_burst,
                    pasted,
                );
            }
        }

        let mods = key.modifiers;
        let alt = mods.contains(KeyModifiers::ALT);
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let shift = mods.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Char('c') if ctrl => InputAction::Quit,
            KeyCode::Esc if review_open => InputAction::CloseReview,
            KeyCode::Esc if agent_busy => InputAction::Interrupt,
            KeyCode::Char('s') if ctrl => InputAction::Submit,
            KeyCode::Enter if mods.is_empty() => {
                if review_open {
                    InputAction::CloseReview
                } else if self
                    .paste_burst
                    .newline_should_insert_instead_of_submit(now)
                {
                    self.input.insert_newline();
                    self.paste_burst.extend_window(now);
                    InputAction::RequestDraw
                } else {
                    InputAction::Submit
                }
            }
            KeyCode::Enter if shift && !alt && !ctrl => {
                if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                    commit_pasted_input(
                        &mut self.input,
                        &mut self.input_state,
                        &mut self.paste_burst,
                        pasted,
                    );
                }
                self.input.insert_newline();
                InputAction::RequestDraw
            }
            KeyCode::Char('j') if ctrl && !alt => {
                if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
                    commit_pasted_input(
                        &mut self.input,
                        &mut self.input_state,
                        &mut self.paste_burst,
                        pasted,
                    );
                }
                self.input.insert_newline();
                InputAction::RequestDraw
            }
            KeyCode::PageUp => InputAction::PageUp,
            KeyCode::PageDown => InputAction::PageDown,
            KeyCode::Up if alt => InputAction::ScrollUp,
            KeyCode::Down if alt => InputAction::ScrollDown,
            KeyCode::Char('g') if alt => InputAction::ReturnToLatest,
            KeyCode::Char('t') if ctrl && !alt => {
                if review_open {
                    InputAction::CloseReview
                } else {
                    InputAction::OpenTranscriptReview
                }
            }
            KeyCode::Char('w') if ctrl && !alt => {
                if review_open {
                    InputAction::CloseReview
                } else {
                    InputAction::OpenWatchdogReview
                }
            }
            KeyCode::Home if alt => InputAction::JumpToTop,
            KeyCode::Up if mods.is_empty() && !review_open => {
                self.history_up();
                InputAction::RequestDraw
            }
            KeyCode::Down if mods.is_empty() && !review_open => {
                self.history_down();
                InputAction::RequestDraw
            }
            _ => {
                if !review_open {
                    self.input.input(key);
                    match key.code {
                        KeyCode::Char(_) => {
                            let has_ctrl_or_alt = key.modifiers.contains(KeyModifiers::CONTROL)
                                || key.modifiers.contains(KeyModifiers::ALT);
                            if has_ctrl_or_alt {
                                self.paste_burst.clear_window_after_non_char();
                            }
                        }
                        KeyCode::Enter => {}
                        _ => self.paste_burst.clear_window_after_non_char(),
                    }
                    InputAction::RequestDraw
                } else {
                    InputAction::None
                }
            }
        }
    }

    fn handle_non_ascii_char(&mut self, key: event::KeyEvent) {
        if let Some(pasted) = self.paste_burst.flush_before_modified_input() {
            commit_pasted_input(
                &mut self.input,
                &mut self.input_state,
                &mut self.paste_burst,
                pasted,
            );
        }
        self.input.input(key);
    }
}

fn commit_pasted_input(
    input: &mut TextArea,
    input_state: &mut TextAreaState,
    paste_burst: &mut PasteBurst,
    pasted: String,
) {
    insert_pasted_text(input, &pasted);
    *input_state = TextAreaState::default();
    paste_burst.clear_after_explicit_paste();
}

fn set_input_text(input: &mut TextArea, input_state: &mut TextAreaState, text: &str) {
    *input = TextArea::default();
    *input_state = TextAreaState::default();
    if !text.is_empty() {
        input.insert_str(text);
    }
}

fn set_input_text_and_cursor(
    input: &mut TextArea,
    input_state: &mut TextAreaState,
    text: &str,
    cursor_byte: usize,
) {
    set_input_text(input, input_state, text);
    let cursor_byte = clamp_to_char_boundary(text, cursor_byte);
    let mut row = 0usize;
    let mut col = 0usize;
    let mut remaining = cursor_byte;
    for line in text.split('\n') {
        if remaining <= line.len() {
            col = line[..remaining].chars().count();
            break;
        }
        remaining = remaining.saturating_sub(line.len() + 1);
        row += 1;
    }
    input.move_cursor_jump(row, col);
}

fn input_text_and_cursor_byte(input: &TextArea) -> (String, usize) {
    let lines = input.lines();
    let text = lines.join("\n");
    let (row, col) = input.cursor();
    let mut byte_pos = 0usize;
    for (idx, line) in lines.iter().enumerate() {
        if idx == row {
            let safe_col = col.min(line.chars().count());
            byte_pos += line
                .char_indices()
                .nth(safe_col)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            break;
        }
        byte_pos += line.len() + 1;
    }
    (text, byte_pos)
}

fn insert_pasted_text(input: &mut TextArea, text: &str) {
    if text.is_empty() {
        return;
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    input.insert_str(normalized);
}
