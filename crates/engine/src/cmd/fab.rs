//! Slice for Laser / CNC: a solid body cut into flat parts to cut from sheet wood or acrylic and
//! build the model back up (stacked layers, slices on a rod, on a base or back plate, or a waffle
//! grid), the parts finished for a laser or a CNC router and packed onto sheets, and the sheets
//! written as DXF and SVG (cuts red, engraving blue).
//!
//! An analysis and an export: neither command changes the design. The parts follow the body: run
//! again after a change and they are cut from the changed body. The geometry is solidcore's
//! (`solidcore_ops::fab` through `solidcraft_kernel::fab`); this file reads the parameters,
//! names the body, words the warnings and writes the files.

use serde_json::{Map, Value, json};
use solidcraft_kernel::fab::{self as kfab, FabResult, FabSettings, FlatError, Machine, SheetSettings, Warning};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::fab::{
    FabAxis, FabDoesNotFit, FabExportParams, FabExportResult, FabFile, FabFormat, FabMachine, FabPartInfo, FabSettingsParams, FabSheetInfo,
    FabSliceParams, FabSliceResult, FabStyle, FabWarning, read,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session};

const SLICE_CONTRACT: TypedContract<FabSliceParams> = TypedContract::new();
const EXPORT_CONTRACT: TypedContract<FabExportParams> = TypedContract::new();

/// The keys every slicing reads, for the params text.
macro_rules! settings_doc {
    () => {
        "body? (name; default the only solid body); style?: stacked|rod|base|back|waffle (default stacked); \
    axis?: X|Y|Z (stacked: the stacking direction, default Z; rod: the rod's direction, default X; base and back: X or Y, default X; waffle: ignored); \
    thickness? (sheet, mm, default 3); spacing? (centre to centre, 0 automatic); count? (upright or rod slices, overrides spacing); \
    clearance? (play in slots and holes, default 0.2); rod_diameter? (default 8); dowels? (stacked, default 2, 0 none); dowel_diameter? (default 6); \
    wall? (material round holes, spacer ring width, default 4); plate_margin? (base/back, default 10); guide_outline? (stacked: engrave where the next layer goes, default true); \
    machine?: laser|cnc (default laser); kerf? (laser, default 0.15); tool_diameter? (cnc bit, default 3.175); \
    sheet_width?, sheet_height? (default 600 x 400); gap? (between parts, default 3); margin? (from the sheet edge, default 5); labels? (engrave part names, default true); label_height? (default 5)"
    };
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("fab.slice", "Slice for Laser / CNC", crate::checked!("fab.slice", slice))
        .at("INSERT", "FILES")
        .icon("fab_slice")
        .noundo()
        .contract(&SLICE_CONTRACT)
        .output(output_schema_for::<FabSliceResult>)
        .params(concat!(
            "the settings below; sheets? (also pack the parts onto sheets, default true); geometry? (each part's rings and 3D places, each sheet's lines, for a preview) \
             → parts (name, kind slice|plate|spacer, copies, area, size), warnings ({code, message, ...}), sheets (parts on each) or sheets_error. \
             Changes nothing: the parts follow the body each time it runs. Settings: ",
            settings_doc!()
        )),
    CommandSpec::new("fab.export_sheets", "Export Sheets for Laser / CNC", crate::checked!("fab.export_sheets", export_sheets))
        .icon("fab_slice")
        .noundo()
        .contract(&EXPORT_CONTRACT)
        .output(output_schema_for::<FabExportResult>)
        .params(concat!(
            "the settings: ",
            settings_doc!(),
            "; path? (without extension: writes <path>-sheet1.dxf, <path>-sheet1.svg, ... and removes that path's \
             older sheet files numbered past the last sheet; without it the texts are returned); \
             formats?: [dxf, svg] (default both) → files [{sheet, format, path|text}], removed: only cut and engrave paths, no sheet outline: DXF layers CUT (red) and ENGRAVE (blue); SVG cuts stroked red, engraving blue",
        )),
];

fn axis(a: FabAxis) -> kfab::Axis {
    match a {
        FabAxis::X => kfab::Axis::X,
        FabAxis::Y => kfab::Axis::Y,
        FabAxis::Z => kfab::Axis::Z,
    }
}

/// The slicing and sheet settings, the recipe's defaults ([`kfab::DEFAULT_SETTINGS`],
/// [`kfab::DEFAULT_SHEETS`]) filled in; the ranges are the kernel's to check.
pub(crate) fn settings(p: &FabSettingsParams) -> (FabSettings, SheetSettings) {
    let d = kfab::DEFAULT_SETTINGS;
    let style = p.style.unwrap_or_default();
    let default_axis = match style {
        FabStyle::Stacked => kfab::Axis::Z,
        _ => kfab::Axis::X,
    };
    let fs = FabSettings {
        style: match style {
            FabStyle::Stacked => kfab::Style::Stacked,
            FabStyle::Rod => kfab::Style::Rod,
            FabStyle::Base => kfab::Style::Base,
            FabStyle::Back => kfab::Style::Back,
            FabStyle::Waffle => kfab::Style::Waffle,
        },
        axis: p.axis.map_or(default_axis, axis),
        thickness: p.thickness.unwrap_or(d.thickness),
        spacing: p.spacing.unwrap_or(d.spacing),
        count: p.count.unwrap_or(d.count),
        clearance: p.clearance.unwrap_or(d.clearance),
        rod_diameter: p.rod_diameter.unwrap_or(d.rod_diameter),
        dowels: p.dowels.unwrap_or(d.dowels),
        dowel_diameter: p.dowel_diameter.unwrap_or(d.dowel_diameter),
        wall: p.wall.unwrap_or(d.wall),
        plate_margin: p.plate_margin.unwrap_or(d.plate_margin),
        guide_outline: p.guide_outline.unwrap_or(d.guide_outline),
        ..d
    };
    let sd = kfab::DEFAULT_SHEETS;
    let machine = match p.machine.unwrap_or_default() {
        FabMachine::Laser => Machine::Laser { kerf: p.kerf.unwrap_or(kfab::DEFAULT_KERF) },
        FabMachine::Cnc => Machine::Router { tool_diameter: p.tool_diameter.unwrap_or(kfab::DEFAULT_TOOL_DIAMETER) },
    };
    let ss = SheetSettings {
        width: p.sheet_width.unwrap_or(sd.width),
        height: p.sheet_height.unwrap_or(sd.height),
        gap: p.gap.unwrap_or(sd.gap),
        margin: p.margin.unwrap_or(sd.margin),
        machine,
        labels: p.labels.unwrap_or(sd.labels),
        label_height: p.label_height.unwrap_or(sd.label_height),
    };
    (fs, ss)
}

/// The kernel's failure (not a refusal) as the command's error: internal for a panic it caught.
fn kernel(cmd: &str, e: solidcraft_kernel::KernelError) -> EngineError {
    match EngineError::from(e) {
        EngineError::Other(m) => EngineError::Other(format!("{cmd}: {m}")),
        other => other,
    }
}

/// The parameter a kernel setting's name stands for.
fn param_name(setting: &str) -> &str {
    match setting {
        "width" => "sheet_width",
        "height" => "sheet_height",
        other => other,
    }
}

/// The name of the part at `index` of a slicing's parts (`part 1`, ... without the slicing).
fn part_name(res: Option<&FabResult>, index: usize) -> String {
    res.and_then(|r| r.parts.get(index)).map_or_else(|| format!("part {}", index + 1), |p| p.name.clone())
}

/// A packing's refusal for a part too big for the sheet, as data for the reply.
fn does_not_fit(e: &FlatError, res: &FabResult, ss: &SheetSettings) -> Option<FabDoesNotFit> {
    let FlatError::DoesNotFit { part, width, height } = *e else {
        return None;
    };
    Some(FabDoesNotFit {
        part: part_name(Some(res), part),
        width_mm: width,
        height_mm: height,
        sheet_width_mm: ss.width,
        sheet_height_mm: ss.height,
        margin_mm: ss.margin,
    })
}

/// A slicing's refusal as the command's error, in the parameters' words.
fn refusal(cmd: &str, e: &FlatError, res: Option<&FabResult>, ss: &SheetSettings) -> EngineError {
    match e {
        FlatError::Setting { name, limit } => bad(cmd, format!("`{}` is out of range: it must be {limit}", param_name(name))),
        FlatError::Nothing => bad(cmd, "nothing to cut: the slices miss the body"),
        FlatError::TooBig(what) => bad(cmd, format!("too much to cut: {what}")),
        FlatError::DoesNotFit { part, width, height } => {
            let name = part_name(res, *part);
            bad(
                cmd,
                format!(
                    "{name} ({width:.1} x {height:.1} mm) does not fit on a {} x {} mm sheet with a {} mm margin: use a bigger sheet",
                    ss.width, ss.height, ss.margin
                ),
            )
        }
        FlatError::Kernel(_) | FlatError::Section { .. } | FlatError::OpenSection { .. } => {
            EngineError::Other(format!("{cmd}: slicing the body failed: {e}"))
        }
    }
}

fn axis_name(a: kfab::Axis) -> &'static str {
    match a {
        kfab::Axis::X => "X",
        kfab::Axis::Y => "Y",
        kfab::Axis::Z => "Z",
    }
}

/// A warning with its code, its English words and its details.
fn warning(w: &Warning) -> FabWarning {
    let mut d = Map::new();
    let (code, message) = match w {
        Warning::NoMaterial { axis, at } => {
            d.insert("axis".into(), json!(axis_name(*axis)));
            d.insert("at".into(), json!(at));
            ("no_material", format!("No material at {} = {at:.1} mm: that slice is left out", axis_name(*axis)))
        }
        Warning::FallsApart { part, pieces } => {
            d.insert("part".into(), json!(part));
            d.insert("pieces".into(), json!(pieces));
            ("falls_apart", format!("{part} falls apart into {pieces} pieces"))
        }
        Warning::DowelsShort { placed, wanted } => {
            d.insert("placed".into(), json!(placed));
            d.insert("wanted".into(), json!(wanted));
            ("dowels_short", format!("Room for {placed} of {wanted} dowel holes in every layer"))
        }
        Warning::RodHub { part } => {
            d.insert("part".into(), json!(part));
            ("rod_hub", format!("{part}: the rod misses it, so it gets a hub around the rod"))
        }
        Warning::GapsNotWhole { thickness } => {
            d.insert("thickness".into(), json!(thickness));
            (
                "gaps_not_whole",
                format!(
                    "The gaps between slices are not a whole number of sheets: use a spacing that is a multiple of {thickness} mm for exact spacers"
                ),
            )
        }
        Warning::NeverCross => ("never_cross", "The two sets of slices never cross: nothing holds them together".to_string()),
    };
    FabWarning { code: code.into(), message, details: d }
}

fn kind_name(k: kfab::PartKind) -> &'static str {
    match k {
        kfab::PartKind::Slice => "slice",
        kfab::PartKind::Plate => "plate",
        kfab::PartKind::Spacer => "spacer",
    }
}

fn frame_json(f: &kfab::Frame) -> Value {
    json!({"origin": f.origin, "u": f.u, "v": f.v, "n": f.n})
}

/// The body to slice: the one named, else the design's only solid body.
fn body_of(s: &Session, p: &FabSettingsParams, cmd: &str) -> Result<(String, solidcraft_kernel::Body)> {
    let world = s.world_state()?;
    let body = match p.body.as_deref().filter(|b| !b.trim().is_empty()) {
        Some(name) => world.body(name).ok_or_else(|| bad(cmd, format!("no body `{name}`")))?,
        None => {
            let mut solids = world.bodies.iter().filter(|b| !b.body.is_mesh());
            match (solids.next(), solids.next()) {
                (Some(b), None) => b,
                (None, _) => return Err(bad(cmd, "the design has no solid body to slice")),
                (Some(_), Some(_)) => return Err(bad(cmd, "the design has several bodies: give `body`")),
            }
        }
    };
    if body.body.is_mesh() {
        return Err(bad(cmd, format!("`{}` is a mesh body: only solid bodies can be sliced yet", body.name)));
    }
    Ok((body.name.clone(), body.body.clone()))
}

/// The body cut into parts.
fn run_slice(s: &Session, p: &FabSettingsParams, cmd: &str) -> Result<(String, FabResult, SheetSettings)> {
    let (name, body) = body_of(s, p, cmd)?;
    let (fs, ss) = settings(p);
    let res = kfab::slice_body(&body, &fs).map_err(|e| kernel(cmd, e))?.map_err(|e| refusal(cmd, &e, None, &ss))?;
    Ok((name, res, ss))
}

fn slice(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "fab.slice";
    let args: FabSliceParams = read(cmd, p)?;
    let (name, res, ss) = run_slice(s, &args.settings, cmd)?;
    let geometry = args.geometry.unwrap_or(false);
    let parts = res
        .parts
        .iter()
        .map(|part| {
            let [width_mm, height_mm] = kfab::part_size(&part.shape).map_err(|e| kernel(cmd, e))?.unwrap_or([0.0; 2]);
            Ok(FabPartInfo {
                name: part.name.clone(),
                kind: kind_name(part.kind).into(),
                copies: part.copies,
                area_mm2: kfab::part_area(&part.shape).map_err(|e| kernel(cmd, e))?,
                width_mm,
                height_mm,
                rings: geometry.then(|| part.shape.rings().cloned().collect()),
                places: geometry
                    .then(|| if part.places.is_empty() { vec![frame_json(&part.frame)] } else { part.places.iter().map(frame_json).collect() }),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (sheets, sheets_error, does_not_fit) = if args.sheets.unwrap_or(true) {
        match kfab::layout_sheets(&res, &ss).map_err(|e| kernel(cmd, e))? {
            Ok(sheets) => (
                sheets
                    .iter()
                    .map(|sh| FabSheetInfo {
                        parts: sh.parts.clone(),
                        cuts: sh.cuts.len(),
                        engraved: sh.engrave.len(),
                        cut_rings: geometry.then(|| sh.cuts.clone()),
                        engrave_lines: geometry.then(|| sh.engrave.clone()),
                    })
                    .collect(),
                None,
                None,
            ),
            Err(e @ (FlatError::DoesNotFit { .. } | FlatError::TooBig(_))) => {
                (Vec::new(), Some(refusal(cmd, &e, Some(&res), &ss).to_string()), does_not_fit(&e, &res, &ss))
            }
            Err(e) => return Err(refusal(cmd, &e, Some(&res), &ss)),
        }
    } else {
        (Vec::new(), None, None)
    };
    let reply = FabSliceResult {
        body: name,
        style: args.settings.style.unwrap_or_default(),
        thickness: res.thickness,
        count: res.count(),
        parts,
        warnings: res.warnings.iter().map(warning).collect(),
        sheet_size: [ss.width, ss.height],
        sheets,
        sheets_error,
        does_not_fit,
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

fn export_sheets(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "fab.export_sheets";
    let args: FabExportParams = read(cmd, p)?;
    let formats = args.formats.clone().unwrap_or_else(|| vec![FabFormat::Dxf, FabFormat::Svg]);
    if formats.is_empty() {
        return Err(bad(cmd, "`formats` must name dxf and/or svg"));
    }
    let path = args.path.as_deref().map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let (name, res, ss) = run_slice(s, &args.settings, cmd)?;
    let sheets = kfab::layout_sheets(&res, &ss).map_err(|e| kernel(cmd, e))?.map_err(|e| refusal(cmd, &e, Some(&res), &ss))?;
    // every file is made before the first is written
    let mut made = Vec::new();
    for (i, sh) in sheets.iter().enumerate() {
        let drawing =
            solidcraft_io::fab_out::SheetDrawing { width: ss.width, height: ss.height, cuts: &sh.cuts, engrave: &sh.engrave, parts: &sh.parts };
        for f in &formats {
            let text = match f {
                FabFormat::Dxf => solidcraft_io::fab_out::sheet_dxf(&drawing),
                FabFormat::Svg => solidcraft_io::fab_out::sheet_svg(&drawing),
            }
            .map_err(|e| EngineError::Other(format!("{cmd}: {e}")))?;
            made.push((i + 1, *f, text));
        }
    }
    let mut files = Vec::new();
    for (sheet, format, text) in made {
        let ext = extension(format);
        match &path {
            Some(stem) => {
                let file = format!("{stem}-sheet{sheet}.{ext}");
                solidcraft_io::vfs::check(&file).map_err(|e| EngineError::Other(format!("{file}: {e}")))?;
                solidcraft_io::write_atomic(std::path::Path::new(&file), text.as_bytes(), false)
                    .map_err(|e| EngineError::Other(format!("{file}: {e}")))?;
                files.push(FabFile { sheet, format, path: Some(file), text: None });
            }
            None => files.push(FabFile { sheet, format, path: None, text: Some(text) }),
        }
    }
    let removed = match &path {
        Some(stem) => remove_older_sheets(stem, &formats, sheets.len()).map_err(|e| EngineError::Other(format!("{cmd}: {e}")))?,
        None => 0,
    };
    let reply = FabExportResult {
        body: name,
        count: res.count(),
        sheets: sheets.len(),
        files,
        removed,
        warnings: res.warnings.iter().map(warning).collect(),
        param_warnings: Vec::new(),
    };
    serde_json::to_value(reply).map_err(|e| EngineError::Other(e.to_string()))
}

/// A sheet file's extension.
fn extension(format: FabFormat) -> &'static str {
    match format {
        FabFormat::Dxf => "dxf",
        FabFormat::Svg => "svg",
    }
}

/// Removes `<stem>-sheet{n}.<ext>` of each format for n past `last`, up to the first number with
/// no file: only names this command's own scheme makes, so an earlier, longer export to the same
/// path leaves no sheet behind. Returns how many it removed.
fn remove_older_sheets(stem: &str, formats: &[FabFormat], last: usize) -> std::io::Result<usize> {
    let mut removed = 0;
    for &f in formats {
        let ext = extension(f);
        for n in last + 1.. {
            match std::fs::remove_file(format!("{stem}-sheet{n}.{ext}")) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[path = "fab_tests.rs"]
mod tests;
