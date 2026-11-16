//! Emulated list boxes used by the headless video dialog.

// Links the generated guest code that defines recomp_lookup (MinGW's linker
// does not drop the runtime's unused reference to it).
use bugatron_runtime_tests as _;
use bugatron_runtime::dialog::{ListBox, LB_ERR};

fn lb(items: &[&str]) -> ListBox {
    let mut l = ListBox::default();
    for (i, s) in items.iter().enumerate() {
        assert_eq!(l.add(s.to_string()), i as u32);
    }
    l
}

#[test]
fn select_string_matches_prefix_case_insensitively_and_wraps() {
    let mut l = lb(&["Window", "640 x 480 x 32", "640 x 480 x 16", "800 x 600 x 32"]);
    assert_eq!(l.cur_sel(), LB_ERR);
    assert_eq!(l.select_string(u32::MAX, "640 x 480 x 16"), 2);
    assert_eq!(l.cur_sel(), 2);
    assert_eq!(l.select_string(u32::MAX, "window"), 0);
    // Search starts after the given index and wraps around.
    assert_eq!(l.select_string(1, "640"), 2);
    assert_eq!(l.select_string(2, "640"), 1);
    assert_eq!(l.select_string(u32::MAX, "1024"), LB_ERR);
    assert_eq!(l.cur_sel(), 1, "failed search keeps the selection");
}

#[test]
fn delete_returns_remaining_count_and_adjusts_selection() {
    let mut l = lb(&["a", "b", "c"]);
    l.select_string(u32::MAX, "c");
    assert_eq!(l.delete(0), 2);
    assert_eq!(l.cur_sel(), 1);
    assert_eq!(l.delete(1), 1);
    assert_eq!(l.cur_sel(), LB_ERR);
    assert_eq!(l.delete(5), LB_ERR);
}
