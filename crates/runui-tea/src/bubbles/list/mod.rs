//! A list of items (Bubbles' `list`): pages of items drawn by a delegate,
//! a cursor, filtering as you type with `/` (fuzzy, as sahilm/fuzzy
//! matches), a status bar counting the items, pagination dots and help.
//!
//! Items are trait objects; the interface gets its own back by
//! downcasting:
//!
//! ```
//! use runui_tea::bubbles::list::{self, DefaultItem, Item};
//!
//! struct Pkg(String);
//! impl Item for Pkg {
//!     fn filter_value(&self) -> String { self.0.clone() }
//!     fn as_default_item(&self) -> Option<&dyn DefaultItem> { Some(self) }
//! }
//! impl DefaultItem for Pkg {
//!     fn title(&self) -> String { self.0.clone() }
//!     fn description(&self) -> String { String::new() }
//! }
//!
//! let mut l = list::Model::new(vec![Box::new(Pkg("site.all".into()))], list::new_default_delegate(), 40, 10);
//! assert_eq!(l.items()[0].downcast_ref::<Pkg>().unwrap().0, "site.all");
//! ```

mod defaultitem;
pub mod fuzzy;
mod keys;
mod style;

use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use defaultitem::{DefaultDelegate, DefaultItemStyles, new_default_delegate, new_default_item_styles};
pub use keys::{KeyMap, default_key_map};
pub use style::{BULLET, ELLIPSIS, Styles, default_styles};

use super::help::{self, KeyMap as _};
use super::key::{self, Binding};
use super::paginator;
use super::spinner;
use super::textinput;
use crate::ansi;
use crate::style as lipgloss;
use crate::tea::{self, Cmd, KeyPressMsg, Msg};

/// Item is an item in the list.
pub trait Item: Any {
    /// filter_value is what filtering matches against.
    fn filter_value(&self) -> String;
    /// as_default_item is the item as a [`DefaultItem`], for the default
    /// delegate, when it is one (Go's type assertion).
    fn as_default_item(&self) -> Option<&dyn DefaultItem> {
        None
    }
}

impl dyn Item {
    /// downcast_ref is the item as its own type, if it is one.
    pub fn downcast_ref<T: Item>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    /// is says whether the item is of type T.
    pub fn is<T: Item>(&self) -> bool {
        (self as &dyn Any).is::<T>()
    }
}

/// DefaultItem is an item the default delegate draws: a title, and a
/// description under it.
pub trait DefaultItem: Item {
    /// title is the item's first line.
    fn title(&self) -> String;
    /// description is the item's second line.
    fn description(&self) -> String;
}

/// ItemDelegate draws the items and can take messages for them.
pub trait ItemDelegate {
    /// render draws the item at `index` (of the items shown).
    fn render(&self, w: &mut String, m: &Model, index: i32, item: &dyn Item);
    /// height is how many lines an item takes.
    fn height(&self) -> i32;
    /// spacing is how many lines go between items.
    fn spacing(&self) -> i32;
    /// update takes a message the list didn't handle.
    fn update(&self, msg: &Msg, m: &mut Model) -> Cmd;
    /// short_help is the delegate's keys for the short help, when it has
    /// help (Go's `help.KeyMap` check).
    fn short_help(&self) -> Option<Vec<Binding>> {
        None
    }
    /// full_help is the delegate's keys for the full help, when it has help.
    fn full_help(&self) -> Option<Vec<Vec<Binding>>> {
        None
    }
}

/// A delegate that draws nothing: what stands in while the list's own
/// delegate is out taking a message.
struct NoDelegate;

impl ItemDelegate for NoDelegate {
    fn render(&self, _: &mut String, _: &Model, _: i32, _: &dyn Item) {}
    fn height(&self) -> i32 {
        1
    }
    fn spacing(&self) -> i32 {
        0
    }
    fn update(&self, _: &Msg, _: &mut Model) -> Cmd {
        None
    }
}

/// An item that passed the filter: where it is in the list and which of its
/// characters matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilteredItem {
    /// index is the item's place in the unfiltered list.
    pub index: i32,
    /// matches is where the filter matched in its filter value.
    pub matches: Vec<i32>,
}

/// FilterMatchesMsg is the items that passed the filter, as the filter
/// command finds them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterMatchesMsg(pub Vec<FilteredItem>);

/// Rank is an item the filter matched: its index and the characters
/// matched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rank {
    /// Index.
    pub index: i32,
    /// Matched indexes.
    pub matched_indexes: Vec<i32>,
}

/// FilterFunc filters items: the term and the items' filter values in, the
/// matches out.
pub type FilterFunc = fn(&str, &[String]) -> Vec<Rank>;

/// default_filter matches fuzzily, best first.
pub fn default_filter(term: &str, targets: &[String]) -> Vec<Rank> {
    fuzzy::find(term, targets)
        .into_iter()
        .map(|r| Rank { index: r.index, matched_indexes: r.matched_indexes })
        .collect()
}

/// unsorted_filter matches fuzzily, in the items' order.
pub fn unsorted_filter(term: &str, targets: &[String]) -> Vec<Rank> {
    fuzzy::find_no_sort(term, targets)
        .into_iter()
        .map(|r| Rank { index: r.index, matched_indexes: r.matched_indexes })
        .collect()
}

/// The status message's timing out, for the message shown when it was set.
struct StatusMessageTimeoutMsg(u64);

static STATUS_GEN: AtomicU64 = AtomicU64::new(0);

/// FilterState is where filtering is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FilterState {
    /// No filter set.
    #[default]
    Unfiltered,
    /// The filter is being typed.
    Filtering,
    /// A filter is applied and not being typed.
    FilterApplied,
}

pub use FilterState::{FilterApplied, Filtering, Unfiltered};

impl FilterState {
    /// string is the state's name.
    pub fn string(&self) -> &'static str {
        match self {
            Unfiltered => "unfiltered",
            Filtering => "filtering",
            FilterApplied => "filter applied",
        }
    }
}

/// Model is the list.
pub struct Model {
    show_title: bool,
    show_filter: bool,
    show_status_bar: bool,
    show_pagination: bool,
    show_help: bool,
    filtering_enabled: bool,
    item_name_singular: String,
    item_name_plural: String,
    /// Title.
    pub title: String,
    /// Styles.
    pub styles: Styles,
    /// Infinite scrolling.
    pub infinite_scrolling: bool,
    /// Key map.
    pub key_map: KeyMap,
    /// Filter.
    pub filter: FilterFunc,
    disable_quit_keybindings: bool,
    /// Additional short help keys.
    pub additional_short_help_keys: Option<fn() -> Vec<Binding>>,
    /// Additional full help keys.
    pub additional_full_help_keys: Option<fn() -> Vec<Binding>>,
    spinner: spinner::Model,
    show_spinner: bool,
    width: i32,
    height: i32,
    /// Paginator.
    pub paginator: paginator::Model,
    cursor: i32,
    /// Help.
    pub help: help::Model,
    /// Filter input.
    pub filter_input: textinput::Model,
    filter_state: FilterState,
    /// Status message lifetime.
    pub status_message_lifetime: Duration,
    status_message: String,
    status_gen: u64,
    items: Vec<Box<dyn Item>>,
    filtered_items: Option<Vec<FilteredItem>>,
    delegate: Box<dyn ItemDelegate>,
}

fn clamp(v: i32, low: i32, high: i32) -> i32 {
    let (low, high) = if low > high { (high, low) } else { (low, high) };
    v.max(low).min(high)
}

/// new is Go's `list.New`.
pub fn new<D: ItemDelegate + 'static>(items: Vec<Box<dyn Item>>, delegate: D, width: i32, height: i32) -> Model {
    Model::new(items, delegate, width, height)
}

impl Model {
    /// new is a list of `items` drawn by `delegate`, `width` by `height`.
    pub fn new<D: ItemDelegate + 'static>(items: Vec<Box<dyn Item>>, delegate: D, width: i32, height: i32) -> Model {
        let styles = default_styles(true);
        let mut sp = spinner::Model::new(vec![]);
        sp.spinner = spinner::line();
        sp.style = styles.spinner;
        let mut filter_input = textinput::Model::new();
        filter_input.prompt = "Filter: ".into();
        filter_input.char_limit = 64;
        filter_input.focus();
        let mut p = paginator::Model::new();
        p.type_ = paginator::Type::Dots;
        p.active_dot = styles.active_pagination_dot.string();
        p.inactive_dot = styles.inactive_pagination_dot.string();
        let mut m = Model {
            show_title: true,
            show_filter: true,
            show_status_bar: true,
            show_pagination: true,
            show_help: true,
            item_name_singular: "item".into(),
            item_name_plural: "items".into(),
            filtering_enabled: true,
            key_map: default_key_map(),
            filter: default_filter,
            styles,
            title: "List".into(),
            filter_input,
            status_message_lifetime: Duration::from_secs(1),
            width,
            height,
            delegate: Box::new(delegate),
            items,
            paginator: p,
            spinner: sp,
            help: help::Model::new(),
            infinite_scrolling: false,
            disable_quit_keybindings: false,
            additional_short_help_keys: None,
            additional_full_help_keys: None,
            show_spinner: false,
            cursor: 0,
            filter_state: Unfiltered,
            status_message: String::new(),
            status_gen: 0,
            filtered_items: None,
        };
        m.update_pagination();
        m.update_keybindings();
        m
    }

    /// set_filtering_enabled allows filtering, or not.
    pub fn set_filtering_enabled(&mut self, v: bool) {
        self.filtering_enabled = v;
        if !v {
            self.reset_filtering();
        }
        self.update_keybindings();
    }

    /// filtering_enabled says whether filtering is allowed.
    pub fn filtering_enabled(&self) -> bool {
        self.filtering_enabled
    }

    /// set_show_title shows the title, or not.
    pub fn set_show_title(&mut self, v: bool) {
        self.show_title = v;
        self.update_pagination();
    }

    /// set_filter_text applies a filter, as if typed and accepted.
    pub fn set_filter_text(&mut self, filter: &str) {
        self.filter_state = Filtering;
        self.filter_input.set_value(filter);
        let msg = self.filter_items_now();
        self.filtered_items = Some(msg.0);
        self.filter_state = FilterApplied;
        self.go_to_start();
        self.filter_input.cursor_end();
        self.update_pagination();
        self.update_keybindings();
    }

    /// set_filter_state sets where filtering is.
    pub fn set_filter_state(&mut self, state: FilterState) {
        self.go_to_start();
        self.filter_state = state;
        self.filter_input.cursor_end();
        self.filter_input.focus();
        self.update_keybindings();
    }

    /// show_title says whether the title shows.
    pub fn show_title(&self) -> bool {
        self.show_title
    }

    /// set_show_filter shows the filter, or not.
    pub fn set_show_filter(&mut self, v: bool) {
        self.show_filter = v;
        self.update_pagination();
    }

    /// show_filter says whether the filter shows.
    pub fn show_filter(&self) -> bool {
        self.show_filter
    }

    /// set_show_status_bar shows the status bar, or not.
    pub fn set_show_status_bar(&mut self, v: bool) {
        self.show_status_bar = v;
        self.update_pagination();
    }

    /// show_status_bar says whether the status bar shows.
    pub fn show_status_bar(&self) -> bool {
        self.show_status_bar
    }

    /// set_status_bar_item_name sets what the items are called in the
    /// status bar, one and many.
    pub fn set_status_bar_item_name(&mut self, singular: &str, plural: &str) {
        self.item_name_singular = singular.into();
        self.item_name_plural = plural.into();
    }

    /// status_bar_item_name is what the items are called, one and many.
    pub fn status_bar_item_name(&self) -> (String, String) {
        (self.item_name_singular.clone(), self.item_name_plural.clone())
    }

    /// set_show_pagination shows the pagination, or not.
    pub fn set_show_pagination(&mut self, v: bool) {
        self.show_pagination = v;
        self.update_pagination();
    }

    /// show_pagination says whether the pagination shows.
    pub fn show_pagination(&self) -> bool {
        self.show_pagination
    }

    /// set_show_help shows the help, or not.
    pub fn set_show_help(&mut self, v: bool) {
        self.show_help = v;
        self.update_pagination();
    }

    /// show_help says whether the help shows.
    pub fn show_help(&self) -> bool {
        self.show_help
    }

    /// items is every item.
    pub fn items(&self) -> &[Box<dyn Item>] {
        &self.items
    }

    /// set_items replaces the items: when filtered, the filter runs again
    /// (the command it returns).
    pub fn set_items(&mut self, items: Vec<Box<dyn Item>>) -> Cmd {
        let mut cmd = None;
        self.items = items;
        if self.filter_state != Unfiltered {
            self.filtered_items = None;
            cmd = self.filter_items();
        }
        self.update_pagination();
        self.update_keybindings();
        cmd
    }

    /// select moves the cursor to an item shown.
    pub fn select(&mut self, index: i32) {
        self.paginator.page = index / self.paginator.per_page;
        self.cursor = index % self.paginator.per_page;
    }

    /// reset_selected moves the cursor to the first item.
    pub fn reset_selected(&mut self) {
        self.select(0);
    }

    /// reset_filter clears the filter.
    pub fn reset_filter(&mut self) {
        self.reset_filtering();
    }

    /// set_item replaces an item.
    pub fn set_item(&mut self, index: i32, item: Box<dyn Item>) -> Cmd {
        let mut cmd = None;
        if let Some(slot) = self.items.get_mut(index as usize) {
            *slot = item;
        }
        if self.filter_state != Unfiltered {
            cmd = self.filter_items();
        }
        self.update_pagination();
        cmd
    }

    /// insert_item puts an item in at `index` (at the end when past it).
    pub fn insert_item(&mut self, index: i32, item: Box<dyn Item>) -> Cmd {
        let mut cmd = None;
        let at = (index.max(0) as usize).min(self.items.len());
        self.items.insert(at, item);
        if self.filter_state != Unfiltered {
            cmd = self.filter_items();
        }
        self.update_pagination();
        self.update_keybindings();
        cmd
    }

    /// remove_item takes an item out.
    pub fn remove_item(&mut self, index: i32) {
        if (index as usize) < self.items.len() {
            self.items.remove(index as usize);
        }
        if self.filter_state != Unfiltered {
            if let Some(f) = self.filtered_items.as_mut()
                && (index as usize) < f.len()
            {
                f.remove(index as usize);
            }
            if self.filtered_items.as_ref().is_none_or(Vec::is_empty) {
                self.reset_filtering();
            }
        }
        self.update_pagination();
    }

    /// set_delegate replaces the delegate.
    pub fn set_delegate<D: ItemDelegate + 'static>(&mut self, d: D) {
        self.delegate = Box::new(d);
        self.update_pagination();
    }

    fn filtered(&self) -> Vec<&dyn Item> {
        match &self.filtered_items {
            Some(f) => f.iter().filter_map(|fi| self.items.get(fi.index as usize).map(|b| b.as_ref())).collect(),
            None => Vec::new(),
        }
    }

    /// visible_items is the items shown: those passing the filter, when
    /// there's one.
    pub fn visible_items(&self) -> Vec<&dyn Item> {
        if self.filter_state != Unfiltered {
            return self.filtered();
        }
        self.items.iter().map(|b| b.as_ref()).collect()
    }

    fn visible_len(&self) -> i32 {
        if self.filter_state != Unfiltered {
            return self.filtered().len() as i32;
        }
        self.items.len() as i32
    }

    /// selected_item is the item the cursor is on, if any.
    pub fn selected_item(&self) -> Option<&dyn Item> {
        let i = self.index();
        let items = self.visible_items();
        if i < 0 || items.is_empty() || items.len() as i32 <= i {
            return None;
        }
        Some(items[i as usize])
    }

    /// matches_for_item is which characters of the item at `index` (of
    /// those shown) the filter matched.
    pub fn matches_for_item(&self, index: i32) -> Vec<i32> {
        match &self.filtered_items {
            Some(f) if (index as usize) < f.len() => f[index as usize].matches.clone(),
            _ => Vec::new(),
        }
    }

    /// index is the cursor's place among the items shown.
    pub fn index(&self) -> i32 {
        self.paginator.page * self.paginator.per_page + self.cursor
    }

    /// global_index is the cursor's item's place in the unfiltered list.
    pub fn global_index(&self) -> i32 {
        let index = self.index();
        match &self.filtered_items {
            Some(f) if (index as usize) < f.len() => f[index as usize].index,
            _ => index,
        }
    }

    /// cursor is the cursor's place on its page.
    pub fn cursor(&self) -> i32 {
        self.cursor
    }

    /// cursor_up moves the cursor up an item.
    pub fn cursor_up(&mut self) {
        self.cursor -= 1;
        // If we're at the start, stop
        if self.cursor < 0 && self.paginator.on_first_page() {
            // if infinite scrolling is enabled, go to the last item
            if self.infinite_scrolling {
                self.go_to_end();
                return;
            }
            self.cursor = 0;
            return;
        }
        // Move the cursor as normal
        if self.cursor >= 0 {
            return;
        }
        // Go to the previous page
        self.paginator.prev_page();
        self.cursor = self.max_cursor_index();
    }

    /// cursor_down moves the cursor down an item.
    pub fn cursor_down(&mut self) {
        let max_cursor_index = self.max_cursor_index();
        self.cursor += 1;
        // We're still within bounds of the current page, so no need to do
        // anything.
        if self.cursor <= max_cursor_index {
            return;
        }
        // Go to the next page
        if !self.paginator.on_last_page() {
            self.paginator.next_page();
            self.cursor = 0;
            return;
        }
        // We're on the last page, so we can't go any further.
        self.cursor = max_cursor_index.max(0);
        // if infinite scrolling is enabled, go to the first item
        if self.infinite_scrolling {
            self.go_to_start();
        }
    }

    /// go_to_start moves the cursor to the first item.
    pub fn go_to_start(&mut self) {
        self.paginator.page = 0;
        self.cursor = 0;
    }

    /// go_to_end moves the cursor to the last item.
    pub fn go_to_end(&mut self) {
        self.paginator.page = (self.paginator.total_pages - 1).max(0);
        self.cursor = self.max_cursor_index();
    }

    /// prev_page goes a page back.
    pub fn prev_page(&mut self) {
        self.paginator.prev_page();
        self.cursor = clamp(self.cursor, 0, self.max_cursor_index());
    }

    /// next_page goes a page on.
    pub fn next_page(&mut self) {
        self.paginator.next_page();
        self.cursor = clamp(self.cursor, 0, self.max_cursor_index());
    }

    fn max_cursor_index(&self) -> i32 {
        (self.paginator.items_on_page(self.visible_len()) - 1).max(0)
    }

    /// filter_state is where filtering is.
    pub fn filter_state(&self) -> FilterState {
        self.filter_state
    }

    /// filter_value is the filter typed.
    pub fn filter_value(&self) -> String {
        self.filter_input.value()
    }

    /// setting_filter says whether the filter is being typed.
    pub fn setting_filter(&self) -> bool {
        self.filter_state == Filtering
    }

    /// is_filtered says whether a filter is applied.
    pub fn is_filtered(&self) -> bool {
        self.filter_state == FilterApplied
    }

    /// width is how many cells wide the list is.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// height is how many lines the list takes.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// set_spinner sets the spinner's frames.
    pub fn set_spinner(&mut self, s: spinner::Spinner) {
        self.spinner.spinner = s;
    }

    /// toggle_spinner shows the spinner, or hides it.
    pub fn toggle_spinner(&mut self) -> Cmd {
        if !self.show_spinner {
            return self.start_spinner();
        }
        self.stop_spinner();
        None
    }

    /// start_spinner shows the spinner.
    pub fn start_spinner(&mut self) -> Cmd {
        self.show_spinner = true;
        self.spinner.tick()
    }

    /// stop_spinner hides the spinner.
    pub fn stop_spinner(&mut self) {
        self.show_spinner = false;
    }

    /// disable_quit_keybindings turns the quit keys off.
    pub fn disable_quit_keybindings(&mut self) {
        self.disable_quit_keybindings = true;
        self.key_map.quit.set_enabled(false);
        self.key_map.force_quit.set_enabled(false);
    }

    /// new_status_message shows a message in the title bar for a while.
    pub fn new_status_message(&mut self, s: &str) -> Cmd {
        self.status_message = s.into();
        let id = STATUS_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        self.status_gen = id;
        let lifetime = self.status_message_lifetime;
        tea::cmd_msg(move || {
            std::thread::sleep(lifetime);
            StatusMessageTimeoutMsg(id)
        })
    }

    /// set_width sets the width.
    pub fn set_width(&mut self, v: i32) {
        self.set_size(v, self.height);
    }

    /// set_height sets the height.
    pub fn set_height(&mut self, v: i32) {
        self.set_size(self.width, v);
    }

    /// set_size sets the width and height.
    pub fn set_size(&mut self, width: i32, height: i32) {
        let prompt_width = lipgloss::width(&self.styles.title.render(&self.filter_input.prompt));
        self.width = width;
        self.height = height;
        self.help.set_width(width);
        self.filter_input.set_width(width - prompt_width - lipgloss::width(&self.spinner_view()));
        self.update_pagination();
        self.update_keybindings();
    }

    fn reset_filtering(&mut self) {
        if self.filter_state == Unfiltered {
            return;
        }
        self.filter_state = Unfiltered;
        self.filter_input.reset();
        self.filtered_items = None;
        self.update_pagination();
        self.update_keybindings();
    }

    fn items_as_filter_items(&self) -> Vec<FilteredItem> {
        (0..self.items.len() as i32).map(|i| FilteredItem { index: i, matches: Vec::new() }).collect()
    }

    fn update_keybindings(&mut self) {
        if self.filter_state == Filtering {
            let k = &mut self.key_map;
            k.cursor_up.set_enabled(false);
            k.cursor_down.set_enabled(false);
            k.next_page.set_enabled(false);
            k.prev_page.set_enabled(false);
            k.go_to_start.set_enabled(false);
            k.go_to_end.set_enabled(false);
            k.filter.set_enabled(false);
            k.clear_filter.set_enabled(false);
            k.cancel_while_filtering.set_enabled(true);
            k.accept_while_filtering.set_enabled(!self.filter_input.value().is_empty());
            k.quit.set_enabled(false);
            k.show_full_help.set_enabled(false);
            k.close_full_help.set_enabled(false);
            return;
        }
        let has_items = !self.items.is_empty();
        let has_pages = self.paginator.total_pages > 1;
        let filtering_enabled = self.filtering_enabled;
        let applied = self.filter_state == FilterApplied;
        let quit = !self.disable_quit_keybindings;
        {
            let k = &mut self.key_map;
            k.cursor_up.set_enabled(has_items);
            k.cursor_down.set_enabled(has_items);
            k.next_page.set_enabled(has_pages);
            k.prev_page.set_enabled(has_pages);
            k.go_to_start.set_enabled(has_items);
            k.go_to_end.set_enabled(has_items);
            k.filter.set_enabled(filtering_enabled && has_items);
            k.clear_filter.set_enabled(applied);
            k.cancel_while_filtering.set_enabled(false);
            k.accept_while_filtering.set_enabled(false);
            k.quit.set_enabled(quit);
        }
        if self.help.show_all {
            self.key_map.show_full_help.set_enabled(true);
            self.key_map.close_full_help.set_enabled(true);
        } else {
            let min_help = count_enabled_bindings(&self.full_help()) > 1;
            self.key_map.show_full_help.set_enabled(min_help);
            self.key_map.close_full_help.set_enabled(min_help);
        }
    }

    /// update_pagination works out how many items fit on a page, and the
    /// page the cursor's item is on.
    fn update_pagination(&mut self) {
        let index = self.index();
        let mut avail_height = self.height;
        if self.show_title || (self.show_filter && self.filtering_enabled) {
            avail_height -= lipgloss::height(&self.title_view());
        }
        if self.show_status_bar {
            avail_height -= lipgloss::height(&self.status_view());
        }
        if self.show_pagination {
            avail_height -= lipgloss::height(&self.pagination_view());
        }
        if self.show_help {
            avail_height -= lipgloss::height(&self.help_view());
        }
        self.paginator.per_page = (avail_height / (self.delegate.height() + self.delegate.spacing())).max(1);
        let pages = self.visible_len();
        if pages < 1 {
            self.paginator.set_total_pages(1);
        } else {
            self.paginator.set_total_pages(pages);
        }
        // Restore index
        self.paginator.page = index / self.paginator.per_page;
        self.cursor = index % self.paginator.per_page;
        // Make sure the page stays in bounds
        if self.paginator.page >= self.paginator.total_pages - 1 {
            self.paginator.page = (self.paginator.total_pages - 1).max(0);
        }
    }

    fn hide_status_message(&mut self) {
        self.status_message.clear();
        self.status_gen = 0;
    }

    /// update takes a message: keys move the cursor, filter and turn pages;
    /// the filter's matches come back as a message too.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        let mut cmds = Vec::new();
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            if key::matches(k, &[&self.key_map.force_quit]) {
                return tea::quit();
            }
        } else if let Some(f) = msg.downcast_ref::<FilterMatchesMsg>() {
            self.filtered_items = Some(f.0.clone());
            return None;
        } else if msg.is::<spinner::TickMsg>() {
            let cmd = self.spinner.update(msg);
            if self.show_spinner {
                cmds.push(cmd);
            }
        } else if let Some(t) = msg.downcast_ref::<StatusMessageTimeoutMsg>()
            && t.0 == self.status_gen
        {
            self.hide_status_message();
        }
        if self.filter_state == Filtering {
            cmds.push(self.handle_filtering(msg));
        } else {
            cmds.push(self.handle_browsing(msg));
        }
        tea::batch(cmds)
    }

    fn handle_browsing(&mut self, msg: &Msg) -> Cmd {
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            let km = self.key_map.clone();
            let m = |b: &Binding| key::matches(k, &[b]);
            if m(&km.clear_filter) {
                self.reset_filtering();
            } else if m(&km.quit) {
                return tea::quit();
            } else if m(&km.cursor_up) {
                self.cursor_up();
            } else if m(&km.cursor_down) {
                self.cursor_down();
            } else if m(&km.prev_page) {
                self.paginator.prev_page();
            } else if m(&km.next_page) {
                self.paginator.next_page();
            } else if m(&km.go_to_start) {
                self.go_to_start();
            } else if m(&km.go_to_end) {
                self.go_to_end();
            } else if m(&km.filter) {
                self.hide_status_message();
                if self.filter_input.value().is_empty() {
                    // Populate filter with all items only if the filter is
                    // empty.
                    self.filtered_items = Some(self.items_as_filter_items());
                }
                self.go_to_start();
                self.filter_state = Filtering;
                self.filter_input.cursor_end();
                self.filter_input.focus();
                self.update_keybindings();
                return textinput::blink();
            } else if m(&km.show_full_help) || m(&km.close_full_help) {
                self.help.show_all = !self.help.show_all;
                self.update_pagination();
            }
        }
        let d = std::mem::replace(&mut self.delegate, Box::new(NoDelegate));
        let cmd = d.update(msg, self);
        self.delegate = d;
        self.cursor = clamp(self.cursor, 0, self.max_cursor_index());
        cmd
    }

    fn handle_filtering(&mut self, msg: &Msg) -> Cmd {
        let mut cmds = Vec::new();
        // Handle keys
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            if key::matches(k, &[&self.key_map.cancel_while_filtering]) {
                self.reset_filtering();
                self.key_map.filter.set_enabled(true);
                self.key_map.clear_filter.set_enabled(false);
            } else if key::matches(k, &[&self.key_map.accept_while_filtering]) {
                self.hide_status_message();
                if !self.items.is_empty() {
                    if self.visible_len() == 0 {
                        self.reset_filtering();
                    } else {
                        self.filter_input.blur();
                        self.filter_state = FilterApplied;
                        self.update_keybindings();
                        if self.filter_input.value().is_empty() {
                            self.reset_filtering();
                        }
                    }
                }
            }
        }
        // Update the filter text input component
        let before = self.filter_input.value();
        let mut input = self.filter_input.clone();
        let input_cmd = input.update(msg);
        let filter_changed = before != input.value();
        self.filter_input = input;
        cmds.push(input_cmd);
        // If the filtering input has changed, request updated filtering
        if filter_changed {
            cmds.push(self.filter_items());
            let v = !self.filter_input.value().is_empty();
            self.key_map.accept_while_filtering.set_enabled(v);
        }
        // Update pagination
        self.update_pagination();
        tea::batch(cmds)
    }

    /// view draws the list.
    pub fn view(&self) -> String {
        let mut sections: Vec<String> = Vec::new();
        let mut avail_height = self.height;
        if self.show_title || (self.show_filter && self.filtering_enabled) {
            let v = self.title_view();
            avail_height -= lipgloss::height(&v);
            sections.push(v);
        }
        if self.show_status_bar {
            let v = self.status_view();
            avail_height -= lipgloss::height(&v);
            sections.push(v);
        }
        let mut pagination = String::new();
        if self.show_pagination {
            pagination = self.pagination_view();
            avail_height -= lipgloss::height(&pagination);
        }
        let mut help = String::new();
        if self.show_help {
            help = self.help_view();
            avail_height -= lipgloss::height(&help);
        }
        let content = lipgloss::Style::new().height(avail_height).render(&self.populated_view());
        sections.push(content);
        if self.show_pagination {
            sections.push(pagination);
        }
        if self.show_help {
            sections.push(help);
        }
        let refs: Vec<&str> = sections.iter().map(String::as_str).collect();
        lipgloss::join_vertical(lipgloss::LEFT, &refs)
    }

    fn title_view(&self) -> String {
        let mut view = String::new();
        let mut title_bar_style = self.styles.title_bar;
        // We need to account for the size of the spinner, even if we don't
        // render it, to reserve some space for it should we turn it on
        // later.
        let spinner_view = self.spinner_view();
        let spinner_width = lipgloss::width(&spinner_view);
        let spinner_left_gap = " ";
        let spinner_on_left = title_bar_style.get_padding_left() >= spinner_width + lipgloss::width(spinner_left_gap)
            && self.show_spinner;
        // If the filter's showing, draw that. Otherwise draw the title.
        if self.show_filter && self.filter_state == Filtering {
            view += &self.filter_input.view();
        } else if self.show_title {
            if self.show_spinner && spinner_on_left {
                view += &spinner_view;
                view += spinner_left_gap;
                let gap = title_bar_style.get_padding_left();
                title_bar_style = title_bar_style.padding_left(gap - spinner_width - lipgloss::width(spinner_left_gap));
            }
            view += &self.styles.title.render(&self.title);
            // Status message
            if self.filter_state != Filtering {
                view += "  ";
                view += &self.status_message;
                view = ansi::truncate(&view, self.width - spinner_width, ELLIPSIS);
            }
        }
        // Spinner
        if self.show_spinner && !spinner_on_left {
            // Place spinner on the right
            let avail_space = self.width - lipgloss::width(&self.styles.title_bar.render(&view));
            if avail_space > spinner_width {
                view += &" ".repeat((avail_space - spinner_width) as usize);
                view += &spinner_view;
            }
        }
        if !view.is_empty() {
            return title_bar_style.render(&view);
        }
        view
    }

    fn status_view(&self) -> String {
        let mut status = String::new();
        let total_items = self.items.len() as i32;
        let visible_items = self.visible_len();
        let item_name = if visible_items != 1 { &self.item_name_plural } else { &self.item_name_singular };
        let items_display = format!("{visible_items} {item_name}");
        if self.filter_state == Filtering {
            // Filter results
            if visible_items == 0 {
                status = self.styles.status_empty.render("Nothing matched");
            } else {
                status = items_display;
            }
        } else if self.items.is_empty() {
            // Not filtering: no items.
            status = self.styles.status_empty.render(&format!("No {}", self.item_name_plural));
        } else {
            // Normal
            if self.filter_state == FilterApplied {
                let f = self.filter_input.value();
                let f = ansi::truncate(f.trim(), 10, "…");
                status += &format!("“{f}” ");
            }
            status += &items_display;
        }
        let num_filtered = total_items - visible_items;
        if num_filtered > 0 {
            status += &self.styles.divider_dot.string();
            status += &self.styles.status_bar_filter_count.render(&format!("{num_filtered} filtered"));
        }
        self.styles.status_bar.render(&status)
    }

    fn pagination_view(&self) -> String {
        if self.paginator.total_pages < 2 {
            return String::new();
        }
        let mut s = self.paginator.view();
        // If the dot pagination is wider than the width of the window use
        // the arabic paginator.
        if ansi::string_width(&s) > self.width {
            let mut p = self.paginator.clone();
            p.type_ = paginator::Type::Arabic;
            s = self.styles.arabic_pagination.render(&p.view());
        }
        let mut style = self.styles.pagination_style;
        if self.delegate.spacing() == 0 && style.get_margin_top() == 0 {
            style = style.margin_top(1);
        }
        style.render(&s)
    }

    fn populated_view(&self) -> String {
        let items = self.visible_items();
        let mut b = String::new();
        // Empty states
        if items.is_empty() {
            if self.filter_state == Filtering {
                return String::new();
            }
            return self.styles.no_items.render(&format!("No {}.", self.item_name_plural));
        }
        let (start, end) = self.paginator.get_slice_bounds(items.len() as i32);
        let docs = &items[start as usize..end as usize];
        for (i, item) in docs.iter().enumerate() {
            self.delegate.render(&mut b, self, i as i32 + start, *item);
            if i != docs.len() - 1 {
                b.push_str(&"\n".repeat((self.delegate.spacing() + 1).max(0) as usize));
            }
        }
        // If there aren't enough items to fill up this page (always the
        // last page) then we need to add some newlines to fill up the
        // space where items would have been.
        let items_on_page = self.paginator.items_on_page(items.len() as i32);
        if items_on_page < self.paginator.per_page {
            let n = (self.paginator.per_page - items_on_page) * (self.delegate.height() + self.delegate.spacing());
            b.push_str(&"\n".repeat(n.max(0) as usize));
        }
        b
    }

    fn help_view(&self) -> String {
        self.styles.help_style.render(&self.help.view(self))
    }

    fn spinner_view(&self) -> String {
        self.spinner.view()
    }

    /// filter_items is the command that filters the items, its matches
    /// coming back as a FilterMatchesMsg.
    fn filter_items(&self) -> Cmd {
        let term = self.filter_input.value();
        let unfiltered = term.is_empty() || self.filter_state == Unfiltered;
        let n = self.items.len() as i32;
        let targets: Vec<String> =
            if unfiltered { Vec::new() } else { self.items.iter().map(|i| i.filter_value()).collect() };
        let filter = self.filter;
        tea::cmd_msg(move || filter_matches(&term, unfiltered, n, &targets, filter))
    }

    fn filter_items_now(&self) -> FilterMatchesMsg {
        let term = self.filter_input.value();
        let unfiltered = term.is_empty() || self.filter_state == Unfiltered;
        let targets: Vec<String> = self.items.iter().map(|i| i.filter_value()).collect();
        filter_matches(&term, unfiltered, self.items.len() as i32, &targets, self.filter)
    }
}

fn filter_matches(term: &str, unfiltered: bool, n: i32, targets: &[String], filter: FilterFunc) -> FilterMatchesMsg {
    if unfiltered {
        return FilterMatchesMsg((0..n).map(|i| FilteredItem { index: i, matches: Vec::new() }).collect());
    }
    FilterMatchesMsg(
        filter(term, targets)
            .into_iter()
            .map(|r| FilteredItem { index: r.index, matches: r.matched_indexes })
            .collect(),
    )
}

fn count_enabled_bindings(groups: &[Vec<Binding>]) -> i32 {
    groups.iter().flatten().filter(|b| b.enabled()).count() as i32
}

impl help::KeyMap for Model {
    fn short_help(&self) -> Vec<Binding> {
        let mut kb = vec![self.key_map.cursor_up.clone(), self.key_map.cursor_down.clone()];
        let filtering = self.filter_state == Filtering;
        // If the delegate implements the help.KeyMap interface add the
        // short help items to the short help after the cursor movement
        // keys.
        if !filtering && let Some(h) = self.delegate.short_help() {
            kb.extend(h);
        }
        kb.extend([
            self.key_map.filter.clone(),
            self.key_map.clear_filter.clone(),
            self.key_map.accept_while_filtering.clone(),
            self.key_map.cancel_while_filtering.clone(),
        ]);
        if !filtering && let Some(f) = self.additional_short_help_keys {
            kb.extend(f());
        }
        kb.extend([self.key_map.quit.clone(), self.key_map.show_full_help.clone()]);
        kb
    }

    fn full_help(&self) -> Vec<Vec<Binding>> {
        let k = &self.key_map;
        let mut kb = vec![vec![
            k.cursor_up.clone(),
            k.cursor_down.clone(),
            k.next_page.clone(),
            k.prev_page.clone(),
            k.go_to_start.clone(),
            k.go_to_end.clone(),
        ]];
        let filtering = self.filter_state == Filtering;
        // If the delegate implements the help.KeyMap interface add full
        // help keybindings to a special section of the full help.
        if !filtering && let Some(h) = self.delegate.full_help() {
            kb.extend(h);
        }
        let mut list_level = vec![
            k.filter.clone(),
            k.clear_filter.clone(),
            k.accept_while_filtering.clone(),
            k.cancel_while_filtering.clone(),
        ];
        if !filtering && let Some(f) = self.additional_full_help_keys {
            list_level.extend(f());
        }
        kb.push(list_level);
        kb.push(vec![k.quit.clone(), k.close_full_help.clone()]);
        kb
    }
}

#[cfg(test)]
mod tests;
