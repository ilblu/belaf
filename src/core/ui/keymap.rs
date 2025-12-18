use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyModifiers};

macro_rules! optional_config_struct {
    ($($struct_name:ident, $($key_name:ident),*);*) => {
        $(
            #[derive(Debug, serde::Deserialize, Clone, PartialEq, Eq)]
            pub struct $struct_name {
                $(
                    $key_name: Option<Vec<String>>,
                )*
                pub scroll_many: Option<Vec<String>>,
            }
        )*
    };
}

macro_rules! config_struct {
    ($($struct_name:ident, $($key_name:ident),*);*) => {
        $(
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct $struct_name {
                 $(
                    pub $key_name: (KeyCode, Option<KeyCode>),
                )*
                pub scroll_many: KeyModifiers,
            }
        )*
    };
}

optional_config_struct!(
    ConfigKeymap,
    clear,
    delete_confirm,
    delete_deny,
    exec,
    filter_mode,
    force_redraw,
    log_scroll_back,
    log_scroll_forward,
    log_search_mode,
    log_section_height_decrease,
    log_section_height_increase,
    log_section_toggle,
    quit,
    save_logs,
    scroll_down,
    scroll_end,
    scroll_start,
    scroll_up,
    select_next_panel,
    select_previous_panel,
    sort_by_cpu,
    sort_by_id,
    sort_by_image,
    sort_by_memory,
    sort_by_name,
    sort_by_rx,
    sort_by_state,
    sort_by_status,
    sort_by_tx,
    sort_reset,
    toggle_help,
    toggle_mouse_capture
);

config_struct!(
    Keymap,
    clear,
    delete_confirm,
    delete_deny,
    exec,
    filter_mode,
    force_redraw,
    log_scroll_back,
    log_scroll_forward,
    log_search_mode,
    log_section_height_decrease,
    log_section_height_increase,
    log_section_toggle,
    quit,
    save_logs,
    scroll_down,
    scroll_end,
    scroll_start,
    scroll_up,
    select_next_panel,
    select_previous_panel,
    sort_by_cpu,
    sort_by_id,
    sort_by_image,
    sort_by_memory,
    sort_by_name,
    sort_by_rx,
    sort_by_state,
    sort_by_status,
    sort_by_tx,
    sort_reset,
    toggle_help,
    toggle_mouse_capture
);

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

impl Keymap {
    pub const fn new() -> Self {
        Self {
            clear: (KeyCode::Char('c'), Some(KeyCode::Esc)),
            delete_confirm: (KeyCode::Char('y'), None),
            delete_deny: (KeyCode::Char('n'), None),
            exec: (KeyCode::Char('e'), None),
            filter_mode: (KeyCode::Char('/'), Some(KeyCode::F(1))),
            force_redraw: (KeyCode::Char('f'), None),
            log_scroll_back: (KeyCode::Left, None),
            log_scroll_forward: (KeyCode::Right, None),
            log_search_mode: (KeyCode::Char('#'), None),
            log_section_height_decrease: (KeyCode::Char('-'), None),
            log_section_height_increase: (KeyCode::Char('='), None),
            log_section_toggle: (KeyCode::Char('\\'), None),
            quit: (KeyCode::Char('q'), None),
            save_logs: (KeyCode::Char('s'), None),
            scroll_down: (KeyCode::Down, Some(KeyCode::Char('j'))),
            scroll_end: (KeyCode::End, None),
            scroll_many: KeyModifiers::CONTROL,
            scroll_start: (KeyCode::Home, None),
            scroll_up: (KeyCode::Up, Some(KeyCode::Char('k'))),
            select_next_panel: (KeyCode::Tab, None),
            select_previous_panel: (KeyCode::BackTab, None),
            sort_by_cpu: (KeyCode::Char('4'), None),
            sort_by_id: (KeyCode::Char('6'), None),
            sort_by_image: (KeyCode::Char('7'), None),
            sort_by_memory: (KeyCode::Char('5'), None),
            sort_by_name: (KeyCode::Char('1'), None),
            sort_by_rx: (KeyCode::Char('8'), None),
            sort_by_state: (KeyCode::Char('2'), None),
            sort_by_status: (KeyCode::Char('3'), None),
            sort_by_tx: (KeyCode::Char('9'), None),
            sort_reset: (KeyCode::Char('0'), None),
            toggle_help: (KeyCode::Char('h'), None),
            toggle_mouse_capture: (KeyCode::Char('m'), None),
        }
    }
}

impl From<Option<ConfigKeymap>> for Keymap {
    fn from(value: Option<ConfigKeymap>) -> Self {
        let mut keymap = Self::new();

        let mut clash = HashSet::new();
        let mut counter = 0;

        let mut update_keymap =
            |vec_str: Option<Vec<String>>,
             keymap_field: &mut (KeyCode, Option<KeyCode>),
             keymap_clash: &mut HashSet<KeyCode>| {
                if let Some(vec_str) = vec_str {
                    if let Some(vec_keycode) = Self::try_parse_keycode(&vec_str) {
                        if let Some(first) = vec_keycode.first() {
                            keymap_clash.insert(*first);
                            counter += 1;
                            keymap_field.0 = *first;
                        }
                        if let Some(second) = vec_keycode.get(1) {
                            keymap_clash.insert(*second);
                            counter += 1;
                            keymap_field.1 = Some(*second);
                        } else {
                            keymap_field.1 = None;
                        }
                    }
                }
            };

        if let Some(ck) = value {
            update_keymap(ck.clear, &mut keymap.clear, &mut clash);
            update_keymap(ck.delete_deny, &mut keymap.delete_deny, &mut clash);
            update_keymap(ck.delete_confirm, &mut keymap.delete_confirm, &mut clash);
            update_keymap(
                ck.log_section_height_decrease,
                &mut keymap.log_section_height_decrease,
                &mut clash,
            );
            update_keymap(
                ck.log_section_height_increase,
                &mut keymap.log_section_height_increase,
                &mut clash,
            );
            update_keymap(
                ck.log_section_toggle,
                &mut keymap.log_section_toggle,
                &mut clash,
            );

            update_keymap(ck.exec, &mut keymap.exec, &mut clash);
            update_keymap(ck.filter_mode, &mut keymap.filter_mode, &mut clash);
            update_keymap(ck.force_redraw, &mut keymap.force_redraw, &mut clash);
            update_keymap(ck.quit, &mut keymap.quit, &mut clash);
            update_keymap(ck.save_logs, &mut keymap.save_logs, &mut clash);
            update_keymap(ck.scroll_down, &mut keymap.scroll_down, &mut clash);
            update_keymap(ck.scroll_end, &mut keymap.scroll_end, &mut clash);
            update_keymap(ck.scroll_start, &mut keymap.scroll_start, &mut clash);
            update_keymap(ck.scroll_up, &mut keymap.scroll_up, &mut clash);
            update_keymap(ck.log_search_mode, &mut keymap.log_search_mode, &mut clash);
            update_keymap(
                ck.log_scroll_forward,
                &mut keymap.log_scroll_forward,
                &mut clash,
            );
            update_keymap(ck.log_scroll_back, &mut keymap.log_scroll_back, &mut clash);
            update_keymap(
                ck.select_next_panel,
                &mut keymap.select_next_panel,
                &mut clash,
            );
            update_keymap(
                ck.select_previous_panel,
                &mut keymap.select_previous_panel,
                &mut clash,
            );
            update_keymap(ck.sort_by_name, &mut keymap.sort_by_name, &mut clash);
            update_keymap(ck.sort_by_state, &mut keymap.sort_by_state, &mut clash);
            update_keymap(ck.sort_by_status, &mut keymap.sort_by_status, &mut clash);
            update_keymap(ck.sort_by_cpu, &mut keymap.sort_by_cpu, &mut clash);
            update_keymap(ck.sort_by_memory, &mut keymap.sort_by_memory, &mut clash);
            update_keymap(ck.sort_by_id, &mut keymap.sort_by_id, &mut clash);
            update_keymap(ck.sort_by_image, &mut keymap.sort_by_image, &mut clash);
            update_keymap(ck.sort_by_rx, &mut keymap.sort_by_rx, &mut clash);
            update_keymap(ck.sort_by_tx, &mut keymap.sort_by_tx, &mut clash);
            update_keymap(ck.sort_reset, &mut keymap.sort_reset, &mut clash);
            update_keymap(ck.toggle_help, &mut keymap.toggle_help, &mut clash);
            update_keymap(
                ck.toggle_mouse_capture,
                &mut keymap.toggle_mouse_capture,
                &mut clash,
            );
            if let Some(scroll_many) = Self::try_parse_modifier(ck.scroll_many) {
                keymap.scroll_many = scroll_many;
            }
        }
        if counter == clash.len() {
            keymap
        } else {
            Self::new()
        }
    }
}

impl Keymap {
    fn try_parse_modifier(input: Option<Vec<String>>) -> Option<KeyModifiers> {
        input.and_then(|input| {
            input
                .first()
                .and_then(|input| match input.to_lowercase().trim() {
                    "control" => Some(KeyModifiers::CONTROL),
                    "alt" => Some(KeyModifiers::ALT),
                    "shift" => Some(KeyModifiers::SHIFT),
                    _ => None,
                })
        })
    }

    fn try_parse_keycode(input: &[String]) -> Option<Vec<KeyCode>> {
        let mut output = vec![];

        for key in input.iter().take(2) {
            if key.chars().count() == 1 {
                if let Some(first_char) = key.chars().next() {
                    if let Some(first_char) = match first_char {
                        x if x.is_ascii_alphabetic() || x.is_ascii_digit() => Some(first_char),
                        '/' | '\\' | ',' | '.' | '#' | '\'' | '[' | ']' | ';' | '=' | '-' => {
                            Some(first_char)
                        }
                        _ => None,
                    } {
                        output.push(KeyCode::Char(first_char));
                    }
                }
            } else {
                let keycode = match key.to_lowercase().as_str() {
                    "f1" => Some(KeyCode::F(1)),
                    "f2" => Some(KeyCode::F(2)),
                    "f3" => Some(KeyCode::F(3)),
                    "f4" => Some(KeyCode::F(4)),
                    "f5" => Some(KeyCode::F(5)),
                    "f6" => Some(KeyCode::F(6)),
                    "f7" => Some(KeyCode::F(7)),
                    "f8" => Some(KeyCode::F(8)),
                    "f9" => Some(KeyCode::F(9)),
                    "f10" => Some(KeyCode::F(10)),
                    "f11" => Some(KeyCode::F(11)),
                    "f12" => Some(KeyCode::F(12)),
                    "backspace" => Some(KeyCode::Backspace),
                    "backtab" => Some(KeyCode::BackTab),
                    "delete" => Some(KeyCode::Delete),
                    "down" => Some(KeyCode::Down),
                    "end" => Some(KeyCode::End),
                    "esc" => Some(KeyCode::Esc),
                    "home" => Some(KeyCode::Home),
                    "insert" => Some(KeyCode::Insert),
                    "left" => Some(KeyCode::Left),
                    "pagedown" => Some(KeyCode::PageDown),
                    "pageup" => Some(KeyCode::PageUp),
                    "right" => Some(KeyCode::Right),
                    "tab" => Some(KeyCode::Tab),
                    "up" => Some(KeyCode::Up),
                    _ => None,
                };
                if let Some(a) = keycode {
                    output.push(a);
                }
            }
        }
        if output.is_empty() {
            None
        } else {
            if output.first() == output.get(1) {
                output.pop();
            }
            Some(output)
        }
    }
}
