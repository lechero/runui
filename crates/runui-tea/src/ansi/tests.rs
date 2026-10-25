//! x/ansi's own tests for these functions, ported. Cases with 8-bit (C1)
//! control bytes are left out: they aren't valid UTF-8, so they can't be a
//! Rust string.

use super::*;

struct WidthCase {
    name: &'static str,
    input: &'static str,
    stripped: &'static str,
    width: i32,
}

const WIDTH_CASES: &[WidthCase] = &[
    WidthCase { name: "empty", input: "", stripped: "", width: 0 },
    WidthCase { name: "ascii", input: "hello", stripped: "hello", width: 5 },
    WidthCase { name: "emoji", input: "👋", stripped: "👋", width: 2 },
    WidthCase { name: "wideemoji", input: "🫧", stripped: "🫧", width: 2 },
    WidthCase { name: "combining", input: "a\u{0300}", stripped: "a\u{0300}", width: 1 },
    WidthCase { name: "control", input: "\x1b[31mhello\x1b[0m", stripped: "hello", width: 5 },
    WidthCase { name: "controlemoji", input: "\x1b[31m👋\x1b[0m", stripped: "👋", width: 2 },
    WidthCase { name: "oscwideemoji", input: "\x1b]2;title👨‍👩‍👦\x07", stripped: "", width: 0 },
    WidthCase {
        name: "oscwideemoji", input: "\x1b[31m👨‍👩‍👦\x1b[m", stripped: "👨\u{200d}👩\u{200d}👦", width: 2
    },
    WidthCase { name: "dcsarabic", input: "\x1bP?123$pسلام\x1b\\اهلا", stripped: "اهلا", width: 4 },
    WidthCase { name: "newline", input: "hello\nworld", stripped: "hello\nworld", width: 10 },
    WidthCase { name: "tab", input: "hello\tworld", stripped: "hello\tworld", width: 10 },
    WidthCase { name: "controlnewline", input: "\x1b[31mhello\x1b[0m\nworld", stripped: "hello\nworld", width: 10 },
    WidthCase { name: "style", input: "\x1B[38;2;249;38;114mfoo", stripped: "foo", width: 3 },
    WidthCase { name: "unicode", input: "\x1b[35m“box”\x1b[0m", stripped: "“box”", width: 5 },
    WidthCase { name: "just_unicode", input: "Claire’s Boutique", stripped: "Claire’s Boutique", width: 17 },
    WidthCase { name: "unclosed_ansi", input: "Hey, \x1b[7m\n猴", stripped: "Hey, \n猴", width: 7 },
    WidthCase { name: "double_asian_runes", input: " 你\x1b[8m好.", stripped: " 你好.", width: 6 },
    WidthCase { name: "flag", input: "🇸🇦", stripped: "🇸🇦", width: 2 },
    WidthCase { name: "half width and ascii", input: "(ﾟ", stripped: "(ﾟ", width: 1 },
];

#[test]
fn strip_removes_sequences() {
    for c in WIDTH_CASES {
        assert_eq!(strip(c.input), c.stripped, "{}", c.name);
    }
}

#[test]
fn string_width_counts_cells() {
    for c in WIDTH_CASES {
        assert_eq!(string_width(c.input), c.width, "{}", c.name);
    }
}

struct TruncateCase {
    name: &'static str,
    input: &'static str,
    extra: &'static str,
    width: i32,
    expect_right: &'static str,
    expect_left: &'static str,
}

const JAPANESE: &str = "耐許ヱヨカハ調出あゆ監件び理別よン國給災レホチ権輝モエフ会割もフ響3現エツ文時しだびほ経機ムイメフ敗文ヨク現義なさド請情ゆじょて憶主管州けでふく。排ゃわつげ美刊ヱミ出見ツ南者オ抜豆ハトロネ論索モネニイ任償スヲ話破リヤヨ秒止口イセソス止央のさ食周健でてつだ官送ト読聴遊容ひるべ。際ぐドらづ市居ネムヤ研校35岩6繹ごわク報拐イ革深52球ゃレスご究東スラ衝3間ラ録占たス。\n\n禁にンご忘康ざほぎル騰般ねど事超スんいう真表何カモ自浩ヲシミ図客線るふ静王ぱーま写村月掛焼詐面ぞゃ。昇強ごントほ価保キ族85岡モテ恋困ひりこな刊並せご出来ぼぎむう点目ヲウ止環公ニレ事応タス必書タメムノ当84無信升ちひょ。価ーぐ中客テサ告覧ヨトハ極整\nラ得95稿はかラせ江利ス宏丸霊ミ考整ス静将ず業巨職ノラホ収嗅ざな。";
const JAPANESE_LEFT: &str = "出あゆ監件び理別よン國給災レホチ権輝モエフ会割もフ響3現エツ文時しだびほ経機ムイメフ敗文ヨク現義なさド請情ゆじょて憶主管州けでふく。排ゃわつげ美刊ヱミ出見ツ南者オ抜豆ハトロネ論索モネニイ任償スヲ話破リヤヨ秒止口イセソス止央のさ食周健でてつだ官送ト読聴遊容ひるべ。際ぐドらづ市居ネムヤ研校35岩6繹ごわク報拐イ革深52球ゃレスご究東スラ衝3間ラ録占たス。\n\n禁にンご忘康ざほぎル騰般ねど事超スんいう真表何カモ自浩ヲシミ図客線るふ静王ぱーま写村月掛焼詐面ぞゃ。昇強ごントほ価保キ族85岡モテ恋困ひりこな刊並せご出来ぼぎむう点目ヲウ止環公ニレ事応タス必書タメムノ当84無信升ちひょ。価ーぐ中客テサ告覧ヨトハ極整\nラ得95稿はかラせ江利ス宏丸霊ミ考整ス静将ず業巨職ノラホ収嗅ざな。";

const TRUNCATE_CASES: &[TruncateCase] = &[
    TruncateCase { name: "empty", input: "", extra: "", width: 0, expect_right: "", expect_left: "" },
    TruncateCase { name: "truncate_length_0", input: "foo", extra: "", width: 0, expect_right: "", expect_left: "foo" },
    TruncateCase { name: "equalascii", input: "one", extra: ".", width: 3, expect_right: "one", expect_left: "" },
    TruncateCase {
        name: "equalemoji", input: "on👋", extra: ".", width: 3, expect_right: "on.", expect_left: ".👋"
    },
    TruncateCase {
        name: "simple multiple words",
        input: "a couple of words",
        extra: "",
        width: 6,
        expect_right: "a coup",
        expect_left: "le of words",
    },
    TruncateCase {
        name: "equalcontrolemoji",
        input: "one\x1b[0m",
        extra: ".",
        width: 3,
        expect_right: "one\x1b[0m",
        expect_left: "\x1b[0m",
    },
    TruncateCase {
        name: "truncate_tail_greater",
        input: "foo",
        extra: "...",
        width: 5,
        expect_right: "foo",
        expect_left: "",
    },
    TruncateCase { name: "simple", input: "foobar", extra: "", width: 3, expect_right: "foo", expect_left: "bar" },
    TruncateCase {
        name: "passthrough",
        input: "foobar",
        extra: "",
        width: 10,
        expect_right: "foobar",
        expect_left: "",
    },
    TruncateCase { name: "ascii", input: "hello", extra: "", width: 3, expect_right: "hel", expect_left: "lo" },
    TruncateCase { name: "emoji", input: "👋", extra: "", width: 2, expect_right: "👋", expect_left: "" },
    TruncateCase { name: "wideemoji", input: "🫧", extra: "", width: 2, expect_right: "🫧", expect_left: "" },
    TruncateCase {
        name: "controlemoji",
        input: "\x1b[31mhello 👋abc\x1b[0m",
        extra: "",
        width: 8,
        expect_right: "\x1b[31mhello 👋\x1b[0m",
        expect_left: "\x1b[31mabc\x1b[0m",
    },
    TruncateCase {
        name: "osc8",
        input: "\x1b]8;;https://charm.sh\x1b\\Charmbracelet 🫧\x1b]8;;\x1b\\",
        extra: "",
        width: 5,
        expect_right: "\x1b]8;;https://charm.sh\x1b\\Charm\x1b]8;;\x1b\\",
        expect_left: "\x1b]8;;https://charm.sh\x1b\\bracelet 🫧\x1b]8;;\x1b\\",
    },
    TruncateCase {
        name: "style_tail",
        input: "\x1B[38;5;219mHiya!",
        extra: "…",
        width: 3,
        expect_right: "\x1B[38;5;219mHi…",
        expect_left: "\x1B[38;5;219m…a!",
    },
    TruncateCase {
        name: "double_style_tail",
        input: "\x1B[38;5;219mHiya!\x1B[38;5;219mHello",
        extra: "…",
        width: 7,
        expect_right: "\x1B[38;5;219mHiya!\x1B[38;5;219mH…",
        expect_left: "\x1B[38;5;219m\x1B[38;5;219m…llo",
    },
    TruncateCase {
        name: "noop",
        input: "\x1B[7m--",
        extra: "",
        width: 2,
        expect_right: "\x1B[7m--",
        expect_left: "\x1b[7m",
    },
    TruncateCase {
        name: "double_width",
        input: "\x1B[38;2;249;38;114m你好\x1B[0m",
        extra: "",
        width: 3,
        expect_right: "\x1B[38;2;249;38;114m你\x1B[0m",
        expect_left: "\x1B[38;2;249;38;114m好\x1B[0m",
    },
    TruncateCase { name: "double_width_rune", input: "你", extra: "", width: 1, expect_right: "", expect_left: "你" },
    TruncateCase {
        name: "double_width_runes",
        input: "你好",
        extra: "",
        width: 2,
        expect_right: "你",
        expect_left: "好",
    },
    TruncateCase {
        name: "spaces_only", input: "    ", extra: "…", width: 2, expect_right: " …", expect_left: "…  "
    },
    TruncateCase { name: "longer_tail", input: "foo", extra: "...", width: 2, expect_right: "", expect_left: "...o" },
    TruncateCase {
        name: "same_tail_width",
        input: "foo",
        extra: "...",
        width: 3,
        expect_right: "foo",
        expect_left: "",
    },
    TruncateCase {
        name: "same_tail_width_control",
        input: "\x1b[31mfoo\x1b[0m",
        extra: "...",
        width: 3,
        expect_right: "\x1b[31mfoo\x1b[0m",
        expect_left: "\x1b[31m\x1b[0m",
    },
    TruncateCase { name: "same_width", input: "foo", extra: "", width: 3, expect_right: "foo", expect_left: "" },
    TruncateCase {
        name: "truncate_with_tail",
        input: "foobar",
        extra: ".",
        width: 4,
        expect_right: "foo.",
        expect_left: ".ar",
    },
    TruncateCase {
        name: "style",
        input: "I really \x1B[38;2;249;38;114mlove\x1B[0m Go!",
        extra: "",
        width: 8,
        expect_right: "I really\x1B[38;2;249;38;114m\x1B[0m",
        expect_left: " \x1B[38;2;249;38;114mlove\x1B[0m Go!",
    },
    TruncateCase {
        name: "dcs",
        input: "\x1BPq#0;2;0;0;0#1;2;100;100;0#2;2;0;100;0#1~~@@vv@@~~@@~~$#2??}}GG}}??}}??-#1!14@\x1B\\foobar",
        extra: "…",
        width: 4,
        expect_right: "\x1BPq#0;2;0;0;0#1;2;100;100;0#2;2;0;100;0#1~~@@vv@@~~@@~~$#2??}}GG}}??}}??-#1!14@\x1B\\foo…",
        expect_left: "\x1BPq#0;2;0;0;0#1;2;100;100;0#2;2;0;100;0#1~~@@vv@@~~@@~~$#2??}}GG}}??}}??-#1!14@\x1B\\…ar",
    },
    TruncateCase {
        name: "emoji_tail",
        input: "\x1b[36mHello there!\x1b[m",
        extra: "😃",
        width: 8,
        expect_right: "\x1b[36mHello 😃\x1b[m",
        expect_left: "\x1b[36m😃ere!\x1b[m",
    },
    TruncateCase {
        name: "unicode",
        input: "\x1b[35mClaire‘s Boutique\x1b[0m",
        extra: "",
        width: 8,
        expect_right: "\x1b[35mClaire‘s\x1b[0m",
        expect_left: "\x1b[35m Boutique\x1b[0m",
    },
    TruncateCase {
        name: "wide_chars",
        input: "こんにちは",
        extra: "…",
        width: 7,
        expect_right: "こんに…",
        expect_left: "…ちは",
    },
    TruncateCase {
        name: "style_wide_chars",
        input: "\x1b[35mこんにちは\x1b[m",
        extra: "…",
        width: 7,
        expect_right: "\x1b[35mこんに…\x1b[m",
        expect_left: "\x1b[35m…ちは\x1b[m",
    },
    TruncateCase {
        name: "osc8_lf",
        input: "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\\nสวัสดีสวัสดี\x1b]8;;\x1b\\",
        extra: "…",
        width: 9,
        expect_right: "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\\n…\x1b]8;;\x1b\\",
        expect_left: "\x1b]8;;https://example.com\x1b\\…วัสดีสวัสดี\x1b]8;;\x1b\\",
    },
    TruncateCase {
        name: "simple japanese text prefix/suffix",
        input: "耐許ヱヨカハ調出あゆ監",
        extra: "…",
        width: 13,
        expect_right: "耐許ヱヨカハ…",
        expect_left: "…調出あゆ監",
    },
    TruncateCase {
        name: "simple japanese text",
        input: "耐許ヱヨカハ調出あゆ監",
        extra: "",
        width: 14,
        expect_right: "耐許ヱヨカハ調",
        expect_left: "出あゆ監",
    },
    TruncateCase {
        name: "new line inside and outside range",
        input: "\n\nsomething\nin\nthe\nway\n\n",
        extra: "-",
        width: 10,
        expect_right: "\n\nsomething\n-",
        expect_left: "-n\nthe\nway\n\n",
    },
    TruncateCase {
        name: "multi-width graphemes with newlines - japanese text",
        input: JAPANESE,
        extra: "",
        width: 14,
        expect_right: "耐許ヱヨカハ調",
        expect_left: JAPANESE_LEFT,
    },
];

#[test]
fn truncate_cuts_right() {
    for c in TRUNCATE_CASES {
        assert_eq!(truncate(c.input, c.width, c.extra), c.expect_right, "{}", c.name);
    }
}

#[test]
fn truncate_left_cuts_left() {
    for c in TRUNCATE_CASES {
        assert_eq!(truncate_left(c.input, c.width, c.extra), c.expect_left, "{}", c.name);
    }
}

#[test]
fn cut_keeps_a_range() {
    let cases: &[(&str, &str, i32, i32, &str)] = &[
        ("simple string", "This is a long string", 2, 6, "is i"),
        (
            "with ansi",
            "I really \x1B[38;2;249;38;114mlove\x1B[0m Go!",
            4,
            25,
            "ally \x1b[38;2;249;38;114mlove\x1b[0m Go!",
        ),
        ("left is 0", "Foo \x1B[38;2;249;38;114mbar\x1B[0mbaz", 0, 5, "Foo \x1B[38;2;249;38;114mb\x1B[0m"),
        ("right is 0", "\x1b[7mHello\x1b[m", 3, 0, ""),
        ("right is less than left", "\x1b[7mHello\x1b[m", 3, 2, ""),
        ("cut size is 0", "\x1b[7mHello\x1b[m", 2, 2, ""),
        (
            "maintains open ansi",
            "\x1b[38;5;212;48;5;63mHello, Artichoke!\x1b[m",
            7,
            16,
            "\x1b[38;5;212;48;5;63mArtichoke\x1b[m",
        ),
        (
            "multiline",
            "\n\x1b[38;2;98;98;98m\nif [ -f RE\nADME.md ]; then\x1b[m\n\x1b[38;2;98;98;98m    echo oi\x1b[m\n\x1b[38;2;98;98;98mfi\x1b[m\n",
            8,
            13,
            "\x1b[38;2;98;98;98mRE\nADM\x1b[m\x1b[38;2;98;98;98m\x1b[m\x1b[38;2;98;98;98m\x1b[m",
        ),
    ];
    for (desc, input, left, right, expect) in cases {
        assert_eq!(cut(input, *left, *right), *expect, "{desc}");
    }
}

const WRAP_INPUT_STYLE: &str =
    "\x1B[38;2;249;38;114m(\x1B[0m\x1B[38;2;248;248;242mjust another test\x1B[38;2;249;38;114m)\x1B[0m";
const STYLE_FOOBAR: &str =
    "\x1B[38;2;249;38;114mfoo\x1B[0m\x1B[38;2;248;248;242m \x1B[0m\x1B[38;2;230;219;116mbar\x1B[0m";

#[test]
fn hardwrap_breaks_anywhere() {
    let cases: &[(&str, &str, i32, &str, bool)] = &[
        ("empty string", "", 0, "", true),
        ("passthrough", "foobar\n ", 0, "foobar\n ", true),
        ("pass", "foo", 4, "foo", true),
        ("simple", "foobarfoo", 4, "foob\narfo\no", true),
        ("lf", "f\no\nobar", 3, "f\no\noba\nr", true),
        ("lf_space", "foo bar\n  baz", 3, "foo\n ba\nr\n  b\naz", true),
        ("tab", "foo\tbar", 3, "foo\n\tbar", true),
        ("unicode_space", "foo\u{a0}bar", 3, "foo\nbar", false),
        ("style_nochange", STYLE_FOOBAR, 7, STYLE_FOOBAR, true),
        (
            "style",
            WRAP_INPUT_STYLE,
            3,
            "\x1B[38;2;249;38;114m(\x1B[0m\x1B[38;2;248;248;242mju\nst \nano\nthe\nr t\nest\x1B[38;2;249;38;114m\n)\x1B[0m",
            true,
        ),
        (
            "style_lf",
            "I really \x1B[38;2;249;38;114mlove\x1B[0m Go!",
            8,
            "I really\n\x1b[38;2;249;38;114mlove\x1b[0m Go!",
            false,
        ),
        (
            "style_emoji",
            "I really \x1B[38;2;249;38;114mlove u🫧\x1B[0m",
            8,
            "I really\n\x1b[38;2;249;38;114mlove u🫧\x1b[0m",
            false,
        ),
        (
            "hyperlink",
            "I really \x1B]8;;https://example.com/\x1B\\love\x1B]8;;\x1B\\ Go!",
            10,
            "I really \x1b]8;;https://example.com/\x1b\\l\nove\x1b]8;;\x1b\\ Go!",
            false,
        ),
        (
            "dcs",
            "\x1BPq#0;2;0;0;0#1;2;100;100;0#2;2;0;100;0#1~~@@vv@@~~@@~~$#2??}}GG}}??}}??-#1!14@\x1B\\foobar",
            3,
            "\x1BPq#0;2;0;0;0#1;2;100;100;0#2;2;0;100;0#1~~@@vv@@~~@@~~$#2??}}GG}}??}}??-#1!14@\x1B\\foo\nbar",
            false,
        ),
        ("begin_with_space", " foo", 4, " foo", false),
        ("style_dont_affect_wrap", STYLE_FOOBAR, 7, STYLE_FOOBAR, false),
        (
            "preserve_style",
            WRAP_INPUT_STYLE,
            3,
            "\x1B[38;2;249;38;114m(\x1B[0m\x1B[38;2;248;248;242mju\nst \nano\nthe\nr t\nest\x1B[38;2;249;38;114m\n)\x1B[0m",
            false,
        ),
        ("emoji", "foo🫧foobar", 4, "foo\n🫧fo\nobar", false),
        (
            "osc8_wrap",
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\สวัสดีสวัสดี\x1b]8;;\x1b\\",
            8,
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\\nสวัสดีสวัสดี\x1b]8;;\x1b\\",
            false,
        ),
        ("column", "VERTICAL", 1, "V\nE\nR\nT\nI\nC\nA\nL", false),
    ];
    for (name, input, limit, expected, preserve) in cases {
        assert_eq!(hardwrap(input, *limit, *preserve), *expected, "{name}");
    }
}

#[test]
fn wordwrap_breaks_between_words() {
    let cases: &[(&str, &str, i32, &str, &str)] = &[
        ("empty string", "", 0, "", ""),
        ("passthrough", "foobar\n ", 0, "", "foobar\n "),
        ("pass", "foo", 3, "", "foo"),
        ("toolong", "foobarfoo", 4, "", "foobarfoo"),
        ("white space", "foo bar foo", 4, "", "foo\nbar\nfoo"),
        ("broken_at_spaces", "foo bars foobars", 4, "", "foo\nbars\nfoobars"),
        ("hyphen", "foo-foobar", 4, "-", "foo-\nfoobar"),
        ("emoji_breakpoint", "foo😃 foobar", 4, "😃", "foo😃\nfoobar"),
        ("wide_emoji_breakpoint", "foo🫧 foobar", 4, "🫧", "foo🫧\nfoobar"),
        ("space_breakpoint", "foo --bar", 9, "-", "foo --bar"),
        ("simple", "foo bars foobars", 4, "", "foo\nbars\nfoobars"),
        ("limit", "foo bar", 5, "", "foo\nbar"),
        ("remove white spaces", "foo    \nb   ar   ", 4, "", "foo\nb\nar"),
        ("white space trail width", "foo\nb\t a\n bar", 4, "", "foo\nb\t a\n bar"),
        ("explicit_line_break", "foo bar foo\n", 4, "", "foo\nbar\nfoo\n"),
        ("explicit_breaks", "\nfoo bar\n\n\nfoo\n", 4, "", "\nfoo\nbar\n\n\nfoo\n"),
        (
            "example",
            " This is a list: \n\n\t* foo\n\t* bar\n\n\n\t* foo  \nbar    ",
            6,
            "",
            " This\nis a\nlist: \n\n\t* foo\n\t* bar\n\n\n\t* foo\nbar",
        ),
        ("style_code_dont_affect_length", STYLE_FOOBAR, 7, "", STYLE_FOOBAR),
        (
            "style_code_dont_get_wrapped",
            WRAP_INPUT_STYLE,
            3,
            "",
            "\x1B[38;2;249;38;114m(\x1B[0m\x1B[38;2;248;248;242mjust\nanother\ntest\x1B[38;2;249;38;114m)\x1B[0m",
        ),
        (
            "osc8_wrap",
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\ สวัสดีสวัสดี\x1b]8;;\x1b\\",
            8,
            "",
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\\nสวัสดีสวัสดี\x1b]8;;\x1b\\",
        ),
    ];
    for (name, input, limit, bp, expected) in cases {
        assert_eq!(wordwrap(input, *limit, bp), *expected, "{name}");
    }
}

#[test]
fn wrap_wordwrap() {
    let input = "the quick brown foxxxxxxxxxxxxxxxx jumped over the lazy dog.";
    assert_eq!(wrap(input, 16, ""), "the quick brown\nfoxxxxxxxxxxxxxx\nxx jumped over\nthe lazy dog.");
}

const JAPANESE_WRAP: &str = "耐許ヱヨカハ調出あゆ監件び理別よン國給災レホチ権輝モエフ会割もフ響3現エツ文時しだびほ経機ムイメフ敗文ヨク現義なさド請情ゆじょて憶主管州けでふく。排ゃわつげ美刊ヱミ出見ツ南者オ抜豆ハトロネ論索モネニイ任償スヲ話破リヤヨ秒止口イセソス止央のさ食周健でてつだ官送ト読聴遊容ひるべ。際ぐドらづ市居ネムヤ研校35岩6繹ごわク報拐イ革深52球ゃレスご究東スラ衝3間ラ録占たス。\n禁にンご忘康ざほぎル騰般ねど事超スんいう真表何カモ自浩ヲシミ図客線るふ静王ぱーま写村月掛焼詐面ぞゃ。昇強ごントほ価保キ族85岡モテ恋困ひりこな刊並せご出来ぼぎむう点目ヲウ止環公ニレ事応タス必書タメムノ当84無信升ちひょ。価ーぐ中客テサ告覧ヨトハ極整ラ得95稿はかラせ江利ス宏丸霊ミ考整ス静将ず業巨職ノラホ収嗅ざな。";

#[test]
fn wrap_breaks_words_and_lines() {
    let cases: &[(&str, &str, &str, i32)] = &[
        (
            "simple",
            "I really \x1B[38;2;249;38;114mlove\x1B[0m Go!",
            "I really\n\x1B[38;2;249;38;114mlove\x1B[0m Go!",
            8,
        ),
        ("passthrough", "hello world", "hello world", 11),
        ("asian", "こんにち", "こんに\nち", 7),
        ("emoji", "😃👰🏻‍♀️🫧", "😃\n👰🏻‍♀️\n🫧", 2),
        (
            "long style",
            "\x1B[38;2;249;38;114ma really long string\x1B[0m",
            "\x1B[38;2;249;38;114ma really\nlong\nstring\x1B[0m",
            10,
        ),
        (
            "long style nbsp",
            "\x1B[38;2;249;38;114ma really\u{a0}long string\x1B[0m",
            "\x1b[38;2;249;38;114ma\nreally\u{a0}lon\ng string\x1b[0m",
            10,
        ),
        (
            "longer",
            "the quick brown foxxxxxxxxxxxxxxxx jumped over the lazy dog.",
            "the quick brown\nfoxxxxxxxxxxxxxx\nxx jumped over\nthe lazy dog.",
            16,
        ),
        (
            "longer asian",
            "猴 猴 猴猴 猴猴猴猴猴猴猴猴猴 猴猴猴 猴猴 猴’ 猴猴 猴.",
            "猴 猴 猴猴\n猴猴猴猴猴猴猴猴\n猴 猴猴猴 猴猴\n猴’ 猴猴 猴.",
            16,
        ),
        (
            "long input",
            "Rotated keys for a-good-offensive-cheat-code-incorporated/animal-like-law-on-the-rocks.",
            "Rotated keys for a-good-offensive-cheat-code-incorporated/animal-like-law-\non-the-rocks.",
            76,
        ),
        (
            "long input2",
            "Rotated keys for a-good-offensive-cheat-code-incorporated/crypto-line-operating-system.",
            "Rotated keys for a-good-offensive-cheat-code-incorporated/crypto-line-\noperating-system.",
            76,
        ),
        ("hyphen breakpoint", "a-good-offensive-cheat-code", "a-good-\noffensive-\ncheat-code", 10),
        ("exact", "\x1b[91mfoo\x1b[0", "\x1b[91mfoo\x1b[0", 3),
        ("extra space", "foo ", "foo", 3),
        ("extra space style", "\x1b[mfoo \x1b[m", "\x1b[mfoo\x1b[m", 3),
        (
            "paragraph with styles",
            "Lorem ipsum dolor \x1b[1msit\x1b[m amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \x1b[31mUt enim\x1b[m ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea \x1b[38;5;200mcommodo consequat\x1b[m. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. \x1b[1;2;33mExcepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.\x1b[m",
            "Lorem ipsum dolor \x1b[1msit\x1b[m amet,\nconsectetur adipiscing elit,\nsed do eiusmod tempor\nincididunt ut labore et dolore\nmagna aliqua. \x1b[31mUt enim\x1b[m ad minim\nveniam, quis nostrud\nexercitation ullamco laboris\nnisi ut aliquip ex ea \x1b[38;5;200mcommodo\nconsequat\x1b[m. Duis aute irure\ndolor in reprehenderit in\nvoluptate velit esse cillum\ndolore eu fugiat nulla\npariatur. \x1b[1;2;33mExcepteur sint\noccaecat cupidatat non\nproident, sunt in culpa qui\nofficia deserunt mollit anim\nid est laborum.\x1b[m",
            30,
        ),
        (
            "Multi Byte spaces",
            "A\u{202f}B\u{202f}C\u{202f}DA\u{205f}\u{205f}B\u{205f}C\u{205f}DA\u{3000}B\u{3000}C\u{3000}D",
            "A\u{202f}B\u{202f}C\nDA\u{205f}\u{205f}B\u{205f}C\nDA\u{3000}B\nC\u{3000}D",
            7,
        ),
        ("hyphen break", "foo-bar", "foo-\nbar", 5),
        ("double space", "f  bar foobaz", "f  bar\nfoobaz", 6),
        ("passthrough", "foobar\n ", "foobar\n ", 0),
        ("pass", "foo", "foo", 3),
        ("toolong", "foobarfoo", "foob\narfo\no", 4),
        ("white space", "foo bar foo", "foo\nbar\nfoo", 4),
        ("broken_at_spaces", "foo bars foobars", "foo\nbars\nfoob\nars", 4),
        ("hyphen", "foob-foobar", "foob\n-foo\nbar", 4),
        ("wide_emoji_breakpoint", "foo🫧 foobar", "foo\n🫧\nfoob\nar", 4),
        ("space_breakpoint", "foo --bar", "foo --bar", 9),
        ("simple", "foo bars foobars", "foo\nbars\nfoob\nars", 4),
        ("limit", "foo bar", "foo\nbar", 5),
        ("remove white spaces", "foo    \nb   ar   ", "foo\nb\nar", 4),
        ("white space trail width", "foo\nb\t a\n bar", "foo\nb\t a\n bar", 4),
        ("explicit_line_break", "foo bar foo\n", "foo\nbar\nfoo\n", 4),
        ("explicit_breaks", "\nfoo bar\n\n\nfoo\n", "\nfoo\nbar\n\n\nfoo\n", 4),
        (
            "example",
            " This is a list: \n\n\t* foo\n\t* bar\n\n\n\t* foo  \nbar    ",
            " This\nis a\nlist: \n\n\t* foo\n\t* bar\n\n\n\t* foo\nbar",
            6,
        ),
        ("style_code_dont_affect_length", STYLE_FOOBAR, STYLE_FOOBAR, 7),
        (
            "style_code_dont_get_wrapped",
            WRAP_INPUT_STYLE,
            "\x1b[38;2;249;38;114m(\x1b[0m\x1b[38;2;248;248;242mjust\nanother\ntest\x1b[38;2;249;38;114m)\x1b[0m",
            7,
        ),
        (
            "osc8_wrap",
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\ สวัสดีสวัสดี\x1b]8;;\x1b\\",
            "สวัสดีสวัสดี\x1b]8;;https://example.com\x1b\\\nสวัสดีสวัสดี\x1b]8;;\x1b\\",
            8,
        ),
        ("tab", "foo\tbar", "foo\nbar", 3),
        ("Narrow NBSP", "0\u{202f}1\u{202f}2\u{202f}3\u{202f}4", "0\u{202f}1\u{202f}2\u{202f}3\n4", 7),
        ("Paragraph Separator", "0\u{2029}1\u{2029}2\u{2029}3\u{2029}4", "0\u{2029}1\u{2029}2\u{2029}3\u{2029}4", 7),
        ("Medium Mathematical Space", "0\u{205f}1\u{205f}2\u{205f}3\u{205f}4", "0\u{205f}1\u{205f}2\u{205f}3\n4", 7),
        ("Ideagraphic space", "0\u{3000}1\u{3000}2\u{3000}3\u{3000}", "0\u{3000}1\u{3000}2\n3\u{3000}", 7),
    ];
    for (name, input, expected, width) in cases {
        assert_eq!(wrap(input, *width, ""), *expected, "{name}");
    }
}

#[test]
fn wrap_japanese_narrow_and_wide() {
    let narrow = "耐許ヱヨカハ\n調出あゆ監件\nび理別よン國\n給災レホチ権\n輝モエフ会割\nもフ響3現エツ\n文時しだびほ\n経機ムイメフ\n敗文ヨク現義\nなさド請情ゆ\nじょて憶主管\n州けでふく。\n排ゃわつげ美\n刊ヱミ出見ツ\n南者オ抜豆ハ\nトロネ論索モ\nネニイ任償ス\nヲ話破リヤヨ\n秒止口イセソ\nス止央のさ食\n周健でてつだ\n官送ト読聴遊\n容ひるべ。際\nぐドらづ市居\nネムヤ研校35\n岩6繹ごわク報\n拐イ革深52球\nゃレスご究東\nスラ衝3間ラ録\n占たス。\n禁にンご忘康\nざほぎル騰般\nねど事超スん\nいう真表何カ\nモ自浩ヲシミ\n図客線るふ静\n王ぱーま写村\n月掛焼詐面ぞ\nゃ。昇強ごン\nトほ価保キ族8\n5岡モテ恋困ひ\nりこな刊並せ\nご出来ぼぎむ\nう点目ヲウ止\n環公ニレ事応\nタス必書タメ\nムノ当84無信\n升ちひょ。価\nーぐ中客テサ\n告覧ヨトハ極\n整ラ得95稿は\nかラせ江利ス\n宏丸霊ミ考整\nス静将ず業巨\n職ノラホ収嗅\nざな。";
    assert_eq!(wrap(JAPANESE_WRAP, 13, ""), narrow);
    let wide = "耐許ヱヨカハ調出あゆ監件び理別\nよン國給災レホチ権輝モエフ会割\nもフ響3現エツ文時しだびほ経機\nムイメフ敗文ヨク現義なさド請情\nゆじょて憶主管州けでふく。排ゃ\nわつげ美刊ヱミ出見ツ南者オ抜豆\nハトロネ論索モネニイ任償スヲ話\n破リヤヨ秒止口イセソス止央のさ\n食周健でてつだ官送ト読聴遊容ひ\nるべ。際ぐドらづ市居ネムヤ研校\n35岩6繹ごわク報拐イ革深52球ゃ\nレスご究東スラ衝3間ラ録占たス\n。\n禁にンご忘康ざほぎル騰般ねど事\n超スんいう真表何カモ自浩ヲシミ\n図客線るふ静王ぱーま写村月掛焼\n詐面ぞゃ。昇強ごントほ価保キ族\n85岡モテ恋困ひりこな刊並せご出\n来ぼぎむう点目ヲウ止環公ニレ事\n応タス必書タメムノ当84無信升ち\nひょ。価ーぐ中客テサ告覧ヨトハ\n極整ラ得95稿はかラせ江利ス宏丸\n霊ミ考整ス静将ず業巨職ノラホ収\n嗅ざな。";
    assert_eq!(wrap(JAPANESE_WRAP, 30, ""), wide);
}

#[test]
fn rune_widths_as_go_runewidth() {
    assert_eq!(rune_width('a'), 1);
    assert_eq!(rune_width('你'), 2);
    assert_eq!(rune_width('\u{300}'), 0);
    assert_eq!(rune_width('★'), 1);
    assert_eq!(rune_width('\t'), 0);
}

#[test]
fn tokens_split_text_and_sequences() {
    let t = tokens("a\x1b[1;31mb\x1b]8;p;u\x07c\nd");
    assert!(matches!(t[0], Token::Text("a", 1)));
    assert!(matches!(&t[1], Token::Sgr(p) if p.len() == 2));
    assert!(matches!(t[2], Token::Text("b", 1)));
    assert!(matches!(&t[3], Token::Link(_, p, u) if p == "p" && u == "u"));
    assert!(matches!(t[4], Token::Text("c", 1)));
    assert!(matches!(t[5], Token::Control("\n")));
}
