//! The default delegate: an item drawn as its title and, under it, its
//! description; the selected one with a border on its left, matches of the
//! filter underlined.

use super::{FilterApplied, Filtering, Item, ItemDelegate, Model};
use crate::ansi;
use crate::bubbles::key::Binding;
use crate::style::{Style, color, normal_border, style_runes};
use crate::tea::{Cmd, Msg};

use super::style::ELLIPSIS;

/// DefaultItemStyles is how the default delegate draws items.
#[derive(Clone, Copy, Debug)]
pub struct DefaultItemStyles {
    /// The normal state.
    pub normal_title: Style,
    /// Normal desc.
    pub normal_desc: Style,
    /// The selected item state.
    pub selected_title: Style,
    /// Selected desc.
    pub selected_desc: Style,
    /// The dimmed state, for when the filter input is initially activated.
    pub dimmed_title: Style,
    /// Dimmed desc.
    pub dimmed_desc: Style,
    /// Characters matching the current filter, if any.
    pub filter_match: Style,
}

/// new_default_item_styles is Bubbles' item styles for a dark background,
/// or a light one.
pub fn new_default_item_styles(is_dark: bool) -> DefaultItemStyles {
    let ld = |light: &str, dark: &str| if is_dark { color(dark) } else { color(light) };
    let normal_title = Style::new().foreground(ld("#1a1a1a", "#dddddd")).padding(&[0, 0, 0, 2]);
    let normal_desc = normal_title.foreground(ld("#A49FA5", "#777777"));
    let selected_title = Style::new()
        .border(normal_border(), &[false, false, false, true])
        .border_foreground(&[ld("#F793FF", "#AD58B4")])
        .foreground(ld("#EE6FF8", "#EE6FF8"))
        .padding(&[0, 0, 0, 1]);
    let selected_desc = selected_title.foreground(ld("#F793FF", "#AD58B4"));
    let dimmed_title = Style::new().foreground(ld("#A49FA5", "#777777")).padding(&[0, 0, 0, 2]);
    let dimmed_desc = dimmed_title.foreground(ld("#C2B8C2", "#4D4D4D"));
    DefaultItemStyles {
        normal_title,
        normal_desc,
        selected_title,
        selected_desc,
        dimmed_title,
        dimmed_desc,
        filter_match: Style::new().underline(true),
    }
}

/// DefaultDelegate draws items as title and description.
#[derive(Clone, Debug)]
pub struct DefaultDelegate {
    /// Show description.
    pub show_description: bool,
    /// Styles.
    pub styles: DefaultItemStyles,
    /// Update func.
    pub update_func: Option<fn(&Msg, &mut Model) -> Cmd>,
    /// Short help func.
    pub short_help_func: Option<fn() -> Vec<Binding>>,
    /// Full help func.
    pub full_help_func: Option<fn() -> Vec<Vec<Binding>>>,
    height: i32,
    spacing: i32,
}

/// new_default_delegate is the delegate as Bubbles makes it: two lines an
/// item, a line between items.
pub fn new_default_delegate() -> DefaultDelegate {
    DefaultDelegate {
        show_description: true,
        styles: new_default_item_styles(true),
        update_func: None,
        short_help_func: None,
        full_help_func: None,
        height: 2,
        spacing: 1,
    }
}

impl DefaultDelegate {
    /// set_height sets how many lines an item takes (with its description).
    pub fn set_height(&mut self, i: i32) {
        self.height = i;
    }

    /// set_spacing sets how many lines go between items.
    pub fn set_spacing(&mut self, i: i32) {
        self.spacing = i;
    }
}

impl ItemDelegate for DefaultDelegate {
    fn height(&self) -> i32 {
        if self.show_description { self.height } else { 1 }
    }

    fn spacing(&self) -> i32 {
        self.spacing
    }

    fn update(&self, msg: &Msg, m: &mut Model) -> Cmd {
        match self.update_func {
            Some(f) => f(msg, m),
            None => None,
        }
    }

    fn render(&self, w: &mut String, m: &Model, index: i32, item: &dyn Item) {
        let s = &self.styles;
        let Some(i) = item.as_default_item() else { return };
        let mut title = i.title();
        let mut desc = i.description();
        if m.width() <= 0 {
            // short-circuit
            return;
        }
        // Prevent text from exceeding list width
        let textwidth = m.width() - s.normal_title.get_padding_left() - s.normal_title.get_padding_right();
        title = ansi::truncate(&title, textwidth, ELLIPSIS);
        if self.show_description {
            let mut lines = Vec::new();
            for (i, line) in desc.split('\n').enumerate() {
                if i as i32 >= self.height - 1 {
                    break;
                }
                lines.push(ansi::truncate(line, textwidth, ELLIPSIS));
            }
            desc = lines.join("\n");
        }
        // Conditions
        let is_selected = index == m.index();
        let empty_filter = m.filter_state() == Filtering && m.filter_value().is_empty();
        let is_filtered = m.filter_state() == Filtering || m.filter_state() == FilterApplied;
        let mut matched_runes = Vec::new();
        if is_filtered && (index as usize) < m.filtered_items.as_ref().map_or(0, Vec::len) {
            // Get indices of matched characters
            matched_runes = m.matches_for_item(index);
        }
        if empty_filter {
            title = s.dimmed_title.render(&title);
            desc = s.dimmed_desc.render(&desc);
        } else if is_selected && m.filter_state() != Filtering {
            if is_filtered {
                // Highlight matches
                let unmatched = s.selected_title.inline(true);
                let matched = unmatched.inherit(s.filter_match);
                title = style_runes(&title, &matched_runes, matched, unmatched);
            }
            title = s.selected_title.render(&title);
            desc = s.selected_desc.render(&desc);
        } else {
            if is_filtered {
                // Highlight matches
                let unmatched = s.normal_title.inline(true);
                let matched = unmatched.inherit(s.filter_match);
                title = style_runes(&title, &matched_runes, matched, unmatched);
            }
            title = s.normal_title.render(&title);
            desc = s.normal_desc.render(&desc);
        }
        if self.show_description {
            w.push_str(&format!("{title}\n{desc}"));
            return;
        }
        w.push_str(&title);
    }

    fn short_help(&self) -> Option<Vec<Binding>> {
        Some(self.short_help_func.map(|f| f()).unwrap_or_default())
    }

    fn full_help(&self) -> Option<Vec<Vec<Binding>>> {
        Some(self.full_help_func.map(|f| f()).unwrap_or_default())
    }
}
