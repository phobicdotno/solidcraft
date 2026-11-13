use super::*;
use crate::scenario::Harness;
use serde_json::json;

fn with_box() -> Harness {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 20})).unwrap();
    h
}

fn open(h: &mut Harness) {
    h.app.start("fab.slice");
    h.frames(3);
}

fn form(h: &mut Harness) -> &mut Fab {
    match h.app.dialog.as_mut().map(|d| &mut d.kind) {
        Some(Kind::Fab(k)) => k.as_mut(),
        _ => panic!("no Slice for Laser / CNC window"),
    }
}

fn shown(label: &str) -> bool {
    crate::scenario::rect_of(&format!("dialog:row:{label}")).is_some()
}

#[test]
fn the_ribbon_opens_the_window_and_close_shuts_it() {
    let mut h = with_box();
    open(&mut h);
    assert!(matches!(h.app.dialog.as_ref().map(|d| &d.kind), Some(Kind::Fab(_))));
    assert!(h.app.dialog.as_ref().is_some_and(|d| d.inputs.is_empty()), "one solid body: nothing to pick");
    assert!(shown("Sheet thickness"));
    // It slices at once on opening (the result or the refusal comes back from another thread).
    let began = std::time::Instant::now();
    while h.app.fab.result.is_none() {
        assert!(began.elapsed().as_secs() < 30, "no slicing came back");
        h.frames(1);
    }
    form(&mut h).thickness = "4 mm".into();
    h.step(&json!({"widget": "Close"})).unwrap();
    h.frames(2);
    assert!(h.app.dialog.is_none());
    assert!(!h.app.fab.working() && h.app.fab.result.is_none(), "closing drops the slicing");
    // The next window starts from these settings.
    open(&mut h);
    assert_eq!(form(&mut h).thickness, "4 mm");
}

#[test]
fn a_programmatic_slice_opens_no_window() {
    let mut h = with_box();
    let _ = h.app.run("fab.slice", json!({}));
    h.frames(2);
    assert!(h.app.dialog.is_none());
}

#[test]
fn each_style_sends_the_settings_it_reads() {
    let h = Harness::new();
    let app = &h.app;
    let mut k = Fab::new(&app.session.doc);
    let p = params(app, &k, &[]).unwrap();
    for (key, want) in [
        ("style", json!("stacked")),
        ("axis", json!("Z")),
        ("thickness", json!(3.0)),
        ("clearance", json!(0.2)),
        ("dowels", json!(2)),
        ("dowel_diameter", json!(6.0)),
        ("guide_outline", json!(true)),
        ("wall", json!(4.0)),
        ("machine", json!("laser")),
        ("kerf", json!(0.15)),
        ("sheet_width", json!(600.0)),
        ("sheet_height", json!(400.0)),
        ("gap", json!(3.0)),
        ("labels", json!(true)),
        ("sheets", json!(true)),
        ("geometry", json!(true)),
    ] {
        assert_eq!(p[key], want, "{key} in {p}");
    }
    for key in ["spacing", "count", "rod_diameter", "plate_margin", "tool_diameter", "body"] {
        assert!(p.get(key).is_none(), "stacked layers send no {key}: {p}");
    }

    k.style = ROD;
    k.axis = 0;
    let p = params(app, &k, &[]).unwrap();
    assert_eq!(
        (p["axis"].clone(), p["rod_diameter"].clone(), p["spacing"].clone(), p["count"].clone()),
        (json!("X"), json!(8.0), json!(0.0), json!(0))
    );
    assert!(p.get("dowels").is_none() && p.get("plate_margin").is_none());

    k.style = 2;
    k.axis = 2;
    let p = params(app, &k, &[]).unwrap();
    assert_eq!(p["style"], "base");
    assert_eq!(p["axis"], "Y", "a base plate is sliced across X or Y");
    assert_eq!(p["plate_margin"], 10.0);

    k.style = WAFFLE;
    k.spacing = "12".into();
    k.count = "5".into();
    let p = params(app, &k, &[]).unwrap();
    assert!(p.get("axis").is_none() && p.get("plate_margin").is_none(), "{p}");
    assert_eq!((p["spacing"].clone(), p["count"].clone()), (json!(12.0), json!(5)));

    k.machine = 1;
    let p = params(app, &k, &[]).unwrap();
    assert_eq!(p["machine"], "cnc");
    assert_eq!(p["tool_diameter"], 3.175);
    assert!(p.get("kerf").is_none());

    k.count = "2.5".into();
    assert!(matches!(params(app, &k, &[]), Err(Problem::Field(e)) if e.starts_with("Number of slices:")));
    k.count = "0".into();
    k.thickness = "3 +".into();
    assert!(matches!(params(app, &k, &[]), Err(Problem::Field(e)) if e.starts_with("Sheet thickness:")));
}

#[test]
fn lengths_follow_the_design_unit() {
    let mut h = Harness::new();
    h.app.run("document.units", json!({"units": "in"})).unwrap();
    let mut k = Fab::new(&h.app.session.doc);
    assert!(k.thickness.ends_with(" in"), "{}", k.thickness);
    let p = params(&h.app, &k, &[]).unwrap();
    assert!((p["thickness"].as_f64().unwrap() - 3.0).abs() < 1e-9, "{p}");
    assert!((p["sheet_width"].as_f64().unwrap() - 600.0).abs() < 1e-9, "{p}");
    k.thickness = "0.25".into();
    let p = params(&h.app, &k, &[]).unwrap();
    assert!((p["thickness"].as_f64().unwrap() - 6.35).abs() < 1e-9, "{p}");
}

#[test]
fn several_bodies_ask_which_to_slice() {
    let mut h = with_box();
    h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "base": [100, 0, 0], "operation": "new"})).unwrap();
    h.app.run("select.clear", json!({})).unwrap();
    open(&mut h);
    let d = h.app.dialog.as_ref().unwrap();
    assert_eq!(d.inputs.len(), 1, "a body pick");
    assert_eq!(h.app.fab.problem, Some(Problem::PickBody));
    assert!(h.app.fab.result.is_none(), "nothing is sliced until a body is picked");
    let name = crate::open_meshing::session_world(&h.app.session).bodies[0].name.clone();
    h.app.dialog.as_mut().unwrap().inputs[0].items.push(Sel::Body { name: name.clone() });
    let (k, inputs) = (form(&mut h).clone(), h.app.dialog.as_ref().unwrap().inputs.clone());
    assert_eq!(params(&h.app, &k, &inputs).unwrap()["body"], json!(name));
}

#[test]
fn the_rows_follow_the_style_and_the_machine() {
    let mut h = with_box();
    open(&mut h);
    assert!(shown("Stacking axis") && shown("Dowel holes") && shown("Dowel diameter") && shown("Kerf") && shown("Material around holes"));
    assert!(!shown("Rod diameter") && !shown("Slice spacing") && !shown("Number of slices") && !shown("Plate margin") && !shown("Bit diameter"));
    form(&mut h).style = ROD;
    h.frames(2);
    assert!(shown("Rod axis") && shown("Rod diameter") && shown("Slice spacing") && shown("Number of slices"));
    assert!(!shown("Dowel holes") && !shown("Plate margin"));
    form(&mut h).style = 3;
    h.frames(2);
    assert!(shown("Slicing axis") && shown("Plate margin") && !shown("Rod diameter"));
    form(&mut h).style = WAFFLE;
    form(&mut h).machine = 1;
    h.frames(2);
    assert!(!shown("Slicing axis") && !shown("Rod axis") && !shown("Stacking axis") && !shown("Plate margin"));
    assert!(shown("Bit diameter") && !shown("Kerf"));
}

#[test]
fn a_new_style_starts_on_its_own_axis() {
    let mut h = with_box();
    open(&mut h);
    assert_eq!(form(&mut h).axis, 2);
    h.step(&json!({"widget": "Stacked layers"})).unwrap();
    h.step(&json!({"widget": "Slices on a rod"})).unwrap();
    h.frames(2);
    assert_eq!((form(&mut h).style, form(&mut h).axis), (ROD, 0));
}

fn result() -> Value {
    json!({
        "body": "Body1", "style": "rod", "thickness": 3.0, "count": 9,
        "parts": [
            {"name": "S1", "kind": "slice", "copies": 6, "area_mm2": 1.0, "width_mm": 1.0, "height_mm": 1.0},
            {"name": "R", "kind": "spacer", "copies": 3, "area_mm2": 1.0, "width_mm": 1.0, "height_mm": 1.0}
        ],
        "warnings": [
            {"code": "no_material", "message": "No material at X = 12.5 mm: that slice is left out", "axis": "X", "at": 12.5},
            {"code": "falls_apart", "message": "S1 falls apart into 2 pieces", "part": "S1", "pieces": 2},
            {"code": "dowels_short", "message": "Room for 1 of 2 dowel holes in every layer", "placed": 1, "wanted": 2},
            {"code": "rod_hub", "message": "S2: …", "part": "S2"},
            {"code": "no_tab", "message": "S3 …", "part": "S3"},
            {"code": "gaps_not_whole", "message": "…", "thickness": 3.0},
            {"code": "never_cross", "message": "The two sets of slices never cross: nothing holds them together"},
            {"code": "something_new", "message": "Something new"}
        ],
        "sheet_size": [600.0, 400.0],
        "sheets": [{"parts": ["S1"], "cuts": 1, "engraved": 0}, {"parts": ["R"], "cuts": 1, "engraved": 0}]
    })
}

#[test]
fn the_summary_counts_parts_by_kind_and_sheets() {
    let h = Harness::new();
    let mut v = result();
    assert_eq!(summary(&h.app, &v), "9 parts (6 slices, 3 spacer rings) on 2 sheets");
    v["parts"] = json!([
        {"name": "S1", "kind": "slice", "copies": 4},
        {"name": "Base", "kind": "plate", "copies": 1}
    ]);
    v["count"] = json!(5);
    v["sheets"] = json!([{"parts": []}]);
    assert_eq!(summary(&h.app, &v), "5 parts (4 slices, 1 plate) on 1 sheet");
    v["sheets"] = json!([]);
    assert_eq!(summary(&h.app, &v), "5 parts (4 slices, 1 plate)");
}

#[test]
fn warnings_are_worded_by_their_code() {
    let h = Harness::new();
    let lines = warnings(&h.app, &result());
    let mm = |v: f64| crate::prefs::show_mm(&h.app, v, 1);
    assert_eq!(
        lines,
        [
            format!("No material at X = {}: that slice is left out", mm(12.5)).as_str(),
            "S1 falls apart into 2 pieces",
            "Room for 1 of 2 dowel holes in every layer",
            "S2: the rod misses it, so it gets a hub around the rod",
            "S3 is too narrow where it meets the plate for a tab",
            format!("The gaps between slices are not a whole number of sheets: use a spacing that is a multiple of {} for exact spacers", mm(3.0))
                .as_str(),
            "… and 2 more",
        ]
    );
    let mut v = result();
    v["warnings"] = json!([{"code": "never_cross", "message": "x"}, {"code": "something_new", "message": "Something new"}]);
    assert_eq!(warnings(&h.app, &v), ["The two sets of slices never cross: nothing holds them together", "Something new"]);
}

#[test]
fn warnings_follow_the_language() {
    let mut h = Harness::new();
    h.app.preferences.language = "de".into();
    let lines = warnings(&h.app, &result());
    assert_ne!(lines[1], "S1 falls apart into 2 pieces");
    assert!(lines[1].contains("S1") && lines[1].contains('2'), "{}", lines[1]);
}

#[test]
fn a_part_too_big_for_the_sheet_is_worded_by_the_window() {
    let mut h = Harness::new();
    let mut v = result();
    v["sheets_error"] = json!("fab.slice: L1 (100.2 x 60.2 mm) does not fit on a 80 x 80 mm sheet with a 5 mm margin");
    v["does_not_fit"] =
        json!({"part": "L1", "width_mm": 100.15, "height_mm": 60.15, "sheet_width_mm": 80.0, "sheet_height_mm": 80.0, "margin_mm": 5.0});
    let mm = |v: f64| crate::prefs::show_mm(&h.app, v, 1);
    let want = format!(
        "L1 ({} x {}) does not fit on a {} x {} sheet with a {} margin: use a bigger sheet",
        mm(100.15),
        mm(60.15),
        mm(80.0),
        mm(80.0),
        mm(5.0)
    );
    assert_eq!(sheets_error(&h.app, &v).as_deref(), Some(want.as_str()));
    h.app.preferences.language = "de".into();
    let de = sheets_error(&h.app, &v).unwrap();
    assert!(de.contains("L1") && de != want, "{de}");
    // any other reason is the command's own words
    v["does_not_fit"] = Value::Null;
    v["sheets_error"] = json!("fab.slice: too much to cut: parts");
    assert_eq!(sheets_error(&h.app, &v).as_deref(), Some("fab.slice: too much to cut: parts"));
}

#[test]
fn the_export_stem_drops_the_sheet_extension() {
    assert_eq!(stem("/tmp/a/vase-sheets.dxf"), "/tmp/a/vase-sheets");
    assert_eq!(stem("C:\\x\\Vase.SVG"), "C:\\x\\Vase");
    assert_eq!(stem("/tmp/plain"), "/tmp/plain");
}

#[test]
fn shapes_place_every_copy_and_fill_every_sheet() {
    let square = json!([[0, 0], [10, 0], [10, 10], [0, 10]]);
    let at = |z: f64| json!({"origin": [0, 0, z], "u": [1, 0, 0], "v": [0, 1, 0], "n": [0, 0, 1]});
    let v = json!({
        "thickness": 2.0,
        "parts": [{"name": "L1", "kind": "slice", "copies": 2, "rings": [square], "places": [at(1.0), at(3.0)]},
                  {"name": "P", "kind": "plate", "copies": 1, "rings": [square], "places": [at(-5.0)]}],
        "sheets": [{"cut_rings": [square, [[2, 2], [4, 2], [4, 4], [2, 4]]]}]
    });
    let (plates, fills) = shapes(&v);
    assert_eq!(plates.len(), 3);
    assert_eq!(plates.iter().filter(|(_, slice)| *slice).count(), 2);
    let volume: f64 = plates.iter().map(|(m, _)| m.measure().volume).sum();
    assert!((volume - 3.0 * 200.0).abs() < 1e-6, "{volume}");
    let area: f64 = fills[0].iter().map(|[a, b, c]| ((*b - *a).cross(*c - *a) * 0.5).abs()).sum();
    assert!((area - 96.0).abs() < 1e-6, "{area}");
}

#[test]
fn the_sheets_view_takes_the_flat_camera() {
    let mut h = with_box();
    open(&mut h);
    assert!(!crate::dialogs_sheet::flat_active(&h.app));
    h.step(&json!({"widget": "Sheets"})).unwrap();
    h.frames(2);
    assert!(sheets_active(&h.app) && crate::dialogs_sheet::flat_active(&h.app));
    assert_eq!(crate::navigation_bar_view::canvas_caps(&h.app), crate::navigation::NavCaps { orbit: false, pick: false });
}

fn wait_ready(h: &mut Harness) {
    let began = std::time::Instant::now();
    while h.app.fab.busy() || h.app.fab.result.is_none() {
        assert!(began.elapsed().as_secs() < 60, "no slicing came back");
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.frames(1);
    }
    h.frames(2);
}

#[test]
fn a_bad_field_holds_the_export_until_it_is_mended() {
    let mut h = with_box();
    open(&mut h);
    wait_ready(&mut h);
    assert!(crate::scenario::count_of("fab:ready") == Some(1.0));
    let thickness = form(&mut h).thickness.clone();
    form(&mut h).thickness = "x".into();
    h.frames(2);
    // The last parts stay drawn, but Export sheets… would write settings the form no longer shows.
    assert!(h.app.fab.problem.is_some() && h.app.fab.result.is_some());
    assert!(crate::scenario::count_of("fab:ready") == Some(0.0), "the export is held");
    form(&mut h).thickness = thickness;
    wait_ready(&mut h);
    assert!(h.app.fab.problem.is_none() && crate::scenario::count_of("fab:ready") == Some(1.0));
}

#[test]
fn a_sphere_shows_its_parts_assembled_then_its_sheets() {
    let mut h = Harness::new();
    h.app.run("solid.sphere", json!({"radius": 40})).unwrap();
    open(&mut h);
    wait_ready(&mut h);
    let v = h.app.fab.result.clone().unwrap().unwrap_or_else(|e| panic!("{e}"));
    assert!(v["count"].as_u64().unwrap_or(0) > 5, "{}", v["count"]);
    assert!(!h.app.fab.plates.is_empty() && h.app.preview.active, "the parts are drawn assembled");
    assert_eq!(h.app.preview.effective_replaced(), [v["body"].as_str().unwrap().to_string()], "in place of the body");
    assert!(crate::scenario::count_of("fab:ready") == Some(1.0));
    // A change waits for the settings to settle, then slices again.
    let before = h.app.fab.generation;
    form(&mut h).thickness = "6 mm".into();
    h.frames(1);
    assert!(h.app.fab.busy());
    // Export sheets… writes only what is shown: it waits for the change to be sliced.
    h.frames(1);
    assert!(h.app.fab.busy() && crate::scenario::count_of("fab:ready") == Some(0.0), "the export is held");
    wait_ready(&mut h);
    assert!(h.app.fab.generation > before);
    assert_eq!(h.app.fab.result.as_ref().unwrap().as_ref().unwrap()["thickness"], 6.0);
    h.step(&json!({"widget": "Sheets"})).unwrap();
    h.frames(3);
    assert!(!h.app.preview.active, "the sheets view draws no parts in 3D");
    assert!(!h.app.fab.fills.is_empty() && h.app.flat_view.fitted, "the sheets are framed");
}
