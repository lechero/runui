//! The list's styles.

use crate::bubbles::textinput;
use crate::style::{Style, color};

/// BULLET marks a page in the pagination.
pub const BULLET: &str = "•";
/// ELLIPSIS ends what's cut.
pub const ELLIPSIS: &str = "…";

/// Styles is how the list looks.
#[derive(Clone, Copy, Debug)]
pub struct Styles {
    /// Title bar.
    pub title_bar: Style,
    /// Title.
    pub title: Style,
    /// Spinner.
    pub spinner: Style,
    /// Filter.
    pub filter: textinput::Styles,
    /// Default filter character match.
    pub default_filter_character_match: Style,
    /// Status bar.
    pub status_bar: Style,
    /// Status empty.
    pub status_empty: Style,
    /// Status bar active filter.
    pub status_bar_active_filter: Style,
    /// Status bar filter count.
    pub status_bar_filter_count: Style,
    /// No items.
    pub no_items: Style,
    /// Pagination style.
    pub pagination_style: Style,
    /// Help style.
    pub help_style: Style,
    /// Active pagination dot.
    pub active_pagination_dot: Style,
    /// Inactive pagination dot.
    pub inactive_pagination_dot: Style,
    /// Arabic pagination.
    pub arabic_pagination: Style,
    /// Divider dot.
    pub divider_dot: Style,
}

/// default_styles is Bubbles' list styles for a dark background, or a
/// light one.
pub fn default_styles(is_dark: bool) -> Styles {
    let ld = |light: &str, dark: &str| if is_dark { color(dark) } else { color(light) };
    let very_subdued_color = ld("#DDDADA", "#3C3C3C");
    let subdued_color = ld("#9B9B9B", "#5C5C5C");
    let prompt = Style::new().foreground(ld("#04B575", "#ECFD65"));
    let mut filter = textinput::default_styles(is_dark);
    filter.cursor.color = ld("#EE6FF8", "#EE6FF8");
    filter.blurred.prompt = prompt;
    filter.focused.prompt = prompt;
    Styles {
        title_bar: Style::new().padding(&[0, 0, 1, 2]),
        title: Style::new().background(color("62")).foreground(color("230")).padding(&[0, 1]),
        spinner: Style::new().foreground(ld("#8E8E8E", "#747373")),
        filter,
        default_filter_character_match: Style::new().underline(true),
        status_bar: Style::new().foreground(ld("#A49FA5", "#777777")).padding(&[0, 0, 1, 2]),
        status_empty: Style::new().foreground(subdued_color),
        status_bar_active_filter: Style::new().foreground(ld("#1a1a1a", "#dddddd")),
        status_bar_filter_count: Style::new().foreground(very_subdued_color),
        no_items: Style::new().foreground(ld("#909090", "#626262")),
        arabic_pagination: Style::new().foreground(subdued_color),
        pagination_style: Style::new().padding_left(2),
        help_style: Style::new().padding(&[1, 0, 0, 2]),
        active_pagination_dot: Style::new().foreground(ld("#847A85", "#979797")).set_string(BULLET),
        inactive_pagination_dot: Style::new().foreground(very_subdued_color).set_string(BULLET),
        divider_dot: Style::new().foreground(very_subdued_color).set_string(" • "),
    }
}
