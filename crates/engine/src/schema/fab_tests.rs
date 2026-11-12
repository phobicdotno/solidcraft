use serde_json::{Value, json};

use super::{FabExportParams, FabExportResult, FabSliceParams, FabSliceResult};
use crate::Session;
use crate::schema::tests::{assert_keys, assert_strict_keeps_todays_errors, conforms};
use crate::schema::{ParamCheck, input_schema_for, output_schema_for};

/// The settings both commands take.
const SETTINGS: [&str; 22] = [
    "axis",
    "body",
    "clearance",
    "count",
    "dowel_diameter",
    "dowels",
    "gap",
    "guide_outline",
    "kerf",
    "label_height",
    "labels",
    "machine",
    "margin",
    "plate_margin",
    "rod_diameter",
    "sheet_height",
    "sheet_width",
    "spacing",
    "style",
    "thickness",
    "tool_diameter",
    "wall",
];

fn keys(extra: &[&'static str]) -> Vec<&'static str> {
    let mut all: Vec<&str> = SETTINGS.iter().copied().chain(extra.iter().copied()).collect();
    all.sort_unstable();
    all
}

#[test]
fn schemas_and_the_key_check_agree() {
    // Each command takes the settings and only its own keys: no `path` for a slicing, no
    // `geometry` for an export.
    assert_keys(input_schema_for::<FabSliceParams>(), &keys(&["geometry", "sheets"]));
    assert_keys(input_schema_for::<FabExportParams>(), &keys(&["formats", "path"]));
}

/// The design has no solid body (a sketch only), so a call whose parameters read fails there;
/// a value of the wrong shape fails first, with the same words with the check off and in strict.
#[test]
fn strict_keeps_todays_errors() {
    assert_strict_keeps_todays_errors(&[
        ("fab.slice", json!({})),
        ("fab.slice", json!({"count": "lots"})),
        ("fab.slice", json!({"style": "origami"})),
        ("fab.slice", json!({"axis": "W"})),
        ("fab.slice", json!({"thickness": "thick"})),
        ("fab.slice", json!({"geometry": 1})),
        ("fab.slice", json!({"body": "Nope"})),
        ("fab.export_sheets", json!({})),
        ("fab.export_sheets", json!({"formats": "dxf"})),
        ("fab.export_sheets", json!({"formats": ["pdf"]})),
        ("fab.export_sheets", json!({"formats": []})),
        ("fab.export_sheets", json!({"machine": "plasma"})),
        ("fab.export_sheets", json!({"labels": "yes"})),
    ]);
}

/// A slicing and an export of a 100 x 60 x 40 block reply as their output schemas say: in warn
/// without warnings for good calls (with the preview's geometry, without sheets, and with a part
/// too big for the sheet), and with them for a key the command does not take, which the check
/// off ignores and strict refuses.
#[test]
fn slicings_and_exports_reply_with_their_schemas() {
    let (sliced, exported) = (output_schema_for::<FabSliceResult>(), output_schema_for::<FabExportResult>());
    let matches = |schema: &schemars::Schema, r: &Value, warned: bool| {
        let schema = schema.as_value();
        assert!(conforms(schema, schema, r), "{r} doesn't match {schema}");
        assert_eq!(r.get("param_warnings").is_some(), warned, "{r}");
    };
    let block = |level: ParamCheck| {
        let mut s = Session { param_check: level, ..Session::default() };
        s.execute("solid.box", &json!({"length": 100, "width": 60, "height": 40})).unwrap();
        s
    };
    let mut s = block(ParamCheck::Warn);
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    let r = run("fab.slice", json!({"thickness": 5}));
    matches(&sliced, &r, false);
    assert_eq!((r["count"].as_u64(), r["sheets"].as_array().map(Vec::len)), (Some(8), Some(1)), "{r}");
    let r = run("fab.slice", json!({"style": "waffle", "count": 3, "geometry": true, "machine": "cnc", "tool_diameter": 3}));
    matches(&sliced, &r, false);
    assert!(r["parts"][0]["rings"].is_array() && r["sheets"][0]["cut_rings"].is_array(), "{r}");
    matches(&sliced, &run("fab.slice", json!({"thickness": 5, "sheets": false})), false);
    let r = run("fab.slice", json!({"thickness": 5, "sheet_width": 50, "sheet_height": 50}));
    matches(&sliced, &r, false);
    assert!(r["sheets_error"].is_string(), "{r}");
    let r = run("fab.export_sheets", json!({"thickness": 5, "formats": ["svg", "dxf"], "labels": false}));
    matches(&exported, &r, false);
    assert_eq!(r["files"].as_array().map(Vec::len), Some(2), "{r}");
    // Keys of the other command, and a misspelt one, are warned about; the call still runs.
    let r = run("fab.slice", json!({"thickness": 5, "path": "x"}));
    matches(&sliced, &r, true);
    let r = run("fab.export_sheets", json!({"thickness": 5, "geometry": true, "thicknes": 2}));
    matches(&exported, &r, true);
    assert_eq!(r["param_warnings"].as_array().map(Vec::len), Some(2), "{r}");
    // With the check off an unknown key is ignored, as for every command; strict refuses it.
    assert!(block(ParamCheck::Off).execute("fab.slice", &json!({"thickness": 5, "colour": "red"})).is_ok());
    let e = block(ParamCheck::Strict).execute("fab.slice", &json!({"thickness": 5, "colour": "red"})).unwrap_err().to_string();
    assert!(e.contains("colour"), "{e}");
}

/// Each setting's description gives the default a slicing starts from: the recipe's, through
/// `solidcraft_kernel::fab`.
#[test]
fn the_documented_defaults_are_the_recipes() {
    use solidcraft_kernel::fab::{DEFAULT_KERF, DEFAULT_SETTINGS as D, DEFAULT_SHEETS as S, DEFAULT_TOOL_DIAMETER};
    let schema = input_schema_for::<FabSliceParams>();
    for (key, value) in [
        ("thickness", D.thickness.to_string()),
        ("spacing", D.spacing.to_string()),
        ("count", D.count.to_string()),
        ("clearance", D.clearance.to_string()),
        ("rod_diameter", D.rod_diameter.to_string()),
        ("dowels", D.dowels.to_string()),
        ("dowel_diameter", D.dowel_diameter.to_string()),
        ("wall", D.wall.to_string()),
        ("plate_margin", D.plate_margin.to_string()),
        ("guide_outline", D.guide_outline.to_string()),
        ("kerf", DEFAULT_KERF.to_string()),
        ("tool_diameter", DEFAULT_TOOL_DIAMETER.to_string()),
        ("sheet_width", S.width.to_string()),
        ("sheet_height", S.height.to_string()),
        ("gap", S.gap.to_string()),
        ("margin", S.margin.to_string()),
        ("labels", S.labels.to_string()),
        ("label_height", S.label_height.to_string()),
    ] {
        let text = schema.as_value()["properties"][key]["description"].as_str().unwrap_or_default();
        assert!(text.contains(&format!("Default: {value}.")), "{key}: {text}");
    }
}
