//! Bubbles' list tests, ported, and the list as the interface sets it up.

use super::*;

struct Plain(&'static str);

impl Item for Plain {
    fn filter_value(&self) -> String {
        self.0.to_string()
    }
}

struct PlainDelegate;

impl ItemDelegate for PlainDelegate {
    fn render(&self, w: &mut String, m: &Model, index: i32, item: &dyn Item) {
        let Some(i) = item.downcast_ref::<Plain>() else { return };
        w.push_str(&m.styles.title_bar.render(&format!("{}. {}", index + 1, i.0)));
    }
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

fn items(v: &[&'static str]) -> Vec<Box<dyn Item>> {
    v.iter().map(|s| Box::new(Plain(s)) as Box<dyn Item>).collect()
}

fn names(m: &Model) -> Vec<String> {
    m.visible_items().iter().map(|i| i.filter_value()).collect()
}

#[test]
fn status_bar_item_name() {
    let mut list = Model::new(items(&["foo", "bar"]), PlainDelegate, 10, 10);
    assert!(list.status_view().contains("2 items"));
    list.set_items(items(&["foo"]));
    assert!(list.status_view().contains("1 item"));
}

#[test]
fn status_bar_without_items() {
    let list = Model::new(items(&[]), PlainDelegate, 10, 10);
    assert!(list.status_view().contains("No items"));
}

#[test]
fn custom_status_bar_item_name() {
    let mut list = Model::new(items(&["foo", "bar"]), PlainDelegate, 10, 10);
    list.set_status_bar_item_name("connection", "connections");
    assert!(list.status_view().contains("2 connections"));
    list.set_items(items(&["foo"]));
    assert!(list.status_view().contains("1 connection"));
    list.set_items(items(&[]));
    assert!(list.status_view().contains("No connections"));
}

#[test]
fn set_filter_text() {
    let mut list = Model::new(items(&["foo", "bar", "baz"]), PlainDelegate, 10, 10);
    list.set_filter_text("ba");
    list.set_filter_state(Unfiltered);
    assert_eq!(names(&list), vec!["foo", "bar", "baz"]);
    list.set_filter_state(Filtering);
    assert_eq!(names(&list), vec!["bar", "baz"]);
    list.set_filter_state(FilterApplied);
    assert_eq!(names(&list), vec!["bar", "baz"]);
}

#[test]
fn set_filter_state() {
    let mut list = Model::new(items(&["foo", "bar", "baz"]), PlainDelegate, 10, 10);
    list.set_filter_text("ba");
    let footer = |l: &Model| l.view().split('\n').next_back().unwrap_or("").to_string();
    list.set_filter_state(Unfiltered);
    let f = footer(&list);
    assert!(f.contains("up") && !f.contains("clear filter"), "{f:?}");
    list.set_filter_state(Filtering);
    let f = footer(&list);
    assert!(f.contains("filter") && !f.contains("more"), "{f:?}");
    list.set_filter_state(FilterApplied);
    assert!(footer(&list).contains("clear"));
}

struct Titled(&'static str, &'static str);

impl Item for Titled {
    fn filter_value(&self) -> String {
        self.0.to_string()
    }
    fn as_default_item(&self) -> Option<&dyn DefaultItem> {
        Some(self)
    }
}

impl DefaultItem for Titled {
    fn title(&self) -> String {
        self.0.to_string()
    }
    fn description(&self) -> String {
        self.1.to_string()
    }
}

/// the list as the interface makes it: no title, no help, no spacing.
fn interface_list(v: &[(&'static str, &'static str)]) -> Model {
    let mut d = new_default_delegate();
    d.set_spacing(0);
    let mut l = Model::new(Vec::new(), d, 0, 0);
    l.set_show_title(false);
    l.set_show_help(false);
    l.set_status_bar_item_name("file", "files");
    l.disable_quit_keybindings();
    l.set_items(v.iter().map(|(t, d)| Box::new(Titled(t, d)) as Box<dyn Item>).collect());
    l.set_size(30, 12);
    l
}

/// drive feeds a message to the list, and the messages its commands make
/// (the filter's matches) back to it.
fn drive(l: &mut Model, m: Msg) {
    let mut queue = vec![m];
    while let Some(m) = queue.pop() {
        if let Some(c) = l.update(&m)
            && let Some(m) = c()
        {
            match m.downcast::<tea::BatchMsg>() {
                Ok(b) => {
                    for c in b.0.into_iter().flatten() {
                        if let Some(m) = c() {
                            queue.push(m);
                        }
                    }
                }
                Err(m) => queue.push(m),
            }
        }
    }
}

#[test]
fn interface_list_draws_status_and_items() {
    let l = interface_list(&[("/apps/a.html", "file"), ("/apps/b.html", "file"), ("/conf/c.xml", "file")]);
    let v = ansi::strip(&l.view());
    let lines: Vec<&str> = v.split('\n').collect();
    assert_eq!(lines[0].trim(), "");
    assert_eq!(lines[1].trim_end(), "  3 files");
    assert_eq!(lines[3].trim_end(), "│ /apps/a.html");
    assert_eq!(lines[4].trim_end(), "│ file");
    assert_eq!(lines[5].trim_end(), "  /apps/b.html");
    assert_eq!(lines.len(), 12);
}

#[test]
fn filtering_by_typing() {
    let mut l = interface_list(&[("/apps/a.html", "file"), ("/apps/b.html", "file"), ("/conf/c.xml", "file")]);
    assert_eq!(l.filter_state(), Unfiltered);
    drive(&mut l, tea::msg(KeyPressMsg::parse("/")));
    assert_eq!(l.filter_state(), Filtering);
    assert!(l.setting_filter());
    for c in "conf".chars() {
        drive(&mut l, tea::msg(KeyPressMsg::char(c)));
    }
    assert_eq!(l.filter_value(), "conf");
    assert_eq!(l.visible_items().len(), 1);
    assert!(ansi::strip(&l.view()).contains("Filter: conf"));
    drive(&mut l, tea::msg(KeyPressMsg::parse("enter")));
    assert_eq!(l.filter_state(), FilterApplied);
    let v = ansi::strip(&l.view());
    assert!(v.contains("“conf” 1 file • 2 filtered"), "{v}");
    assert_eq!(l.selected_item().unwrap().filter_value(), "/conf/c.xml");
    drive(&mut l, tea::msg(KeyPressMsg::parse("esc")));
    assert_eq!(l.filter_state(), Unfiltered);
    assert_eq!(l.visible_items().len(), 3);
}

#[test]
fn cursor_moves_and_pages() {
    let many: Vec<(&'static str, &'static str)> =
        vec![("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5"), ("f", "6")];
    let mut l = interface_list(&many);
    l.set_size(30, 8);
    // a title line, the status bar and the pagination leave room for one
    assert_eq!(l.paginator.per_page, 1);
    drive(&mut l, tea::msg(KeyPressMsg::parse("down")));
    assert_eq!((l.index(), l.paginator.page), (1, 1));
    drive(&mut l, tea::msg(KeyPressMsg::parse("down")));
    assert_eq!((l.index(), l.paginator.page), (2, 2));
    drive(&mut l, tea::msg(KeyPressMsg::parse("G")));
    assert_eq!(l.index(), 5);
    l.select(0);
    assert_eq!(l.selected_item().unwrap().filter_value(), "a");
    assert!(ansi::strip(&l.view()).contains("  ••••••"));
}

#[test]
fn quit_keys_disabled() {
    let mut l = interface_list(&[("a", "1")]);
    let c = l.update(&tea::msg(KeyPressMsg::parse("ctrl+c")));
    assert!(c.is_none() || c.unwrap()().is_none_or(|m| !m.is::<tea::QuitMsg>()));
}
