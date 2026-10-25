//! Bubbles' textarea tests, ported.

use super::*;
use crate::tea::{KEY_DOWN, KEY_END, KEY_ENTER, KEY_LEFT, KEY_RIGHT, KEY_UP, MOD_ALT, MOD_SHIFT};

fn nil() -> Msg {
    tea::msg(())
}

fn new_text_area() -> Model {
    let mut t = Model::new();
    t.prompt = "> ".into();
    t.placeholder = "Hello, World!".into();
    t.focus();
    t.update(&nil());
    t
}

fn key_press(c: char) -> Msg {
    tea::msg(KeyPressMsg::char(c))
}

fn key(code: u32, mods: tea::KeyMod) -> Msg {
    tea::msg(KeyPressMsg::with_mods(code, mods))
}

fn send_string(m: &mut Model, s: &str) {
    for c in s.chars() {
        m.update(&key_press(c));
    }
}

fn strip_string(s: &str) -> String {
    ansi::strip(s)
        .split('\n')
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn vertical_scrolling() {
    let mut t = new_text_area();
    t.prompt = String::new();
    t.show_line_numbers = false;
    t.set_height(1);
    t.set_width(20);
    t.char_limit = 100;
    t.update(&nil());
    send_string(&mut t, "This is a really long line that should wrap around the text area.");
    assert!(t.view().contains("the text area."), "{}", t.view());
    t.viewport.borrow_mut().goto_top();
    for line in ["This is a really", "long line that", "should wrap around", "the text area."] {
        let view = t.view();
        assert!(view.contains(line), "{line}: {view}");
        t.viewport.borrow_mut().scroll_down(1);
    }
}

#[test]
fn word_wrap_overflowing() {
    let mut t = new_text_area();
    t.set_height(3);
    t.set_width(20);
    t.char_limit = 500;
    t.update(&nil());
    for c in "Testing Testing Testing Testing Testing Testing Testing Testing".chars() {
        t.update(&key_press(c));
        t.view();
    }
    t.row = 0;
    t.col = 0;
    for c in "Testing".chars() {
        t.update(&key_press(c));
        t.view();
    }
    assert!(t.line_info().width <= 20);
}

#[test]
fn value_soft_wrap() {
    let mut t = new_text_area();
    t.set_width(16);
    t.set_height(10);
    t.char_limit = 500;
    t.update(&nil());
    let input = "Testing Testing Testing Testing Testing Testing Testing Testing";
    for c in input.chars() {
        t.update(&key_press(c));
        t.view();
    }
    assert_eq!(t.value(), input);
}

#[test]
fn set_value() {
    let mut t = new_text_area();
    t.set_value("Foo\nBar\nBaz");
    assert_eq!((t.row, t.col), (2, 3));
    assert_eq!(t.value(), "Foo\nBar\nBaz");
    t.set_value("Test");
    assert_eq!(t.value(), "Test");
}

#[test]
fn insert_string() {
    let mut t = new_text_area();
    send_string(&mut t, "foo baz");
    t.col = 4;
    t.insert_string("bar ");
    assert_eq!(t.value(), "foo bar baz");
}

#[test]
fn can_handle_emoji() {
    let mut t = new_text_area();
    send_string(&mut t, "🧋");
    assert_eq!(t.value(), "🧋");
    t.set_value("🧋🧋🧋");
    assert_eq!(t.value(), "🧋🧋🧋");
    assert_eq!(t.col, 3);
    assert_eq!(t.line_info().char_offset, 6);
}

#[test]
fn vertical_navigation_keeps_cursor_horizontal_position() {
    let mut t = new_text_area();
    t.set_width(20);
    t.set_value("你好你好\nHello");
    t.row = 0;
    t.col = 2;
    let li = t.line_info();
    assert_eq!((li.char_offset, li.column_offset), (4, 2));
    t.update(&key(KEY_DOWN, tea::MOD_NONE));
    let li = t.line_info();
    assert_eq!((li.char_offset, li.column_offset), (4, 4));
}

#[test]
fn vertical_navigation_should_remember_position_while_traversing() {
    let mut t = new_text_area();
    t.set_width(40);
    t.set_value("Hello\nWorld\nThis is a long line.");
    assert_eq!((t.col, t.row), (20, 2));
    t.update(&key(KEY_UP, tea::MOD_NONE));
    assert_eq!((t.col, t.row), (5, 1));
    t.update(&key(KEY_UP, tea::MOD_NONE));
    assert_eq!((t.col, t.row), (5, 0));
    t.update(&key(KEY_DOWN, tea::MOD_NONE));
    t.update(&key(KEY_DOWN, tea::MOD_NONE));
    assert_eq!((t.col, t.row), (20, 2));
    t.update(&key(KEY_UP, tea::MOD_NONE));
    t.update(&key(KEY_LEFT, tea::MOD_NONE));
    assert_eq!((t.col, t.row), (4, 1));
    t.update(&key(KEY_DOWN, tea::MOD_NONE));
    assert_eq!((t.col, t.row), (4, 2));
}

#[test]
fn view() {
    let cases: Vec<(&str, fn(&mut Model), &str, i32, i32)> = vec![
        ("placeholder", |_| {}, ">   1 Hello, World!\n>\n>\n>\n>\n>", 0, 0),
        ("single line", |m| m.set_value("the first line"), ">   1 the first line\n>\n>\n>\n>\n>", 0, 14),
        (
            "multiple lines",
            |m| m.set_value("the first line\nthe second line\nthe third line"),
            ">   1 the first line\n>   2 the second line\n>   3 the third line\n>\n>\n>",
            2,
            14,
        ),
        (
            "single line without line numbers",
            |m| {
                m.set_value("the first line");
                m.show_line_numbers = false;
            },
            "> the first line\n>\n>\n>\n>\n>",
            0,
            14,
        ),
        (
            "single line and custom end of buffer character",
            |m| {
                m.set_value("the first line");
                m.end_of_buffer_character = '*';
            },
            ">   1 the first line\n> *\n> *\n> *\n> *\n> *",
            0,
            14,
        ),
        (
            "single line without line numbers and custom end of buffer character",
            |m| {
                m.set_value("the first line");
                m.show_line_numbers = false;
                m.end_of_buffer_character = '*';
            },
            "> the first line\n> *\n> *\n> *\n> *\n> *",
            0,
            14,
        ),
    ];
    for (name, f, want, row, col) in cases {
        let mut t = new_text_area();
        f(&mut t);
        assert_eq!(strip_string(&t.view()), strip_string(want), "{name}");
        assert_eq!((t.cursor_line_number(), t.line_info().column_offset), (row, col), "{name}");
    }
}

#[test]
fn word_left_stops_at_start() {
    let cases: &[(&str, &str, i32, i32, i32, i32)] = &[
        ("empty input", "", 0, 0, 0, 0),
        ("buffer start before whitespace", " word", 0, 0, 0, 0),
        ("inside leading whitespace", "   word", 0, 2, 0, 0),
        ("multiline whitespace prefix", "  \n  word", 1, 2, 0, 0),
        ("previous word", "first second", 0, 12, 0, 6),
    ];
    for (name, value, row, col, want_row, want_col) in cases {
        let mut t = new_text_area();
        t.set_value(value);
        t.row = *row;
        t.col = *col;
        t.word_left();
        assert_eq!((t.row, t.col), (*want_row, *want_col), "{name}");
    }
}

#[test]
fn word() {
    let mut t = new_text_area();
    t.set_height(3);
    t.set_width(20);
    t.char_limit = 500;
    t.update(&nil());
    for c in "Word1 Word2 Word3 Word4".chars() {
        t.update(&key_press(c));
        t.view();
    }
    assert_eq!(t.word(), "Word4");
    for k in [
        KeyPressMsg { code: KEY_LEFT, mods: MOD_ALT, text: "alt+left".into(), ..Default::default() },
        KeyPressMsg { code: KEY_LEFT, mods: MOD_ALT, text: "alt+left".into(), ..Default::default() },
        KeyPressMsg { code: KEY_RIGHT, text: "right".into(), ..Default::default() },
    ] {
        t.update(&tea::msg(k));
        t.view();
    }
    assert_eq!(t.word(), "Word3");
    for k in [
        KeyPressMsg { code: KEY_END, text: "end".into(), ..Default::default() },
        KeyPressMsg { code: tea::KEY_BACKSPACE, mods: MOD_ALT, text: "alt+backspace".into(), ..Default::default() },
        KeyPressMsg { code: tea::KEY_BACKSPACE, mods: MOD_ALT, text: "alt+backspace".into(), ..Default::default() },
        KeyPressMsg { code: tea::KEY_BACKSPACE, text: "backspace".into(), ..Default::default() },
    ] {
        t.update(&tea::msg(k));
        t.view();
    }
    assert_eq!(t.word(), "Word2");
}

fn selection_area() -> Model {
    let mut t = new_text_area();
    t.prompt = String::new();
    t.show_line_numbers = false;
    t.set_width(20);
    t.set_height(5);
    t
}

#[test]
fn selection_character_forward() {
    let mut t = selection_area();
    send_string(&mut t, "hello world");
    t.cursor_start();
    for _ in 0..5 {
        t.update(&key(KEY_RIGHT, MOD_SHIFT));
    }
    assert_eq!(t.selected_text(), "hello");
    assert!(t.has_selection());
}

#[test]
fn selection_character_backward() {
    let mut t = selection_area();
    send_string(&mut t, "hello");
    for _ in 0..5 {
        t.update(&key(KEY_LEFT, MOD_SHIFT));
    }
    assert_eq!(t.selected_text(), "hello");
}

#[test]
fn selection_word_forward() {
    let mut t = selection_area();
    send_string(&mut t, "hello world foo");
    t.cursor_start();
    t.update(&key(KEY_RIGHT, MOD_SHIFT | MOD_ALT));
    assert_eq!(t.selected_text(), "hello");
}

#[test]
fn selection_multi_line() {
    let mut t = selection_area();
    t.set_height(10);
    send_string(&mut t, "line one");
    t.update(&key(KEY_ENTER, tea::MOD_NONE));
    send_string(&mut t, "line two");
    t.move_to_begin();
    t.update(&key(KEY_DOWN, MOD_SHIFT));
    assert_eq!(t.selected_text(), "line one\n");
    for _ in 0..8 {
        t.update(&key(KEY_RIGHT, MOD_SHIFT));
    }
    assert_eq!(t.selected_text(), "line one\nline two");
}

#[test]
fn select_all_and_clear() {
    let mut t = selection_area();
    send_string(&mut t, "hello world");
    t.select_all();
    assert_eq!(t.selected_text(), "hello world");
    t.clear_selection();
    assert!(!t.has_selection());
    assert_eq!(t.selected_text(), "");
}

#[test]
fn selection_cleared_on_movement_and_type() {
    let mut t = selection_area();
    send_string(&mut t, "hello");
    t.cursor_start();
    t.update(&key(KEY_RIGHT, MOD_SHIFT));
    assert!(t.has_selection());
    t.update(&key(KEY_RIGHT, tea::MOD_NONE));
    assert!(!t.has_selection());
    t.update(&key(KEY_RIGHT, MOD_SHIFT));
    t.update(&key_press('x'));
    assert!(!t.has_selection());
}

#[test]
fn typing_and_deleting_replace_selection() {
    let mut t = selection_area();
    send_string(&mut t, "hello world");
    t.cursor_start();
    for _ in 0..5 {
        t.update(&key(KEY_RIGHT, MOD_SHIFT));
    }
    t.update(&key_press('X'));
    assert_eq!(t.value(), "X world");
    t.select_all();
    t.update(&key(tea::KEY_BACKSPACE, tea::MOD_NONE));
    assert_eq!(t.value(), "");
    send_string(&mut t, "abc");
    t.select_all();
    t.update(&tea::msg(PasteMsg { content: "pasted".into() }));
    assert_eq!(t.value(), "pasted");
}

#[test]
fn delete_selection_multi_line() {
    let mut t = selection_area();
    t.set_height(10);
    send_string(&mut t, "line one");
    t.update(&key(KEY_ENTER, tea::MOD_NONE));
    send_string(&mut t, "line two");
    t.update(&key(KEY_ENTER, tea::MOD_NONE));
    send_string(&mut t, "line three");
    t.move_to_begin();
    for _ in 0..18 {
        t.update(&key(KEY_RIGHT, MOD_SHIFT));
    }
    assert_eq!(t.selected_text(), "line one\nline two\n");
    t.update(&key(tea::KEY_BACKSPACE, tea::MOD_NONE));
    assert_eq!(t.value(), "line three");
}

#[test]
fn selection_range_ordering() {
    let mut t = selection_area();
    send_string(&mut t, "hello world");
    for _ in 0..5 {
        t.update(&key(KEY_LEFT, MOD_SHIFT));
    }
    assert_eq!(t.selected_text(), "world");
}

#[test]
fn newline_and_editing_keys() {
    let mut t = selection_area();
    send_string(&mut t, "ab");
    t.update(&key(KEY_ENTER, tea::MOD_NONE));
    send_string(&mut t, "cd");
    assert_eq!(t.value(), "ab\ncd");
    assert_eq!(t.line_count(), 2);
    assert_eq!((t.line(), t.column()), (1, 2));
    t.update(&tea::msg(KeyPressMsg::parse("ctrl+a")));
    t.update(&key(tea::KEY_BACKSPACE, tea::MOD_NONE));
    assert_eq!(t.value(), "abcd");
    assert_eq!((t.line(), t.column()), (0, 2));
}

#[test]
fn blurred_ignores_keys() {
    let mut t = Model::new();
    send_string(&mut t, "abc");
    assert_eq!(t.value(), "");
}
