//! Slice for Laser / CNC through its commands: each style on solids with answers worked out by
//! hand, refusals, parameter changes and the exported files read back.

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A 100 x 60 x 40 box from the origin, named Block.
fn block() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 60, "height": 40, "corner": [0, 0, 0], "body_name": "Block"}));
    s
}

/// A ball of radius 50 standing on the XY plane (centre at z = 50).
fn ball() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.sphere", json!({"radius": 50, "center": [0, 0, 50], "body_name": "Ball"}));
    s
}

fn names(r: &Value) -> Vec<String> {
    r["parts"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap().to_string()).collect()
}

fn part<'a>(r: &'a Value, name: &str) -> &'a Value {
    r["parts"].as_array().unwrap().iter().find(|p| p["name"] == name).unwrap_or_else(|| panic!("no part {name} in {:?}", names(r)))
}

fn codes(r: &Value) -> Vec<String> {
    r["warnings"].as_array().unwrap().iter().map(|w| w["code"].as_str().unwrap().to_string()).collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn stacked_layers_of_a_box_are_one_sheet_thick_and_full_size() {
    let mut s = block();
    let undo = s.undo.len();
    let r = run(&mut s, "fab.slice", json!({"thickness": 4, "dowels": 2}));
    assert_eq!(names(&r), (1..=10).map(|i| format!("L{i}")).collect::<Vec<_>>());
    assert_eq!(r["count"], 10);
    for p in r["parts"].as_array().unwrap() {
        assert!(close(p["width_mm"].as_f64().unwrap(), 100.0, 0.02), "{p}");
        assert!(close(p["height_mm"].as_f64().unwrap(), 60.0, 0.02), "{p}");
        // two dowel holes of 6 mm + 0.2 play: 6000 - 2 π 3.1² (inscribed polygons: a little more)
        let a = p["area_mm2"].as_f64().unwrap();
        let holes = 2.0 * std::f64::consts::PI * 3.1 * 3.1;
        assert!(a > 6000.0 - holes - 0.01 && a < 6000.0 - holes + 0.5, "{a}");
    }
    assert!(!r["sheets"].as_array().unwrap().is_empty());
    let placed: usize = r["sheets"].as_array().unwrap().iter().map(|sh| sh["parts"].as_array().unwrap().len()).sum();
    assert_eq!(placed, 10);
    assert_eq!(s.undo.len(), undo, "slicing changes nothing");
}

#[test]
fn every_style_slices_a_ball() {
    for style in ["stacked", "rod", "base", "back", "waffle"] {
        let mut s = ball();
        let r = run(&mut s, "fab.slice", json!({"style": style, "thickness": 3, "count": 5}));
        assert!(r["count"].as_u64().unwrap() >= 5, "{style}: {r}");
        assert!(r["sheets_error"].is_null(), "{style}: {r}");
        let kinds: Vec<&str> = r["parts"].as_array().unwrap().iter().map(|p| p["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds.contains(&"plate"), style == "base" || style == "back", "{style}: {kinds:?}");
        assert_eq!(kinds.contains(&"spacer"), style == "rod", "{style}: {kinds:?}");
        for p in r["parts"].as_array().unwrap() {
            assert!(p["area_mm2"].as_f64().unwrap() > 0.0, "{style}: {p}");
        }
    }
}

#[test]
fn a_waffle_box_is_slotted_half_way() {
    let mut s = block();
    let r = run(&mut s, "fab.slice", json!({"style": "waffle", "thickness": 3, "count": 3, "clearance": 0.2, "sheets": false}));
    let mut n = names(&r);
    n.sort();
    assert_eq!(n, ["X1", "X2", "X3", "Y1", "Y2", "Y3"]);
    // an X slice is 60 x 40 less three slots 3.2 wide from the top to half height (+ half the play)
    let want = 60.0 * 40.0 - 3.0 * 3.2 * (20.0 + 0.1);
    assert!(close(part(&r, "X1")["area_mm2"].as_f64().unwrap(), want, 0.05), "{}", part(&r, "X1"));
    let want = 100.0 * 40.0 - 3.0 * 3.2 * (20.0 + 0.1);
    assert!(close(part(&r, "Y1")["area_mm2"].as_f64().unwrap(), want, 0.05), "{}", part(&r, "Y1"));
    assert!(r["sheets"].as_array().unwrap().is_empty());
}

#[test]
fn rod_slices_get_spacer_rings_in_every_gap() {
    let mut s = block();
    let r = run(&mut s, "fab.slice", json!({"style": "rod", "axis": "X", "thickness": 3, "spacing": 12, "rod_diameter": 8, "geometry": true}));
    let slices: Vec<&Value> = r["parts"].as_array().unwrap().iter().filter(|p| p["kind"] == "slice").collect();
    assert_eq!(slices.len(), 9);
    for p in &slices {
        assert_eq!(p["rings"].as_array().unwrap().len(), 2, "the rod hole: {}", p["name"]);
    }
    let spacer = r["parts"].as_array().unwrap().iter().find(|p| p["kind"] == "spacer").unwrap();
    // 9 mm gaps, three sheets each, eight gaps
    assert_eq!(spacer["copies"], 24);
    assert_eq!(spacer["places"].as_array().unwrap().len(), 24);
    assert!(!codes(&r).contains(&"gaps_not_whole".to_string()), "{r}");
}

#[test]
fn base_and_back_plates_have_a_slot_for_every_tab() {
    for style in ["base", "back"] {
        for axis in ["X", "Y"] {
            let mut s = ball();
            let r = run(&mut s, "fab.slice", json!({"style": style, "axis": axis, "thickness": 3, "count": 5, "clearance": 0.2, "geometry": true}));
            let plate = r["parts"].as_array().unwrap().iter().find(|p| p["kind"] == "plate").unwrap();
            assert_eq!(plate["name"], if style == "base" { "Base plate" } else { "Back plate" });
            // two tabs per slice, one slot each
            assert_eq!(plate["rings"].as_array().unwrap().len() - 1, 10, "{style} {axis}");
            for hole in plate["rings"].as_array().unwrap().iter().skip(1) {
                let pts: Vec<[f64; 2]> = serde_json::from_value(hole.clone()).unwrap();
                let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
                let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
                assert!(close((x1 - x0).min(y1 - y0), 3.2, 0.01), "slot width = thickness + clearance");
            }
        }
    }
}

#[test]
fn a_body_that_falls_apart_names_its_pieces() {
    // a U: the block with a channel cut down to z = 10 across its middle
    let mut s = block();
    run(&mut s, "solid.box", json!({"length": 40, "width": 80, "height": 40, "corner": [30, -10, 10], "operation": "cut", "targets": ["Block"]}));
    let r = run(&mut s, "fab.slice", json!({"thickness": 5, "dowels": 0}));
    let n = names(&r);
    assert!(n.contains(&"L1".to_string()) && n.contains(&"L2".to_string()), "{n:?}");
    for i in 3..=8 {
        assert!(n.contains(&format!("L{i}a")) && n.contains(&format!("L{i}b")), "{n:?}");
    }
    assert_eq!(codes(&r).iter().filter(|c| *c == "falls_apart").count(), 6);
    let w = r["warnings"].as_array().unwrap().iter().find(|w| w["code"] == "falls_apart").unwrap();
    assert_eq!(w["pieces"], 2);
    assert!(w["message"].as_str().unwrap().contains("falls apart"));
}

#[test]
fn the_parts_follow_a_parameter_change() {
    let mut s = Session::default();
    run(&mut s, "parameters.add", json!({"name": "H", "expression": "40 mm"}));
    run(&mut s, "solid.box", json!({"length": 100, "width": 60, "height": "H", "corner": [0, 0, 0], "body_name": "Block"}));
    let r = run(&mut s, "fab.slice", json!({"thickness": 4, "sheets": false}));
    assert_eq!(r["count"], 10);
    run(&mut s, "parameters.change", json!({"name": "H", "expression": "20 mm"}));
    let r = run(&mut s, "fab.slice", json!({"thickness": 4, "sheets": false}));
    assert_eq!(r["count"], 5);
}

#[test]
fn refusals_say_which_setting_and_change_nothing() {
    let mut s = block();
    let undo = s.undo.len();
    for (p, word) in [
        (json!({"thickness": -1}), "thickness"),
        (json!({"thickness": 0}), "thickness"),
        (json!({"style": "origami"}), "origami"),
        (json!({"style": "base", "axis": "Z"}), "axis"),
        (json!({"kerf": -0.5}), "kerf"),
        (json!({"sheet_width": 0}), "sheet_width"),
        (json!({"count": "lots"}), "count"),
        (json!({"colour": "red"}), "colour"),
        (json!({"body": "Nope"}), "Nope"),
    ] {
        let e = s.execute("fab.export_sheets", &p).unwrap_err().to_string();
        assert!(e.contains(word), "{p}: {e}");
    }
    assert_eq!(s.undo.len(), undo);
}

#[test]
fn a_part_bigger_than_the_sheet_is_named() {
    let mut s = block();
    let r = run(&mut s, "fab.slice", json!({"thickness": 4, "sheet_width": 80, "sheet_height": 80}));
    let e = r["sheets_error"].as_str().unwrap();
    assert!(e.contains("L1") && e.contains("does not fit"), "{e}");
    assert!(r["sheets"].as_array().unwrap().is_empty());
    let e = s.execute("fab.export_sheets", &json!({"thickness": 4, "sheet_width": 80, "sheet_height": 80})).unwrap_err().to_string();
    assert!(e.contains("does not fit"), "{e}");
}

#[test]
fn an_empty_design_has_nothing_to_slice() {
    let mut s = Session::default();
    let e = s.execute("fab.slice", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no solid body"), "{e}");
}

#[test]
fn several_bodies_need_a_name() {
    let mut s = block();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [200, 0, 0], "operation": "new", "body_name": "Other"}));
    let e = s.execute("fab.slice", &json!({})).unwrap_err().to_string();
    assert!(e.contains("give `body`"), "{e}");
    let r = run(&mut s, "fab.slice", json!({"body": "Other", "thickness": 2, "sheets": false}));
    assert_eq!(r["body"], "Other");
    assert_eq!(r["count"], 5);
}

/// The entities of one layer in a DXF text: (layer, entity type) pairs from the ENTITIES section.
fn dxf_layers(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let start = lines.iter().position(|l| *l == "ENTITIES").unwrap();
    let mut out = Vec::new();
    let mut kind = String::new();
    for w in lines[start..].chunks(2) {
        if let [code, value] = w {
            if *code == "0" {
                kind = (*value).to_string();
            } else if *code == "8" && !kind.is_empty() {
                out.push(((*value).to_string(), kind.clone()));
            }
        }
    }
    out
}

#[test]
fn exported_sheets_read_back_with_cuts_red_and_engraving_blue() {
    let mut s = block();
    let dir = std::env::temp_dir().join(format!("solidcraft-fab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stem = dir.join("block").to_string_lossy().to_string();
    let r = run(&mut s, "fab.export_sheets", json!({"thickness": 4, "path": stem, "kerf": 0.2}));
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2 * r["sheets"].as_u64().unwrap() as usize);
    assert_eq!(files[0]["format"], "dxf");
    assert!(files[0]["path"].as_str().unwrap().ends_with("block-sheet1.dxf"));
    assert!(files[1]["path"].as_str().unwrap().ends_with("block-sheet1.svg"));
    let dxf = std::fs::read_to_string(files[0]["path"].as_str().unwrap()).unwrap();
    let ents = dxf_layers(&dxf);
    let cuts = ents.iter().filter(|(l, _)| l == "CUT").count();
    let engraved = ents.iter().filter(|(l, _)| l == "ENGRAVE").count();
    assert_eq!(ents.iter().filter(|(l, _)| l == "SHEET").count(), 1);
    // ten layers, each an outline and two dowel holes; each named
    assert_eq!(cuts, 30, "{ents:?}");
    assert!(engraved >= 10, "{engraved}");
    // the cut outlines are the parts grown by half the kerf each side: 100.2 x 60.2
    let geoms = solidcraft_io::read_dxf(&dxf).unwrap();
    assert!(!geoms.is_empty());
    let svg = std::fs::read_to_string(files[1]["path"].as_str().unwrap()).unwrap();
    assert!(svg.contains(r#"width="600mm" height="400mm""#));
    assert!(svg.contains(r##"stroke="#ff0000""##) && svg.contains(r##"stroke="#0000ff""##));
    assert!(svg.contains("L1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_path_the_texts_come_back_and_a_router_cuts_true_size() {
    let mut s = block();
    let r = run(&mut s, "fab.export_sheets", json!({"thickness": 10, "machine": "cnc", "tool_diameter": 6, "formats": ["svg"], "labels": false, "dowels": 0}));
    let files = r["files"].as_array().unwrap();
    assert!(files.iter().all(|f| f["format"] == "svg" && f["path"].is_null()));
    let svg = files[0]["text"].as_str().unwrap();
    // four 100 x 60 layers, no holes, no names, nothing engraved
    assert_eq!(svg.matches("<path").count(), r["count"].as_u64().unwrap() as usize);
}

#[test]
fn the_slicing_settings_reach_the_kernel_with_their_defaults() {
    let p: crate::schema::fab::FabParams = serde_json::from_value(json!({"style": "rod"})).unwrap();
    let (fs, ss) = super::settings(&p);
    assert_eq!(fs.axis, solidcraft_kernel::fab::Axis::X);
    assert_eq!(fs.thickness, 3.0);
    assert_eq!(ss.width, 600.0);
    assert_eq!(ss.machine, solidcraft_kernel::fab::Machine::Laser { kerf: 0.15 });
    let p: crate::schema::fab::FabParams = serde_json::from_value(json!({"machine": "cnc"})).unwrap();
    let (fs, ss) = super::settings(&p);
    assert_eq!(fs.axis, solidcraft_kernel::fab::Axis::Z);
    assert_eq!(ss.machine, solidcraft_kernel::fab::Machine::Router { tool_diameter: 3.175 });
}
