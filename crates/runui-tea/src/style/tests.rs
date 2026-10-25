//! Lip Gloss's own tests, ported; the golden tests in `tests/golden.rs`
//! compare more renders against Lip Gloss itself.

use super::*;

fn requires(s: Style, text: &'static str, expected: &str) {
    assert_eq!(s.set_string(text).string(), expected);
}

#[test]
fn underline() {
    let cases = [
        (Style::new().underline(true), "\x1b[4;4ma\x1b[m\x1b[4;4mb\x1b[m\x1b[4m \x1b[m\x1b[4;4mc\x1b[m"),
        (
            Style::new().underline(true).underline_spaces(true),
            "\x1b[4;4ma\x1b[m\x1b[4;4mb\x1b[m\x1b[4m \x1b[m\x1b[4;4mc\x1b[m",
        ),
        (Style::new().underline(true).underline_spaces(false), "\x1b[4;4ma\x1b[m\x1b[4;4mb\x1b[m \x1b[4;4mc\x1b[m"),
        (Style::new().underline_spaces(true), "ab\x1b[4m \x1b[mc"),
        (
            Style::new().underline_style(UNDERLINE_CURLY),
            "\x1b[4;4:3ma\x1b[m\x1b[4;4:3mb\x1b[m\x1b[4m \x1b[m\x1b[4;4:3mc\x1b[m",
        ),
        (
            Style::new().underline_style(UNDERLINE_CURLY).underline_color(color("#FF0000")),
            "\x1b[4;58;2;255;0;0;4:3ma\x1b[m\x1b[4;58;2;255;0;0;4:3mb\x1b[m\x1b[58;2;255;0;0;4m \x1b[m\x1b[4;58;2;255;0;0;4:3mc\x1b[m",
        ),
    ];
    for (s, expected) in cases {
        requires(s, "ab c", expected);
    }
}

#[test]
fn strikethrough() {
    let cases = [
        (Style::new().strikethrough(true), "\x1b[9ma\x1b[m\x1b[9mb\x1b[m\x1b[9m \x1b[m\x1b[9mc\x1b[m"),
        (
            Style::new().strikethrough(true).strikethrough_spaces(true),
            "\x1b[9ma\x1b[m\x1b[9mb\x1b[m\x1b[9m \x1b[m\x1b[9mc\x1b[m",
        ),
        (Style::new().strikethrough(true).strikethrough_spaces(false), "\x1b[9ma\x1b[m\x1b[9mb\x1b[m \x1b[9mc\x1b[m"),
        (Style::new().strikethrough_spaces(true), "ab\x1b[9m \x1b[mc"),
    ];
    for (s, expected) in cases {
        requires(s, "ab c", expected);
    }
}

#[test]
fn style_render() {
    let cases = [
        (Style::new().foreground(color("#5A56E0")), "\x1b[38;2;90;86;224mhello\x1b[m"),
        (Style::new().bold(true), "\x1b[1mhello\x1b[m"),
        (Style::new().italic(true), "\x1b[3mhello\x1b[m"),
        (
            Style::new().underline(true),
            "\x1b[4;4mh\x1b[m\x1b[4;4me\x1b[m\x1b[4;4ml\x1b[m\x1b[4;4ml\x1b[m\x1b[4;4mo\x1b[m",
        ),
        (Style::new().blink(true), "\x1b[5mhello\x1b[m"),
        (Style::new().faint(true), "\x1b[2mhello\x1b[m"),
    ];
    for (s, expected) in cases {
        requires(s, "hello", expected);
    }
}

#[test]
fn value_copy() {
    let s = Style::new().bold(true);
    let i = s;
    let _ = i.bold(false);
    assert_eq!(s.get_bold(), i.get_bold());
}

#[test]
fn style_inherit() {
    let s = Style::new()
        .bold(true)
        .italic(true)
        .underline(true)
        .strikethrough(true)
        .blink(true)
        .faint(true)
        .foreground(color("#ffffff"))
        .background(color("#111111"))
        .margin(&[1, 1, 1, 1])
        .padding(&[1, 1, 1, 1]);
    let i = Style::new().inherit(s);
    assert_eq!(s.get_bold(), i.get_bold());
    assert_eq!(s.get_italic(), i.get_italic());
    assert_eq!(s.get_underline(), i.get_underline());
    assert_eq!(s.get_strikethrough(), i.get_strikethrough());
    assert_eq!(s.get_blink(), i.get_blink());
    assert_eq!(s.get_faint(), i.get_faint());
    assert_eq!(s.get_foreground(), i.get_foreground());
    assert_eq!(s.get_background(), i.get_background());
    assert_ne!(s.get_margin_left(), i.get_margin_left());
    assert_ne!(s.get_margin_right(), i.get_margin_right());
    assert_ne!(s.get_margin_top(), i.get_margin_top());
    assert_ne!(s.get_margin_bottom(), i.get_margin_bottom());
    assert_ne!(s.get_padding_left(), i.get_padding_left());
    assert_ne!(s.get_padding_right(), i.get_padding_right());
    assert_ne!(s.get_padding_top(), i.get_padding_top());
    assert_ne!(s.get_padding_bottom(), i.get_padding_bottom());
}

#[test]
fn style_value() {
    let cases = [
        ("empty", "foo", Style::new(), "foo"),
        ("set string", "foo", Style::new().set_string("bar"), "bar foo"),
        ("set string with bold", "foo", Style::new().set_string("bar").bold(true), "\x1b[1mbar foo\x1b[m"),
        ("new style with string", "foo", Style::new().set_string("bar foobar"), "bar foobar foo"),
        ("margin right", "foo", Style::new().margin_right(1), "foo "),
        ("margin left", "foo", Style::new().margin_left(1), " foo"),
        ("empty text margin right", "", Style::new().margin_right(1), " "),
        ("empty text margin left", "", Style::new().margin_left(1), " "),
    ];
    for (name, text, style, expected) in cases {
        assert_eq!(style.render(text), expected, "{name}");
    }
}

#[test]
fn tab_conversion() {
    assert_eq!(Style::new().render("[\t]"), "[    ]");
    assert_eq!(Style::new().tab_width(2).render("[\t]"), "[  ]");
    assert_eq!(Style::new().tab_width(0).render("[\t]"), "[]");
    assert_eq!(Style::new().tab_width(-1).render("[\t]"), "[\t]");
}

#[test]
fn string_transform() {
    fn upper(s: &str) -> String {
        s.to_uppercase()
    }
    fn reverse(s: &str) -> String {
        s.chars().rev().collect()
    }
    fn same(s: &str) -> String {
        s.to_string()
    }
    let cases: [(&str, fn(&str) -> String, &str); 3] = [
        ("hello", same, "hello"),
        ("raow", upper, "RAOW"),
        ("The quick brown 狐 jumped over the lazy 犬", reverse, "犬 yzal eht revo depmuj 狐 nworb kciuq ehT"),
    ];
    for (input, f, expected) in cases {
        assert_eq!(Style::new().bold(true).transform(f).render(input), format!("\x1b[1m{expected}\x1b[m"));
    }
}

#[test]
fn carriage_return_in_render() {
    let s = Style::new().margin_left(1);
    let got = s.render("Super duper california oranges\r\nHello world\r\n");
    let want = s.render("Super duper california oranges\nHello world\n");
    assert_eq!(got, want);
}

const QUINCES: &str = "The Romans learned from the Greeks that quinces slowly cooked with honey would “set” when cool. The Apicius gives a recipe for preserving whole quinces, stems and leaves attached, in a bath of honey diluted with defrutum: Roman marmalade. Preserves of quince and lemon appear (along with rose, apple, plum and pear) in the Book of ceremonies of the Byzantine Emperor Constantine VII Porphyrogennetos.";

#[test]
fn width_counts_frame() {
    let cases = [
        ("width with borders", Style::new().padding(&[0, 2]).border(normal_border(), &[true])),
        ("width no borders", Style::new().padding(&[0, 2])),
        (
            "width unset borders",
            Style::new().padding(&[0, 2]).border(normal_border(), &[true]).border_left(false).border_right(false),
        ),
    ];
    for (name, style) in cases {
        let content_width = 80 - style.get_horizontal_frame_size();
        let rendered = style.width(content_width).render(QUINCES);
        assert_eq!(width(&rendered), content_width, "{name}");
    }
}

#[test]
fn height_counts_frame() {
    let cases = [
        ("height with borders", Style::new().width(80).padding(&[0, 2]).border(normal_border(), &[true])),
        ("height no borders", Style::new().width(80).padding(&[0, 2])),
        (
            "height unset borders",
            Style::new()
                .width(80)
                .padding(&[0, 2])
                .border(normal_border(), &[true])
                .border_bottom(false)
                .border_top(false),
        ),
    ];
    for (name, style) in cases {
        let content_height = 20 - style.get_vertical_frame_size();
        let rendered = style.height(content_height).render(QUINCES);
        assert_eq!(height(&rendered), content_height, "{name}");
    }
}

#[test]
fn join_vertical_aligns() {
    assert_eq!(join_vertical(LEFT, &["A", "BBBB"]), "A   \nBBBB");
    assert_eq!(join_vertical(RIGHT, &["A", "BBBB"]), "   A\nBBBB");
    assert_eq!(join_vertical(Position(0.25), &["A", "BBBB"]), " A  \nBBBB");
}

#[test]
fn join_horizontal_aligns() {
    assert_eq!(join_horizontal(TOP, &["A", "B\nB\nB\nB"]), "AB\n B\n B\n B");
    assert_eq!(join_horizontal(BOTTOM, &["A", "B\nB\nB\nB"]), " B\n B\n B\nAB");
    assert_eq!(join_horizontal(Position(0.25), &["A", "B\nB\nB\nB"]), " B\nAB\n B\n B");
}

#[test]
fn align_text_vertical_pads() {
    let cases: &[(&str, Position, i32, &str)] = &[
        ("Foo", TOP, 2, "Foo\n"),
        ("Foo", CENTER, 5, "\n\nFoo\n\n"),
        ("Foo", BOTTOM, 5, "\n\n\n\nFoo"),
        ("Foo\nBar", BOTTOM, 5, "\n\n\nFoo\nBar"),
        ("Foo\nBar", CENTER, 5, "\nFoo\nBar\n\n"),
        ("Foo\nBar", TOP, 5, "Foo\nBar\n\n\n"),
        ("Foo\nBar\nBaz", BOTTOM, 5, "\n\nFoo\nBar\nBaz"),
        ("Foo\nBar\nBaz", CENTER, 5, "\nFoo\nBar\nBaz\n"),
        ("Foo\nBar\nBaz", BOTTOM, 3, "Foo\nBar\nBaz"),
        ("Foo\nBar\nBaz", CENTER, 3, "Foo\nBar\nBaz"),
        ("Foo\nBar\nBaz", TOP, 3, "Foo\nBar\nBaz"),
        ("Foo\n\n\n\nBar", BOTTOM, 5, "Foo\n\n\n\nBar"),
        ("Foo\n\n\n\nBar", CENTER, 5, "Foo\n\n\n\nBar"),
        ("Foo\n\n\n\nBar", TOP, 5, "Foo\n\n\n\nBar"),
        ("Foo\nBar\nBaz", CENTER, 9, "\n\n\nFoo\nBar\nBaz\n\n\n"),
        ("Foo\nBar\nBaz", CENTER, 10, "\n\n\nFoo\nBar\nBaz\n\n\n\n"),
    ];
    for (s, pos, h, want) in cases {
        assert_eq!(layout::align_text_vertical(s, *pos, *h), *want, "{s:?} {pos:?} {h}");
    }
}

#[test]
fn style_runes_marks_some() {
    let matched = Style::new().reverse(true);
    let unmatched = Style::new();
    let cases: &[(&str, &[i32], &str)] = &[
        ("hello", &[0], "\x1b[7mh\x1b[mello"),
        ("你好", &[1], "你\x1b[7m好\x1b[m"),
        ("hello 你好", &[6, 7], "hello \x1b[7m你好\x1b[m"),
        ("hello", &[1, 3], "h\x1b[7me\x1b[ml\x1b[7ml\x1b[mo"),
        ("你好", &[0, 1], "\x1b[7m你好\x1b[m"),
    ];
    for (input, indices, expected) in cases {
        assert_eq!(style_runes(input, indices, matched, unmatched), *expected);
    }
}

#[test]
fn rounded_box_with_padding() {
    let s = Style::new().border(rounded_border(), &[]).border_foreground(&[color("#FBBF24")]).padding(&[0, 1]);
    let out = s.render("hi\nthere");
    assert_eq!(crate::ansi::strip(&out), "╭───────╮\n│ hi    │\n│ there │\n╰───────╯");
    assert!(out.starts_with("\x1b[38;2;251;191;36m╭───────╮\x1b[m\n"));
}

#[test]
fn canvas_composes_layers() {
    let frame = "aaaaaa\nbbbbbb\ncccccc";
    let box_ = "\x1b[1mXY\x1b[m";
    let mut c = Canvas::new(6, 3);
    c.compose(&Compositor::new(vec![Layer::new(frame), Layer::new(box_).x(2).y(1).z(1)]));
    assert_eq!(c.render(), "aaaaaa\nbb\x1b[1mXY\x1b[mbb\ncccccc");
}

#[test]
fn canvas_trims_trailing_spaces() {
    let mut c = Canvas::new(5, 2);
    c.compose(&Layer::new("AAA  \nAAA"));
    assert_eq!(c.render(), "AAA\nAAA");
}
