//! Approved product appearance, not a generic WinUI TextBox preset.
use novaui::{argb, dsl::parse, dsl::Doc, Display, Ui};

#[test]
fn search_preserves_b7b33f5_chromeless_focus_and_bar_layers() {
    let mut ui = Ui::new(1280, 720, 1.0);
    let mut doc = Doc::new(parse(include_str!("../ui/player.nui")).unwrap());
    doc.set(&mut ui, "search-focused", false);
    doc.set(&mut ui, "panel-open", true);
    let root = ui.tree.root();
    doc.build(&mut ui, root).unwrap();
    let search = doc.id("search");
    let luminosity = doc.id("search-luminosity");
    let tint = doc.id("search-tint");
    let children = ui.tree.children(doc.id("search-box"));
    assert_eq!(&children[..3], &[luminosity, tint, search], "material compositing order changed");
    assert_eq!(ui.tree.el(luminosity).visual().bg, Some(argb(0x2BFFFFFF)));
    assert_eq!(ui.tree.el(tint).visual().bg, Some(argb(0x24808080)));
    assert!(ui.text_edit(search).is_some(), "styling must retain the real editor");
    for focused in [false, true, false] {
        for hovered in [false, true] {
            ui.tree.set_state(search, novaui::State::Focused, focused);
            ui.tree.set_state(search, novaui::State::Hovered, hovered);
            doc.set(&mut ui, "search-focused", focused);
            doc.update(&mut ui);
            let v = &ui.tree.el(search).visual();
            assert!(
                v.bg.is_none() && v.bg_hover.is_none() && v.bg_focus.is_none(),
                "native input fill leaked into product search"
            );
            assert_eq!(v.border, 0.0, "search must have no TextBox outline");
            assert!(v.underline.is_none() && v.underline_focus.is_none(), "search must have no accent underline");
            for id in [luminosity, tint] {
                assert_eq!(
                    ui.tree.el(id).layout().display == Display::None,
                    focused,
                    "bar material must disappear only while focused, not on hover"
                );
            }
        }
    }
}
