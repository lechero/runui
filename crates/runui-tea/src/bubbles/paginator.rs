//! Pages of items (Bubbles' `paginator`): which page shows, how many items a
//! page holds, and the page shown as dots or "2/5".

use super::key::{self, Binding};
use crate::tea::{Cmd, KeyPressMsg, Msg};

/// Type is how the pages are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Type {
    /// As numbers: "2/5".
    #[default]
    Arabic,
    /// As dots, the page shown lit.
    Dots,
}

/// KeyMap is the keys that turn pages.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    /// Prev page.
    pub prev_page: Binding,
    /// Next page.
    pub next_page: Binding,
}

/// default_key_map is Bubbles' keys.
pub fn default_key_map() -> KeyMap {
    KeyMap {
        prev_page: Binding::new().with_keys(&["pgup", "left", "h"]),
        next_page: Binding::new().with_keys(&["pgdown", "right", "l"]),
    }
}

/// Model is the paginator.
#[derive(Clone, Debug)]
pub struct Model {
    /// Type .
    pub type_: Type,
    /// Page.
    pub page: i32,
    /// Per page.
    pub per_page: i32,
    /// Total pages.
    pub total_pages: i32,
    /// Active dot.
    pub active_dot: String,
    /// Inactive dot.
    pub inactive_dot: String,
    /// arabic_format formats the page and the total, as Go's "%d/%d".
    pub arabic_format: fn(i32, i32) -> String,
    /// Key map.
    pub key_map: KeyMap,
}

fn arabic(page: i32, total: i32) -> String {
    format!("{page}/{total}")
}

impl Default for Model {
    fn default() -> Self {
        Model::new()
    }
}

impl Model {
    /// new is one page of one item, shown as numbers.
    pub fn new() -> Model {
        Model {
            type_: Type::Arabic,
            page: 0,
            per_page: 1,
            total_pages: 1,
            active_dot: "•".into(),
            inactive_dot: "○".into(),
            arabic_format: arabic,
            key_map: default_key_map(),
        }
    }

    /// set_total_pages sets how many pages `items` take, and is that.
    pub fn set_total_pages(&mut self, items: i32) -> i32 {
        if items < 1 {
            return self.total_pages;
        }
        let mut n = items / self.per_page;
        if items % self.per_page > 0 {
            n += 1;
        }
        self.total_pages = n;
        n
    }

    /// items_on_page is how many of `total_items` are on the page shown.
    pub fn items_on_page(&self, total_items: i32) -> i32 {
        if total_items < 1 {
            return 0;
        }
        let (start, end) = self.get_slice_bounds(total_items);
        end - start
    }

    /// get_slice_bounds is where the page shown starts and ends in a list
    /// `length` long.
    pub fn get_slice_bounds(&self, length: i32) -> (i32, i32) {
        let start = self.page * self.per_page;
        let end = (self.page * self.per_page + self.per_page).min(length);
        (start, end)
    }

    /// prev_page goes a page back.
    pub fn prev_page(&mut self) {
        if self.page > 0 {
            self.page -= 1;
        }
    }

    /// next_page goes a page on.
    pub fn next_page(&mut self) {
        if !self.on_last_page() {
            self.page += 1;
        }
    }

    /// on_last_page says whether the last page shows.
    pub fn on_last_page(&self) -> bool {
        self.page == self.total_pages - 1
    }

    /// on_first_page says whether the first page shows.
    pub fn on_first_page(&self) -> bool {
        self.page == 0
    }

    /// update turns pages with its keys.
    pub fn update(&mut self, msg: &Msg) -> Cmd {
        if let Some(k) = msg.downcast_ref::<KeyPressMsg>() {
            if key::matches(k, &[&self.key_map.next_page]) {
                self.next_page();
            } else if key::matches(k, &[&self.key_map.prev_page]) {
                self.prev_page();
            }
        }
        None
    }

    /// view shows the page.
    pub fn view(&self) -> String {
        match self.type_ {
            Type::Dots => {
                let mut s = String::new();
                for i in 0..self.total_pages {
                    s.push_str(if i == self.page { &self.active_dot } else { &self.inactive_dot });
                }
                s
            }
            Type::Arabic => (self.arabic_format)(self.page + 1, self.total_pages),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tea;

    #[test]
    fn set_total_pages() {
        for (items, initial, expected) in [(0, 1, 1), (1, 1, 1), (10, 1, 10), (10, 3, 10), (11, 3, 11)] {
            let mut m = Model::new();
            m.total_pages = initial;
            m.set_total_pages(items);
            assert_eq!(m.total_pages, expected, "{items}");
        }
    }

    #[test]
    fn prev_and_next_page() {
        let mut m = Model::new();
        m.set_total_pages(10);
        m.update(&tea::msg(tea::KeyPressMsg::parse("right")));
        assert_eq!(m.page, 1);
        m.update(&tea::msg(tea::KeyPressMsg::parse("left")));
        m.update(&tea::msg(tea::KeyPressMsg::parse("left")));
        assert_eq!(m.page, 0);
    }

    #[test]
    fn items_on_page() {
        let mut m = Model::new();
        m.per_page = 10;
        m.set_total_pages(25);
        m.page = 2;
        assert_eq!(m.items_on_page(25), 5);
        assert_eq!(m.get_slice_bounds(25), (20, 25));
        m.type_ = Type::Dots;
        assert_eq!(m.view(), "○○•");
        m.type_ = Type::Arabic;
        assert_eq!(m.view(), "3/3");
    }
}
