//! Slice for Laser / CNC (`fab.slice`): the window that sets how a body is cut into flat parts
//! and packed onto sheets, shows the result as it is set (the parts assembled in the viewport,
//! or the sheets as a flat page) and writes the sheets (`fab.export_sheets`).
//!
//! Neither command changes the design. A setting changed in the window is sliced again once the
//! settings have been still for [`DEBOUNCE_S`]; the slicing runs on a copy of the design on a
//! thread of its own (on the web, where there are none, at once), so the window keeps drawing
//! and the last result stays up meanwhile. Writing the sheets runs the same way.

use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Align2, FontId, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};
use solidcore_record::{Vec2, Vec3};
use solidcore_tess::PlateFrame;
use solidcraft_engine::Sel;
use solidcraft_engine::Session;
use solidcraft_engine::doc::expr::Kind as ValueKind;
use solidcraft_engine::geom::prelude::*;
use solidcraft_engine::geom::{Aabb3, Mesh};
use solidcraft_theme::Tokens;

use crate::SolveApp;
use crate::dialogs::{Dialog, Kind};
use crate::gpu::GpuScene;
use crate::selection::{BODIES, SelInput};
use crate::ui_kit;

/// The build styles, as `fab.slice` names them.
pub(crate) const STYLES: [&str; 5] = ["stacked", "rod", "base", "back", "waffle"];
const STYLE_LABELS: [&str; 5] = ["fab.style_stacked", "fab.style_rod", "fab.style_base", "fab.style_back", "fab.style_waffle"];
/// One line on each style, under the style.
const STYLE_HELP: [&str; 5] = ["fab.help_stacked", "fab.help_rod", "fab.help_base", "fab.help_back", "fab.help_waffle"];
const AXES: [&str; 3] = ["X", "Y", "Z"];
/// The machines, as `fab.slice` names them, and their labels.
const MACHINES: [&str; 2] = ["laser", "cnc"];
const MACHINE_LABELS: [&str; 2] = ["fab.machine_laser", "fab.machine_cnc"];
/// How long the settings stay still before they are sliced again (seconds).
pub(crate) const DEBOUNCE_S: f64 = 0.3;
/// Most dowel holes the form offers: the recipe's.
const MAX_DOWELS: i64 = solidcraft_engine::kernel::fab::MAX_DOWELS as i64;
/// Most upright or rod slices the form offers: the recipe's.
const MAX_SLICES: i64 = solidcraft_engine::kernel::fab::MAX_SLICES as i64;
/// Most warnings listed; the rest are counted.
const MAX_WARNINGS: usize = 6;
/// Most triangles the 3D preview draws, all plates together.
const MAX_TRIANGLES: usize = 3_000_000;

const STACKED: usize = 0;
const ROD: usize = 1;
const BASE: usize = 2;
const BACK: usize = 3;
const WAFFLE: usize = 4;

/// The window's settings, as typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Fab {
    /// Index into [`STYLES`].
    pub style: usize,
    /// Index into [`AXES`].
    pub axis: usize,
    pub thickness: String,
    /// Empty: automatic.
    pub spacing: String,
    /// 0: the spacing decides.
    pub count: String,
    pub clearance: String,
    pub rod_diameter: String,
    pub dowels: String,
    pub dowel_diameter: String,
    pub guide_outline: bool,
    pub wall: String,
    pub plate_margin: String,
    /// Index into [`MACHINES`].
    pub machine: usize,
    pub kerf: String,
    pub tool_diameter: String,
    pub sheet_width: String,
    pub sheet_height: String,
    pub gap: String,
    pub labels: bool,
    /// 0: the parts assembled in the viewport, 1: the sheets.
    pub view: usize,
    /// The design has several solid bodies, so one is picked.
    pub picks_body: bool,
}

impl Fab {
    /// The commands' defaults (the recipe's, `solidcraft_kernel::fab`), in the design's length
    /// unit.
    pub fn new(doc: &solidcraft_engine::doc::Document) -> Fab {
        use solidcraft_engine::kernel::fab::{DEFAULT_KERF, DEFAULT_SETTINGS as D, DEFAULT_SHEETS as S, DEFAULT_TOOL_DIAMETER};
        let len = |mm: f64| crate::length_input::model(doc, mm);
        Fab {
            style: STACKED,
            axis: 2,
            thickness: len(D.thickness),
            spacing: String::new(),
            count: D.count.to_string(),
            clearance: len(D.clearance),
            rod_diameter: len(D.rod_diameter),
            dowels: D.dowels.to_string(),
            dowel_diameter: len(D.dowel_diameter),
            guide_outline: D.guide_outline,
            wall: len(D.wall),
            plate_margin: len(D.plate_margin),
            machine: 0,
            kerf: len(DEFAULT_KERF),
            tool_diameter: len(DEFAULT_TOOL_DIAMETER),
            sheet_width: len(S.width),
            sheet_height: len(S.height),
            gap: len(S.gap),
            labels: S.labels,
            view: 0,
            picks_body: false,
        }
    }

    /// The style's name, as `fab.slice` takes it.
    pub fn style_name(&self) -> &'static str {
        STYLES.get(self.style).copied().unwrap_or("stacked")
    }

    fn has_spacing(&self) -> bool {
        self.style != STACKED
    }
}

/// The axes a style is sliced along: stacked and rod any, a base or back plate X or Y, a waffle
/// none (it always slices across X and Y).
pub(crate) fn axis_choices(style: usize) -> &'static [&'static str] {
    match style {
        STACKED | ROD => &AXES,
        WAFFLE => &[],
        _ => &AXES[..2],
    }
}

/// The axis a style starts with: stacked layers along Z, everything else across X.
fn default_axis(style: usize) -> usize {
    if style == STACKED { 2 } else { 0 }
}

fn axis_label(style: usize) -> &'static str {
    match style {
        STACKED => "fab.axis_stacked",
        ROD => "fab.axis_rod",
        _ => "fab.axis_plate",
    }
}

/// Why the settings make no slicing yet.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Problem {
    /// The design has several solid bodies and none is picked.
    PickBody,
    /// A field that does not read as a value, in words.
    Field(String),
}

/// The window for `fab.slice`, with the settings it had last.
pub fn start(app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    if id != "fab.slice" {
        return None;
    }
    let world = crate::open_meshing::session_world(&app.session);
    let solids = world.bodies.iter().filter(|b| !b.body.is_mesh()).count();
    let mut f = app.fab.last.clone().unwrap_or_else(|| Fab::new(&app.session.doc));
    f.view = 0;
    f.picks_body = solids > 1;
    // With one solid body there is nothing to pick: `fab.slice` takes the only one.
    let inputs = if f.picks_body { vec![SelInput::new("Body", BODIES, false)] } else { Vec::new() };
    Some((Kind::Fab(Box::new(f)), inputs))
}

/// The parameters of `fab.slice` the settings make: only the keys their style and machine read,
/// with each part's rings and places and each sheet's lines for the previews.
pub(crate) fn params(app: &SolveApp, k: &Fab, inputs: &[SelInput]) -> Result<Value, Problem> {
    let doc = &app.session.doc;
    let field = |label: &str, e: &dyn std::fmt::Display| {
        Problem::Field(
            solidcraft_ui_text::Locale::for_language(&app.preferences.language)
                .format("fab.field_error", &[("field", app.tr(label)), ("error", &e.to_string())]),
        )
    };
    let length = |label: &str, text: &str| -> Result<f64, Problem> {
        crate::length_input::eval(doc, text, ValueKind::Length)
            .map_err(|e| field(label, &e))
            .and_then(|v| if v.is_finite() { Ok(v) } else { Err(field(label, &"not a number")) })
    };
    let count = |label: &str, text: &str, max: i64| -> Result<u32, Problem> {
        let v = doc.eval(text, ValueKind::Unitless).map_err(|e| field(label, &e))?;
        let n = v.round();
        if !v.is_finite() || (v - n).abs() > 1e-9 || n < 0.0 || n > max as f64 {
            let range = solidcraft_ui_text::Locale::for_language(&app.preferences.language)
                .format("fab.count_error", &[("min", "0"), ("max", &max.to_string())]);
            return Err(field(label, &range));
        }
        Ok(n as u32)
    };
    let body = crate::selection::items(inputs, 0).iter().find_map(|s| match s {
        Sel::Body { name } => Some(name.clone()),
        _ => None,
    });
    if k.picks_body && body.is_none() {
        return Err(Problem::PickBody);
    }
    let mut p = json!({
        "style": k.style_name(),
        "thickness": length("fab.thickness", &k.thickness)?,
        "clearance": length("fab.clearance", &k.clearance)?,
        "wall": length("fab.wall", &k.wall)?,
        "machine": MACHINES.get(k.machine).copied().unwrap_or("laser"),
        "sheet_width": length("fab.sheet_width", &k.sheet_width)?,
        "sheet_height": length("fab.sheet_height", &k.sheet_height)?,
        "gap": length("fab.gap", &k.gap)?,
        "labels": k.labels,
        "sheets": true,
        "geometry": true,
    });
    if let Some(b) = body {
        p["body"] = json!(b);
    }
    let axes = axis_choices(k.style);
    if let Some(a) = axes.get(k.axis.min(axes.len().saturating_sub(1))) {
        p["axis"] = json!(a);
    }
    if k.has_spacing() {
        p["spacing"] = json!(if k.spacing.trim().is_empty() { 0.0 } else { length("fab.spacing", &k.spacing)? });
        p["count"] = json!(count("fab.count", &k.count, MAX_SLICES)?);
    }
    match k.style {
        STACKED => {
            p["dowels"] = json!(count("fab.dowels", &k.dowels, MAX_DOWELS)?);
            p["dowel_diameter"] = json!(length("fab.dowel_diameter", &k.dowel_diameter)?);
            p["guide_outline"] = json!(k.guide_outline);
        }
        ROD => p["rod_diameter"] = json!(length("fab.rod_diameter", &k.rod_diameter)?),
        WAFFLE => {}
        _ => p["plate_margin"] = json!(length("fab.plate_margin", &k.plate_margin)?),
    }
    if k.machine == 1 {
        p["tool_diameter"] = json!(length("fab.tool_diameter", &k.tool_diameter)?);
    } else {
        p["kerf"] = json!(length("fab.kerf", &k.kerf)?);
    }
    Ok(p)
}

/// The form: the view, what the last slicing found and Export sheets…, then the settings in
/// three groups (Build, Holding it together, Sheets and machine). True when Enter was accepted in
/// a field.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut Fab) -> bool {
    let mut enter = false;
    ui_kit::row(ui, app.tr("fab.view"), |ui| {
        ui_kit::segmented(ui, &mut k.view, &[app.tr("fab.view_3d"), app.tr("fab.view_sheets")], ui_kit::Segments::default())
    });

    report(app, ui);
    // Only the settings shown are written: a change still waiting to be sliced, or a field the
    // last result no longer matches (a bad number, a lost body pick), holds the export.
    let can_export = app.fab.problem.is_none()
        && app.fab.exporting.is_none()
        && !app.fab.busy()
        && app.fab.result.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|v| v["sheets"].as_array().is_some_and(|s| !s.is_empty())));
    // Scenarios wait on this: a result is in and nothing newer is coming.
    crate::scenario::publish_count("fab:ready", f64::from(u8::from(can_export)));
    let clicked = ui.add_enabled_ui(can_export, |ui| ui_kit::button(ui, app.tr("fab.export"), ui_kit::Button::Normal).clicked()).inner;
    if clicked {
        export(app);
    }
    match &app.fab.status {
        Some(Ok(s)) => {
            ui_kit::status_line(ui, s, ui_kit::StatusTone::Dim);
        }
        Some(Err(e)) => {
            ui_kit::status_line(ui, e, ui_kit::StatusTone::Error);
        }
        None => {}
    }
    ui_kit::divider(ui, "fab_result");
    ui_kit::section_heading(ui, "fab.group_build");
    let before = k.style;
    let styles: Vec<&str> = STYLE_LABELS.iter().map(|l| app.tr(l)).collect();
    ui_kit::choice_row(ui, app.tr("fab.style"), "fab_style", &mut k.style, &styles);
    k.style = k.style.min(STYLES.len() - 1);
    if k.style != before {
        k.axis = default_axis(k.style);
    }
    ui_kit::text(ui, app.tr(STYLE_HELP.get(k.style).copied().unwrap_or_default()), ui_kit::Text::Hint("fab_style_help"));
    let axes = axis_choices(k.style);
    if !axes.is_empty() {
        let mut axis = k.axis.min(axes.len() - 1);
        ui_kit::choice_row(ui, app.tr(axis_label(k.style)), "fab_axis", &mut axis, axes);
        k.axis = axis;
    }
    enter |= ui_kit::expr_row(ui, app.tr("fab.thickness"), &mut k.thickness);
    if k.has_spacing() {
        enter |= ui_kit::hint_row(ui, app.tr("fab.spacing"), &mut k.spacing, app.tr("fab.automatic"));
        enter |= count_row(app, ui, "fab.count", &mut k.count, MAX_SLICES);
        ui_kit::text(ui, app.tr("fab.count_hint"), ui_kit::Text::Hint("fab_count_hint"));
    }
    enter |= ui_kit::expr_row(ui, app.tr("fab.clearance"), &mut k.clearance);

    ui_kit::section_heading(ui, "fab.group_holding");
    match k.style {
        STACKED => {
            enter |= count_row(app, ui, "fab.dowels", &mut k.dowels, MAX_DOWELS);
            enter |= ui_kit::expr_row(ui, app.tr("fab.dowel_diameter"), &mut k.dowel_diameter);
            ui_kit::checkbox(ui, app.tr("fab.guide_outline"), &mut k.guide_outline);
        }
        ROD => enter |= ui_kit::expr_row(ui, app.tr("fab.rod_diameter"), &mut k.rod_diameter),
        _ => {}
    }
    enter |= ui_kit::expr_row(ui, app.tr("fab.wall"), &mut k.wall);
    if matches!(k.style, BASE | BACK) {
        enter |= ui_kit::expr_row(ui, app.tr("fab.plate_margin"), &mut k.plate_margin);
    }

    ui_kit::section_heading(ui, "fab.group_sheets");
    let machines = [app.tr(MACHINE_LABELS[0]), app.tr(MACHINE_LABELS[1])];
    ui_kit::choice_row(ui, app.tr("fab.machine"), "fab_machine", &mut k.machine, &machines);
    if k.machine == 1 {
        enter |= ui_kit::expr_row(ui, app.tr("fab.tool_diameter"), &mut k.tool_diameter);
    } else {
        enter |= ui_kit::expr_row(ui, app.tr("fab.kerf"), &mut k.kerf);
    }
    enter |= ui_kit::expr_row(ui, app.tr("fab.sheet_width"), &mut k.sheet_width);
    enter |= ui_kit::expr_row(ui, app.tr("fab.sheet_height"), &mut k.sheet_height);
    enter |= ui_kit::expr_row(ui, app.tr("fab.gap"), &mut k.gap);
    ui_kit::checkbox(ui, app.tr("fab.labels"), &mut k.labels);

    enter
}

fn count_row(app: &SolveApp, ui: &mut egui::Ui, label: &str, value: &mut String, max: i64) -> bool {
    let doc = &app.session.doc;
    let eval = |text: &str| doc.eval(text, ValueKind::Unitless).ok();
    ui_kit::count_row(ui, app.tr(label), value, &ui_kit::Count { min: 0, max, eval: &eval })
}

/// What the settings and the last slicing come to: a missing pick or a field in error, the
/// summary, the warnings, why the parts could not be packed, or the slicing's refusal.
fn report(app: &SolveApp, ui: &mut egui::Ui) {
    match &app.fab.problem {
        Some(Problem::PickBody) => {
            ui_kit::text(ui, app.tr("fab.pick_body"), ui_kit::Text::Hint("fab_pick_body"));
            return;
        }
        Some(Problem::Field(e)) => {
            ui_kit::status_line(ui, e, ui_kit::StatusTone::Error);
            return;
        }
        None => {}
    }
    if app.fab.busy() {
        ui_kit::status_line(ui, app.tr("fab.slicing"), ui_kit::StatusTone::Dim);
    }
    match &app.fab.result {
        Some(Ok(v)) => {
            ui_kit::text(ui, &summary(app, v), ui_kit::Text::Report("fab_summary", false));
            if let Some(e) = sheets_error(app, v) {
                ui_kit::status_line(ui, &e, ui_kit::StatusTone::Error);
            }
            for line in warnings(app, v) {
                ui_kit::status_line(ui, &line, ui_kit::StatusTone::Warning);
            }
        }
        Some(Err(e)) => {
            ui_kit::status_line(ui, e, ui_kit::StatusTone::Error);
        }
        None => {}
    }
}

/// Why the parts could not be packed: a part too big for the sheet in the window's language, from
/// the reply's `does_not_fit`; any other reason in the command's English words.
pub(crate) fn sheets_error(app: &SolveApp, v: &Value) -> Option<String> {
    let f = &v["does_not_fit"];
    if !f.is_object() {
        return v["sheets_error"].as_str().map(str::to_string);
    }
    let locale = solidcraft_ui_text::Locale::for_language(&app.preferences.language);
    let length = |key: &str| f[key].as_f64().map_or_else(|| f[key].to_string(), |mm| crate::prefs::show_mm(app, mm, 1));
    let part = f["part"].as_str().unwrap_or_default();
    Some(locale.format(
        "fab.does_not_fit",
        &[
            ("part", part),
            ("width", &length("width_mm")),
            ("height", &length("height_mm")),
            ("sheet_width", &length("sheet_width_mm")),
            ("sheet_height", &length("sheet_height_mm")),
            ("margin", &length("margin_mm")),
        ],
    ))
}

/// "N parts (a slices, 1 plate, b spacer rings) on M sheets", in the window's language.
pub(crate) fn summary(app: &SolveApp, v: &Value) -> String {
    let locale = solidcraft_ui_text::Locale::for_language(&app.preferences.language);
    let mut kinds: Vec<(&str, u64)> = Vec::new();
    for part in v["parts"].as_array().into_iter().flatten() {
        let kind = part["kind"].as_str().unwrap_or("slice");
        let copies = part["copies"].as_u64().unwrap_or(1);
        match kinds.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => *n += copies,
            None => kinds.push((kind, copies)),
        }
    }
    let words: Vec<String> = kinds
        .iter()
        .map(|&(kind, n)| {
            let key = match (kind, n == 1) {
                ("plate", true) => "fab.kind_plate_one",
                ("plate", false) => "fab.kind_plate_many",
                ("spacer", true) => "fab.kind_spacer_one",
                ("spacer", false) => "fab.kind_spacer_many",
                (_, true) => "fab.kind_slice_one",
                (_, false) => "fab.kind_slice_many",
            };
            locale.format(key, &[("count", &n.to_string())])
        })
        .collect();
    let kinds = words.join(locale.text("fab.list_separator"));
    let parts = v["count"].as_u64().unwrap_or_else(|| kinds_total(v)).to_string();
    let sheets = v["sheets"].as_array().map_or(0, Vec::len);
    let key = match sheets {
        0 => "fab.summary_no_sheets",
        1 => "fab.summary_one_sheet",
        _ => "fab.summary",
    };
    locale.format(key, &[("parts", &parts), ("kinds", &kinds), ("sheets", &sheets.to_string())])
}

fn kinds_total(v: &Value) -> u64 {
    v["parts"].as_array().into_iter().flatten().map(|p| p["copies"].as_u64().unwrap_or(1)).sum()
}

/// The warnings of a slicing in the window's language, by their code (an unknown code in the
/// command's English words), at most [`MAX_WARNINGS`] and a line counting the rest.
pub(crate) fn warnings(app: &SolveApp, v: &Value) -> Vec<String> {
    let locale = solidcraft_ui_text::Locale::for_language(&app.preferences.language);
    let all: Vec<&Value> = v["warnings"].as_array().into_iter().flatten().collect();
    let text = |w: &Value, key: &str| w[key].as_str().map_or_else(|| w[key].to_string(), str::to_string);
    let length = |w: &Value, key: &str| w[key].as_f64().map_or_else(|| text(w, key), |mm| crate::prefs::show_mm(app, mm, 1));
    let mut out: Vec<String> = all
        .iter()
        .take(MAX_WARNINGS)
        .map(|w| match w["code"].as_str().unwrap_or_default() {
            "no_material" => locale.format("fab.warn_no_material", &[("axis", &text(w, "axis")), ("at", &length(w, "at"))]),
            "falls_apart" => locale.format("fab.warn_falls_apart", &[("part", &text(w, "part")), ("pieces", &text(w, "pieces"))]),
            "dowels_short" => locale.format("fab.warn_dowels_short", &[("placed", &text(w, "placed")), ("wanted", &text(w, "wanted"))]),
            "rod_hub" => locale.format("fab.warn_rod_hub", &[("part", &text(w, "part"))]),
            "gaps_not_whole" => locale.format("fab.warn_gaps_not_whole", &[("thickness", &length(w, "thickness"))]),
            "never_cross" => locale.text("fab.warn_never_cross").to_string(),
            _ => w["message"].as_str().unwrap_or_default().to_string(),
        })
        .collect();
    if all.len() > MAX_WARNINGS {
        out.push(locale.format("fab.more_warnings", &[("count", &(all.len() - MAX_WARNINGS).to_string())]));
    }
    out
}

/// Ask where to write the sheets, then write the shown slicing's sheets there (`<stem>-sheet1.dxf`,
/// `<stem>-sheet1.svg`, ...) beside the window.
fn export(app: &mut SolveApp) {
    let Some((params, _)) = app.fab.shown.clone() else { return };
    let name = format!("{}-sheets.dxf", app.session.doc.name);
    app.pick_save(&name, &["dxf", "svg"], move |app, path| {
        let Some(path) = path else {
            app.fab.status = Some(Ok(app.tr("fab.export_cancelled").to_string()));
            return;
        };
        let mut p = params;
        if let Some(o) = p.as_object_mut() {
            o.remove("sheets");
            o.remove("geometry");
        }
        p["path"] = json!(stem(&path));
        let scratch = app.session.scratch();
        app.fab.status = Some(Ok(app.tr("fab.exporting").to_string()));
        app.fab.begin_export(scratch, p);
    });
}

/// The file name the picker gave without its sheet-file extension: the stem `fab.export_sheets`
/// adds `-sheet1.dxf` ... to.
pub(crate) fn stem(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    for ext in [".dxf", ".svg"] {
        if lower.ends_with(ext) {
            return path.get(..path.len().saturating_sub(ext.len())).unwrap_or(path).to_string();
        }
    }
    path.to_string()
}

/// What a slicing on another thread hands back: the reply, the plates to draw (each placed copy,
/// and whether it is a slice), and each sheet's fill.
struct Computed {
    result: Result<Value, String>,
    plates: Vec<(Mesh, bool)>,
    fills: Vec<Vec<[Vec2; 3]>>,
}

/// The settings and design revision a slicing is for.
type Key = (Value, u64);

/// The slicings and exports of the open window, and the settings it had last.
#[derive(Default)]
pub struct FabRun {
    /// The settings of the last window, for the next.
    pub last: Option<Fab>,
    open: bool,
    /// What the shown result was sliced from.
    shown: Option<Key>,
    /// Settings waiting for [`DEBOUNCE_S`] to pass, since when (egui time, s).
    wanted: Option<(Key, f64)>,
    running: Option<(Key, Receiver<Computed>)>,
    /// The last slicing's reply or refusal.
    pub result: Option<Result<Value, String>>,
    pub(crate) problem: Option<Problem>,
    plates: Vec<(Mesh, bool)>,
    fills: Vec<Vec<[Vec2; 3]>>,
    /// Counts the results, for the preview's key.
    generation: u64,
    exporting: Option<Receiver<Result<Value, String>>>,
    /// What the last export did.
    pub status: Option<Result<String, String>>,
}

impl FabRun {
    /// A slicing is running or waiting for the settings to settle.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.wanted.is_some()
    }

    /// Any work still going (a slicing, a wait or an export).
    pub fn working(&self) -> bool {
        self.busy() || self.exporting.is_some()
    }

    fn close(&mut self) {
        *self = FabRun { last: self.last.take(), ..FabRun::default() };
    }

    fn apply(&mut self, key: Key, done: Computed) {
        self.shown = Some(key);
        self.result = Some(done.result);
        self.plates = done.plates;
        self.fills = done.fills;
        self.generation = self.generation.wrapping_add(1);
    }

    fn begin_export(&mut self, scratch: Session, params: Value) {
        let job = move || {
            let mut s = scratch;
            s.execute("fab.export_sheets", &params).map_err(|e| e.to_string())
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            match std::thread::Builder::new().name("fab-export".into()).spawn(move || {
                let _ = tx.send(job());
            }) {
                Ok(_) => self.exporting = Some(rx),
                Err(e) => self.status = Some(Err(e.to_string())),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(job());
            self.exporting = Some(rx);
        }
    }
}

/// Slice the settings of `key` on a copy of the design: the reply, and the plates and sheet fills
/// the previews draw.
fn compute(scratch: Session, params: &Value) -> Computed {
    let mut s = scratch;
    match s.execute("fab.slice", params) {
        Ok(v) => {
            let (plates, fills) = shapes(&v);
            Computed { result: Ok(v), plates, fills }
        }
        Err(e) => Computed { result: Err(e.to_string()), plates: Vec::new(), fills: Vec::new() },
    }
}

fn read_rings(v: &Value) -> Vec<Vec<[f64; 2]>> {
    let point = |p: &Value| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]);
    v.as_array().into_iter().flatten().map(|ring| ring.as_array().into_iter().flatten().filter_map(point).collect()).collect()
}

fn read_vec(v: &Value) -> Option<Vec3> {
    Some(Vec3::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?))
}

/// Every copy of every part as a plate where it sits, and each sheet's fill. A plate or a fill
/// the kernel fails on is not drawn.
fn shapes(v: &Value) -> (Vec<(Mesh, bool)>, Vec<Vec<[Vec2; 3]>>) {
    use solidcraft_engine::kernel;
    let thickness = v["thickness"].as_f64().unwrap_or(0.0);
    let mut plates = Vec::new();
    let mut triangles = 0;
    'parts: for part in v["parts"].as_array().into_iter().flatten() {
        let rings = read_rings(&part["rings"]);
        let slice = part["kind"].as_str().is_none_or(|k| k == "slice");
        for place in part["places"].as_array().into_iter().flatten() {
            let (Some(origin), Some(u), Some(w), Some(n)) =
                (read_vec(&place["origin"]), read_vec(&place["u"]), read_vec(&place["v"]), read_vec(&place["n"]))
            else {
                continue;
            };
            let frame = PlateFrame { origin, u, v: w, n };
            let Ok(mesh) = kernel::guard("plate of a flat part", || Ok(solidcore_tess::plate(&rings, &frame, thickness))) else {
                continue;
            };
            triangles += mesh.triangles.len();
            if triangles > MAX_TRIANGLES {
                break 'parts;
            }
            plates.push((mesh, slice));
        }
    }
    let fills = v["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|sheet| {
            let rings = read_rings(&sheet["cut_rings"]);
            kernel::guard("fill of a sheet", || Ok(solidcore_tess::fill(&rings))).unwrap_or_default()
        })
        .collect();
    (plates, fills)
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(run: &mut FabRun, scratch: Session, key: Key) {
    let (tx, rx) = std::sync::mpsc::channel();
    let params = key.0.clone();
    match std::thread::Builder::new().name("fab-slice".into()).spawn(move || {
        let _ = tx.send(compute(scratch, &params));
    }) {
        Ok(_) => run.running = Some((key, rx)),
        Err(e) => run.apply(key, Computed { result: Err(e.to_string()), plates: Vec::new(), fills: Vec::new() }),
    }
}

/// No threads on the web: slice at once.
#[cfg(target_arch = "wasm32")]
fn spawn(run: &mut FabRun, scratch: Session, key: Key) {
    let done = compute(scratch, &key.0);
    run.apply(key, done);
}

/// Per frame: slice again once the settings (or the design) changed and settled, take a finished
/// slicing or export in. `None` without the window; with it, the preview key of the parts to draw
/// assembled in the viewport, or `None` when the viewport shows none (the Sheets view, no result).
pub(crate) fn tick(app: &mut SolveApp, ctx: &egui::Context) -> Option<Option<u64>> {
    let Some((k, inputs)) = app.dialog.as_ref().and_then(|d| match &d.kind {
        Kind::Fab(k) => Some((k.as_ref().clone(), d.inputs.clone())),
        _ => None,
    }) else {
        if app.fab.open || app.fab.working() {
            app.fab.close();
        }
        return None;
    };
    app.fab.open = true;
    app.fab.last = Some(k.clone());
    let now = ctx.input(|i| i.time);
    match params(app, &k, &inputs) {
        Err(problem) => {
            app.fab.problem = Some(problem);
            app.fab.wanted = None;
        }
        Ok(p) => {
            app.fab.problem = None;
            let key = (p, app.session.revision);
            let current = app.fab.shown.as_ref() == Some(&key) || app.fab.running.as_ref().is_some_and(|(r, _)| *r == key);
            if current {
                app.fab.wanted = None;
            } else {
                let waiting = app.fab.wanted.as_ref().filter(|(w, _)| *w == key).map(|(_, t)| *t);
                let since = match waiting {
                    Some(t) => t,
                    None => {
                        app.fab.wanted = Some((key.clone(), now));
                        now
                    }
                };
                // The first slicing of a window starts at once; a change waits for the next.
                let first = app.fab.shown.is_none();
                if app.fab.running.is_none() && (first || now - since >= DEBOUNCE_S) {
                    app.fab.wanted = None;
                    let scratch = app.session.scratch();
                    spawn(&mut app.fab, scratch, key);
                }
            }
        }
    }
    poll(app);
    if app.fab.working() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
    let shows = k.view == 0 && !app.fab.plates.is_empty() && app.fab.result.as_ref().is_some_and(Result::is_ok);
    Some(shows.then(|| preview_key(app.fab.generation)))
}

/// Take in a finished slicing or export.
fn poll(app: &mut SolveApp) {
    if let Some((_, rx)) = &app.fab.running {
        let done = match rx.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Computed { result: Err(app.tr("fab.slicing_stopped").to_string()), plates: Vec::new(), fills: Vec::new() })
            }
        };
        if let Some(done) = done
            && let Some((key, _)) = app.fab.running.take()
        {
            app.fab.apply(key, done);
            // The sheets view frames the new sheets.
            app.flat_view.fitted = false;
        }
    }
    if let Some(rx) = &app.fab.exporting {
        let done = match rx.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(app.tr("fab.export_stopped").to_string())),
        };
        if let Some(done) = done {
            app.fab.exporting = None;
            let status = done.map(|v| exported(app, &v));
            app.fab.status = Some(status);
        }
    }
}

/// "Wrote N files to FOLDER".
fn exported(app: &SolveApp, v: &Value) -> String {
    let files: Vec<&str> = v["files"].as_array().into_iter().flatten().filter_map(|f| f["path"].as_str()).collect();
    let folder = files.first().and_then(|f| std::path::Path::new(f).parent()).map(|p| p.display().to_string()).unwrap_or_default();
    solidcraft_ui_text::Locale::for_language(&app.preferences.language)
        .format("fab.exported", &[("count", &files.len().to_string()), ("folder", &folder)])
}

/// A preview key of its own, so no feature preview's key can match it.
fn preview_key(generation: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("fab.slice", generation).hash(&mut h);
    h.finish()
}

/// The parts assembled, as the viewport's preview: slices in the body's colour, plates and spacer
/// rings in the new-body colour, standing in for the body sliced.
pub(crate) fn built(app: &SolveApp) -> crate::preview::Built {
    let t = Tokens::get();
    let mut sc = GpuScene::default();
    let slice = sc.color(t.body.to_srgba_unmultiplied());
    let other = sc.color(t.preview_new.to_srgba_unmultiplied());
    let edge = sc.color(t.body_edge.to_srgba_unmultiplied());
    let mut bounds = Aabb3::EMPTY;
    for (m, is_slice) in &app.fab.plates {
        let fill = if *is_slice { slice } else { other };
        for tri in &m.triangles {
            for &i in tri {
                if let (Some(p), Some(n)) = (m.positions.get(i as usize), m.normals.get(i as usize)) {
                    sc.tri(p.to_f32(), n.to_f32(), fill);
                }
            }
        }
        for e in &m.edges {
            for w in e.windows(2) {
                sc.line(w[0].to_f32(), w[1].to_f32(), edge, 1.0, false);
            }
        }
        bounds = bounds.union(&m.bounds());
    }
    let replaced =
        app.fab.result.as_ref().and_then(|r| r.as_ref().ok()).and_then(|v| v["body"].as_str()).map(|b| vec![b.to_string()]).unwrap_or_default();
    crate::preview::Built {
        scene: sc,
        sketches: Vec::new(),
        replaced,
        error: None,
        bounds,
        applied: None,
        tapered: Vec::new(),
        stopped: None,
        placed: None,
    }
}

/// The window is open on its Sheets view, which has the viewport.
pub(crate) fn sheets_active(app: &SolveApp) -> bool {
    matches!(app.dialog.as_ref().map(|d| &d.kind), Some(Kind::Fab(k)) if k.view == 1)
}

/// The Sheets view: every sheet as a flat page, one under the other with "Sheet 1" ... above
/// each, the parts filled faintly, the cuts and the engraving in their colours, framed when a
/// result comes in. True when the window shows it.
pub fn overlay(app: &mut SolveApp, painter: &egui::Painter, rect: Rect, d: &Dialog) -> bool {
    let Kind::Fab(k) = &d.kind else { return false };
    if k.view != 1 {
        return false;
    }
    let t = Tokens::get();
    painter.rect_filled(rect, 0.0, t.panel);
    let Some(Ok(v)) = &app.fab.result else {
        painter.text(rect.center(), Align2::CENTER_CENTER, app.tr("fab.no_sheets"), FontId::proportional(t.font_heading), t.text_dim);
        return true;
    };
    let size = |i: usize| v["sheet_size"].get(i).and_then(Value::as_f64).filter(|x| x.is_finite() && *x > 0.0);
    let (Some(w), Some(h)) = (size(0), size(1)) else { return true };
    let sheets: Vec<&Value> = v["sheets"].as_array().into_iter().flatten().collect();
    if sheets.is_empty() {
        painter.text(rect.center(), Align2::CENTER_CENTER, app.tr("fab.no_sheets"), FontId::proportional(t.font_heading), t.text_dim);
        return true;
    }
    // Sheet i sits below sheet i - 1, a tenth of its height apart, room for its caption above.
    let pitch = h * 1.12;
    let base = |i: usize| -(i as f64) * pitch;
    let lo = (0.0, base(sheets.len() - 1));
    let hi = (w, h + h * 0.06);
    let content = crate::shell::content_rect(app, rect);
    let panel_left = app.viewport.dialog_rect.map_or(content.right(), |r| r.left());
    let area = Rect::from_min_max(content.min + vec2(40.0, 60.0), pos2(panel_left.min(content.right()) - 20.0, content.bottom() - 40.0));
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return true;
    }
    if !app.flat_view.fitted {
        app.flat_view.fit(rect, area, lo, hi);
    }
    let cam = app.flat_view.cam;
    let scale = f64::from(rect.height()) / (2.0 * cam.half_height());
    let ox = f64::from(rect.center().x) - cam.target.x * scale;
    let oy = f64::from(rect.center().y) + cam.target.y * scale;
    let to = |x: f64, y: f64| -> Pos2 { pos2((ox + x * scale) as f32, (oy - y * scale) as f32) };
    let font = FontId::proportional(t.font_body);
    let cut = Stroke::new(1.4, t.text);
    let engrave = Stroke::new(1.0, t.accent);
    let locale = solidcraft_ui_text::Locale::for_language(&app.preferences.language);
    for (i, sheet) in sheets.iter().enumerate() {
        let y0 = base(i);
        let page = Rect::from_two_pos(to(0.0, y0), to(w, y0 + h));
        if !page.intersects(rect) {
            continue;
        }
        painter.rect_filled(page, 0.0, t.dialog_body);
        painter.rect_stroke(page, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        painter.text(
            page.left_top() - vec2(0.0, 4.0),
            Align2::LEFT_BOTTOM,
            locale.format("fab.sheet_caption", &[("number", &(i + 1).to_string())]),
            font.clone(),
            t.text_dim,
        );
        if let Some(fill) = app.fab.fills.get(i) {
            let mut mesh = egui::Mesh::default();
            for tri in fill {
                let first = mesh.vertices.len() as u32;
                for p in tri {
                    mesh.colored_vertex(to(p.x, y0 + p.y), t.profile_fill);
                }
                mesh.add_triangle(first, first + 1, first + 2);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
        for ring in read_rings(&sheet["cut_rings"]) {
            let pts: Vec<Pos2> = ring.iter().map(|p| to(p[0], y0 + p[1])).collect();
            if pts.len() >= 3 {
                painter.add(egui::Shape::closed_line(pts, cut));
            }
        }
        for line in read_rings(&sheet["engrave_lines"]) {
            let pts: Vec<Pos2> = line.iter().map(|p| to(p[0], y0 + p[1])).collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts, engrave));
            }
        }
    }
    // The legend, as the flat pattern has it.
    let mut y = rect.top() + 14.0;
    painter.text(pos2(rect.left() + 14.0, y), Align2::LEFT_TOP, app.tr("fab.view_sheets"), FontId::proportional(t.font_button), t.text);
    for (label, stroke) in [("fab.legend_cut", cut), ("fab.legend_engrave", engrave)] {
        y += 20.0;
        painter.line_segment([pos2(rect.left() + 14.0, y + 7.0), pos2(rect.left() + 38.0, y + 7.0)], Stroke::new(2.0, stroke.color));
        painter.text(pos2(rect.left() + 44.0, y), Align2::LEFT_TOP, app.tr(label), font.clone(), t.text_dim);
    }
    true
}

#[cfg(test)]
#[path = "dialogs_fab_tests.rs"]
mod tests;
