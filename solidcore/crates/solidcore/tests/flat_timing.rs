//! Timing of the flat parts' polygon offset and booleans (`flat::region`, iOverlay) against the
//! kernel's exact planar constructions on the same jobs. A measurement, not a test of results:
//! run by hand,
//! `cargo test --profile goldens -p solidcore --test flat_timing -- --ignored --nocapture`.
//!
//! Each job is run once to warm up, then [`RUNS`] times; the median is printed. The kernel runs
//! with SolidCraft's fixes (`crates/smlib/src/core.rs`, `fixes()`, copied below), as the app
//! runs it. Every job's area is printed beside its closed form, so a fast wrong answer shows.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)] // test helpers

use std::f64::consts::PI;
use std::time::Instant;

use solidcore::boolean::api_brep::{boolean_2d, Boolean2d, Boolean2dOutcome};
use solidcore::curve::curve::Curve;
use solidcore::curve::ellipse::Ellipse;
use solidcore::flat::region::{self, Join};
use solidcore::flat::{Region, Shape};
use solidcore::geom::axis2_placement::Axis2Placement;
use solidcore::kernel::UpstreamFixes;
use solidcore::mass::integrator::properties::brep_area;
use solidcore::math::{Point3, Vec3};
use solidcore::primitive::api_curves::create_line_segment;
use solidcore::primitive::api_primitives::create_offset_profile;
use solidcore::primitive::api_trim_ops::create_planar_faces;
use solidcore::{Brep, Kernel};

const RUNS: usize = 7;

/// SolidCraft's fixes, copied from `crates/smlib/src/core.rs`.
#[allow(clippy::too_many_lines)]
fn app_fixes() -> UpstreamFixes {
    let mut f = UpstreamFixes::default();
    f.fillet_retry_radius_as_first = true;
    f.fillet_refuse_rails_leaving_faces = true;
    f.fillet_refuse_face_consuming_blends = true;
    f.fillet_refuse_non_manifold_result = true;
    f.fillet_vertex_error_diagnostics = true;
    f.fillet_corner_ball_patch = true;
    f.fillet_chamfer_cone_on_arc = true;
    f.offset_excess_faces_after_merge = true;
    f.offset_clip_right_side_out = true;
    f.offset_extended_edges_mated = true;
    f.keep_operand_uv_trims = true;
    f.proto_vertex_miss_is_false = true;
    f.check_loop_vertex_tolerance = true;
    f.check_loop_projected_seam_ends = true;
    f.bspline_check_min_radius = true;
    f.oscillating_skips_failed_guesses = true;
    f.refine_cc_range_ends = true;
    f.flat_corners_test_bits = true;
    f.cylinder_rebuild_mean_radius = true;
    f.revolve_analytic_profile_faces = true;
    f.taper_tangent_corner_planes = true;
    f.revolve_nested_shells_as_cavities = true;
    f.revolve_holed_profile_void = true;
    f.join_touching_loops = true;
    f.uv_trim_reversal_on_surface = true;
    f.offset_resolve_crossing_loops = true;
    f.offset_walls_touch_refused = true;
    f.offset_analytic_mate_edges = true;
    f.nurb_cylinder_inside_out_frame = true;
    f.local_operation_refuses_loose_tangent_edges = true;
    f.torus_major_radius_from_origin = true;
    f.fillet_nx1_opposite_continuation_side_edge = true;
    f.torus_iso_circle_axis_from_midpoint = true;
    f.torus_keeps_inside_out = true;
    f.internal_pole_row_through_knot = true;
    f.extrusion_origin_scaled_zero = true;
    f.transform_scales_each_tolerance = true;
    f.transform_model_size_from_body_box = true;
    f.offset_collapsed_rounds_mated = true;
    f.offset_missing_surface_fails = true;
    f.fillet_nx1_end_rib_global_merge = true;
    f.uv_trim_corner_end_points_only = true;
    f.bspline_trim_preserves_uv_spans = true;
    f.proto_edge_self_loop_on_vertex_degenerate = true;
    f.slice_coincident_faces_sheet_side = true;
    f.boolean_outer_shell_first = true;
    f.edge_reversal_keeps_failed_edge = true;
    f.topology_failed_evaluations_propagate = true;
    f.heal_stage3_failures_propagate = true;
    f.cover_refuses_folded_continuation = true;
    f.boolean_vertex_by_best_crossing = true;
    f.boolean_cleanup_removes_slits = true;
    f.classify_edge_interval = true;
    f.ellipse_trim_seam_lines_span_cone_height = true;
    f.degenerate_proto_edge_in_one_vertex = true;
    f.new_edge_vertex_tolerance_covers_gap = true;
    f.torus_signed_gen_curve_interval = true;
    f.point_solve_keeps_triangle_guess = true;
    f.imprint_extends_edge_to_proto_vertex = true;
    f.fillet_point_step_per_point = true;
    f.analytic_copy_keeps_parameterization = true;
    f.precise_properties_body_length_scale = true;
    f.offset_extension_from_base = true;
    f.corner_blend_stays_inside_neighbours = true;
    f.fillet_cleanup_keeps_target_faces = true;
    f.fillet_nx1_tangent_side_edge = true;
    f.fillet_3x2_mixed_concave_side_edge = true;
    f.offset_stitch_at_body_tolerance = true;
    f.curve_contacts_at_curve_ends = true;
    f.shell_empty_cavity_fails = true;
    f.boolean_lists_virtual_surface_copies = true;
    f.ssi_tangent_contact_points = true;
    f.ssi_trace_tangent_curves_once = true;
    f.ssi_singular_start_inside_direction = true;
    f.found_torus_sphere_keep_tested_nurb = true;
    f.boolean_skips_faces_apart = true;
    f.uv_trim_checks_on_surface = true;
    f.local_operation_box_holds_new_surfaces = true;
    f.uv_trim_joins_drop_pieces = true;
    f.shell_edges_on_analytics_exact = true;
    f.ssi_pole_point_keeps_its_tangent = true;
    f.iso_curve_sliver_domain_is_degenerate = true;
    f.corner_blend_coons_patch_tangent_to_sides = true;
    f.fillet_patch_keeps_fillet_edges = true;
    f.fillet_corner_ball_center_from_axes = true;
    f.boolean2d_plane_covers_far_faces = true;
    f.boolean_splits_definition_across_split = true;
    f.curve_tight_box_interval_knots = true;
    f.fillet_refuse_overlapping_bands = true;
    f.ssi_tangent_curve_tracer = true;
    f.torus_check_reads_v_interval = true;
    f.torus_check_spindle_apex = true;
    f.local_operation_step_walls = true;
    f.edge_check_orders_tangent_faces = true;
    f.thicken_planar_side_walls = true;
    f.fillet_knife_edge_rounds = true;
    f.revolution_round_offset_collapses = true;
    f.fillet_chamfer_cone_stored_as_cone = true;
    f.offset_point_vertex_caps = true;
    f.proto_edge_splits_at_captured_vertices = true;
    f.stitch_solid_nests_cavities = true;
    f.local_operation_closes_walls_to_nothing = true;
    f.imprint_reuses_coincident_face_edge = true;
    f.boolean_new_face_closure_both_sides = true;
    f.linear_sweep_arc_is_cylinder = true;
    f.cylinder_plane_tangent_within_tolerance = true;
    f.boolean_wire_edge_keeps_faceuse_shell = true;
    f.fillet_nx1_tangent_corner_edge = true;
    f.loops_go_on_round_the_vertex = true;
    f.revolve_splits_curves_on_axis = true;
    f.cover_wraps_across_closed_seam = true;
    f.fillet_rails_meet_at_one_end_point = true;
    f.fillet_trace_reaches_rails = true;
    f.delete_vertex_joins_seam_iso_curve = true;
    f.box_face_failures_propagate = true;
    f.ellipse_adjust_step_interval_keeps_turns = true;
    f.fillet_rail_end_on_continued_surface = true;
    f.glue_vertices_joins_every_region = true;
    f.fillet_removal_takes_ring_fillets = true;
    f.fillet_trim_inside_by_best_dot = true;
    f.fillet_half_degree_crease = true;
    f.fillet_split_edge_keeps_near_half = true;
    f.curve_loop_normal_from_area = true;
    f.fillet_cliff_overflow = true;
    f.fillet_3x2_tangent_side_edge_roles = true;
    f.offset_extension_as_itself = true;
    f.fillet_mitre_concave_face_corners = true;
    f.fillet_mitre_gap_patch = true;
    f.fillet_run_on_past_far_end_only = true;
    f.good_drop_result_distances_in_order = true;
    f.face_tight_box_searches_trim = true;
    f.ssi_low_angle_start_on_probation = true;
    f.fillet_variable_span_follows_fillet = true;
    f.simple_coincidence_crossing_is_point = true;
    f.planar_path_sweep_statuses = true;
    f.planar_path_profile_placement_statuses = true;
    f.fillet_trim_status = true;
    f.perspective_sphere_offsets = true;
    f.plane_recognition_origin_after_axes = true;
    f.brep_edit_statuses_pass_on = true;
    f.fillet_run_on_bspline_trims = true;
    f.query_statuses_pass_on = true;
    f.sweep_statuses_pass_on = true;
    f.ssi_proto_topology_statuses_pass_on = true;
    f.analytic_inversion_answers_apart = true;
    f.fillet_statuses_pass_on = true;
    f.fillet_corner_per_vertex_fan = true;
    f.fillet_rail_trim_at_near_knot = true;
    f.offset_keep_cavity_regions = true;
    f.gap_samples_non_bspline_v_stride = true;
    f.imprint_skips_collapsed_definition = true;
    f.temporary_radius_setup_errors = true;
    f.ellipse_step_interval_checks = true;
    f.corner_patch_face_side_twist = true;
    f.boolean_union_refuses_solid_leak = true;
    f.curve_loop_start_after_previous_end = true;
    f.loop_use_reports_open_gaps = true;
    f.face_points_span_domain = true;
    f.drop_curve_v_domain_degeneracy = true;
    f.mirror_curve_keeps_euclidean_control_points = true;
    f.fillet_corner_variable_radius_rail_on_opposite_fillet = true;
    f.sphere_from_arcs_common_centre = true;
    f.dupin_bottom_hyperbola_axis = true;
    f.extent3d_contains_both_segment_endpoints = true;
    f.gap_samples_use_trimmed_spans = true;
    f.twist_simple_duw_sign = true;
    f.offset_swap_uv_extended_distance = true;
    f.circumscribe_axis_map_by_basis = true;
    f.validation_does_not_make_uv_trims = true;
    f.iso_curve_uv_line_both_params = true;
    f.analytic_corner_failure_is_false = true;
    f.proto_edge_split_upper_keeps_the_old_max = true;
    f.point_on_start_point_squared_tolerance = true;
    f.bspline_trim_keeps_rational_end_spans = true;
    f.fillet_exact_round_classes = true;
    f.cpoly_remove_poly_face_check_member = true;
    f.check_seam_crossing_unclosed = true;
    f.ascii_doubles_round_trip = true;
    f.transform_aligned_bbox_image = true;
    f.offset_edges_keep_source_tolerance = true;
    f.circle_silhouette_perspective_center = true;
    f.ellipse_convert_to_2d_uses_both_axes = true;
    f.circle_offset_antiparallel_normal = true;
    f.global_solver_identical_prefers_larger_when_maximizing = true;
    f.surface_continuity_checks_last_v_knot = true;
    f.surface_point_solver_propagates_bbox_failure = true;
    f.volume_lift_third_derivative_terms = true;
    f.stop_tracking_checks_own_callback = true;
    f.plane_reparameterize_moves_origin = true;
    f.crv_in_volume_mults_by_break_index = true;
    f.curve_bounded_surface_read_uv_type_and_implicit_outer = true;
    f.composite_curve_bbox_segment_intervals = true;
    f.knot_vector_valid_requires_span = true;
    f.projected_curve_transform_when_owned = true;
    f.curve_property_gs_branch_bounds = true;
    f.volume_representational_end_continuities = true;
    f.offset_open_edge_second_trim_point = true;
    f.close_loop_relates_only_coincident_edges = true;
    f.ssi_split_curves_at_singular_points = true;
    f.surface_parallel_to_vector_checks = true;
    f.fillet_round_ends_at_tangent_continuation = true;
    f.geometric_g2_continuity = true;
    f.bend_volume_bbox_contains_wide_angle_and_inf_u = true;
    f.bend_volume_trim_param_box_across_period = true;
    f.degen_param_tiebreak_uses_actual_param = true;
    f.local_surface_intersect_guards_max_boundary_retry = true;
    f.curve_sweep_closed_path_joins_ends = true;
    f.validate_tolerances_fixes_reported_gaps = true;
    f.edgeuse_convexity_propagates_step_off_failure = true;
    f.database_io_byte_swaps_read_boolean_and_u16 = true;
    f.cone_create_canonical_preserves_inside_out_and_checks_height = true;
    f.point_class_gap_3d_propagates_no_drop_error = true;
    f.offset_updates_result_tolerances = true;
    f.cone_iso_circle_at_axial_height = true;
    f.boolean_coincident_strip_drops_crossing = true;
    f.fillet_overflow_trim_at_creases = true;
    f.fillet_section_end_trim_at_creases = true;
    f.fillet_refuse_mixed_corner_crossing_rails = true;
    f.fillet_refuse_rounds_breaking_through = false;
    f.cone_unbounded_parameterized_as_nurb = true;
    f.ellipse_open_arc_keeps_end_angle = true;
    f.fillet_round_as_definition = true;
    f.fillet_overflow_crease_along_edge = true;
    f.plane_first_torus_section_exact = true;
    f.keep_exact_pcurves = true;
    f.planar_section_face_tolerance = true;
    f.face_swap_uv_transposes_trims = true;
    f.closed_smooth_pole_tangent_limit = true;
    f.fillet_round_ends_on_outline = false;
    f.fillet_setback_beside_curved_faces = true;
    f.fillet_chamfer_corner = true;
    f.brep_ray_classifies_current_trims = true;
    f
}

fn kernel() -> Kernel {
    let mut k = Kernel::default();
    k.fixes = app_fixes();
    k
}

/// The median time (s) of [`RUNS`] runs of `job` after a warm-up run, and the warm-up's
/// answer. Each run's input comes from `setup`, outside the time; the answer is dropped outside
/// it too.
fn timed<S, T>(mut setup: impl FnMut() -> S, mut job: impl FnMut(S) -> T) -> (f64, T) {
    let first = job(setup());
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let input = setup();
            let t = Instant::now();
            let answer = std::hint::black_box(job(input));
            let seconds = t.elapsed().as_secs_f64();
            drop(answer);
            seconds
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[RUNS / 2], first)
}

fn report(case: &str, how: &str, seconds: f64, area: f64, exact: f64) {
    println!(
        "TIMING {case} | {how} | median {:.3} ms of {RUNS} | area {area:.6} | closed form {exact:.6} | off {:.2e}",
        seconds * 1e3,
        (area - exact).abs()
    );
}

fn line(a: [f64; 2], b: [f64; 2]) -> Curve {
    Curve::from(
        create_line_segment(Point3::new(a[0], a[1], 0.0), Point3::new(b[0], b[1], 0.0)).unwrap(),
    )
}

fn sides(pts: &[[f64; 2]]) -> Vec<Curve> {
    (0..pts.len())
        .map(|i| line(pts[i], pts[(i + 1) % pts.len()]))
        .collect()
}

/// A planar face bounded by the polygon `pts`.
fn face(k: &mut Kernel, pts: &[[f64; 2]]) -> Brep {
    create_planar_faces(k, &sides(pts)).unwrap()
}

/// A planar face bounded by the circle of radius `r` about `c`.
fn disk(k: &mut Kernel, c: [f64; 2], r: f64) -> Brep {
    let frame = Axis2Placement::new(Point3::new(c[0], c[1], 0.0), Vec3::X, Vec3::Y);
    let circle = Ellipse::create_canonical_circle(k, &frame, r, None, None).unwrap();
    create_planar_faces(k, &[Curve::from(circle)]).unwrap()
}

/// The faces united in balanced rounds, or the first boolean's failure.
fn unite(k: &mut Kernel, mut faces: Vec<Brep>) -> Result<Brep, String> {
    while faces.len() > 1 {
        let mut next = Vec::new();
        let mut round = faces.into_iter();
        while let Some(a) = round.next() {
            match round.next() {
                Some(b) => {
                    match boolean_2d(k, a, b, Boolean2d::Union).map_err(|e| e.to_string())? {
                        Boolean2dOutcome::Result(r) => next.push(r),
                        Boolean2dOutcome::Unassigned { .. } => return Err("operands left".into()),
                    }
                }
                None => next.push(a),
            }
        }
        faces = next;
    }
    faces.pop().ok_or_else(|| "no face".to_string())
}

fn area(k: &Kernel, mut b: Brep) -> f64 {
    brep_area(k, &mut b, 1e-9).unwrap()
}

/// The regular `n`-gon of circumradius `r` about the origin, counter-clockwise.
fn ngon(n: usize, r: f64) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| {
            let a = 2.0 * PI * i as f64 / n as f64;
            [r * a.cos(), r * a.sin()]
        })
        .collect()
}

/// The 160-sided polygon (circumradius 50 mm) grown by half a 0.15 mm kerf with sharp (mitred)
/// corners, as `flat::finish::kerf` grows a part: the kernel's exact offset of the profile
/// (its side curves made outside the time), the kernel's exact 2D union of the polygon with a
/// band along each side and a mitre kite at each corner (faces made outside the time), and the
/// polygon offset of `flat::region`.
#[test]
#[ignore = "a measurement, run by hand"]
fn kerf_offset_of_a_160_sided_polygon() {
    let (n, r, d) = (160, 50.0, 0.075);
    let pts = ngon(n, r);
    let side = 2.0 * r * (PI / n as f64).sin();
    let apothem = r * (PI / n as f64).cos();
    let exact = n as f64 * side * apothem / 2.0
        + n as f64 * side * d
        + n as f64 * d * d * (PI / n as f64).tan();
    let case = "kerf offset, 160-gon";

    // the kernel's offset of the profile: the side (1 left, 2 right) that grows it
    let curves = sides(&pts);
    let refs: Vec<Option<&Curve>> = curves.iter().map(Some).collect();
    let offset =
        |k: &mut Kernel, t: u64| create_offset_profile(k, &refs, d, t, false, false).unwrap();
    let grown = |t: u64| {
        let mut k = kernel();
        let b = offset(&mut k, t);
        area(&k, b)
    };
    let out = if grown(1) > grown(2) { 1 } else { 2 };
    let (s, (k, b)) = timed(kernel, |mut k| {
        let b = offset(&mut k, out);
        (k, b)
    });
    report(case, "kernel create_offset_profile", s, area(&k, b), exact);

    // the kernel's 2D union of the polygon, a band along each side and a kite at each corner
    // out to the mitre point: bands that start on the side (sharing its edge), and bands that
    // straddle it (overlapping the polygon instead)
    let normal = |i: usize| {
        let (p, q) = (pts[i], pts[(i + 1) % n]);
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let l = dx.hypot(dy);
        [dy / l, -dx / l]
    };
    for (how, inner) in [("on the side", 0.0), ("straddling the side", -d)] {
        let setup = || {
            let mut k = kernel();
            let mut faces = vec![face(&mut k, &pts)];
            let at = |p: [f64; 2], m: [f64; 2], t: f64| [p[0] + t * m[0], p[1] + t * m[1]];
            for i in 0..n {
                let (p, q, m) = (pts[i], pts[(i + 1) % n], normal(i));
                faces.push(face(
                    &mut k,
                    &[at(p, m, inner), at(q, m, inner), at(q, m, d), at(p, m, d)],
                ));
                let m2 = normal((i + 1) % n);
                let c = 1.0 + m[0] * m2[0] + m[1] * m2[1];
                let tip = [q[0] + d * (m[0] + m2[0]) / c, q[1] + d * (m[1] + m2[1]) / c];
                faces.push(face(&mut k, &[q, at(q, m, d), tip, at(q, m2, d)]));
            }
            (k, faces)
        };
        let label = format!("kernel boolean_2d union of bands {how}");
        let (mut k, faces) = setup();
        let t = Instant::now();
        match unite(&mut k, faces) {
            Ok(_) => {
                let (s, (k, b)) = timed(setup, |(mut k, faces)| {
                    let b = unite(&mut k, faces).unwrap();
                    (k, b)
                });
                report(case, &label, s, area(&k, b), exact);
            }
            Err(e) => println!(
                "TIMING {case} | {label} | failed after {:.3} ms: {e}",
                t.elapsed().as_secs_f64() * 1e3
            ),
        }
    }

    // the polygon offset
    let shape = || -> Region {
        vec![Shape {
            outer: pts.clone(),
            holes: Vec::new(),
        }]
    };
    let (s, grown) = timed(shape, |r| region::offset(&r, d, Join::Miter));
    report(case, "flat::region offset", s, region::area(&grown), exact);
}

/// A realistic flat part: a 200 x 120 mm plate with ten slots 3.2 x 20 mm cut in from one edge
/// and four 6 mm holes, the cutters united and taken from the plate: the kernel's exact 2D
/// booleans (circles exact; faces made outside the time) and the polygon booleans of
/// `flat::region` (circles within the slicing's 0.01 mm chord, made outside the time).
#[test]
#[ignore = "a measurement, run by hand"]
fn a_plate_with_ten_slots_and_four_holes() {
    let (w, h, slot_w, slot_d, hole_r) = (200.0, 120.0, 3.2, 20.0, 3.0);
    let xs: Vec<f64> = (0..10).map(|i| 20.0 + 160.0 * f64::from(i) / 9.0).collect();
    let holes = [[10.0, 10.0], [190.0, 10.0], [10.0, 110.0], [190.0, 110.0]];
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    // each slot reaches 1 mm past the edge, so it opens it
    let slot = |x: f64| rect(x - slot_w / 2.0, h - slot_d, x + slot_w / 2.0, h + 1.0);
    let exact = w * h - 10.0 * slot_w * slot_d - 4.0 * PI * hole_r * hole_r;
    let case = "plate, 10 slots, 4 holes";

    let setup = || {
        let mut k = kernel();
        let mut cutters: Vec<Brep> = xs.iter().map(|&x| face(&mut k, &slot(x))).collect();
        cutters.extend(holes.iter().map(|&c| disk(&mut k, c, hole_r)));
        let plate = face(&mut k, &rect(0.0, 0.0, w, h));
        (k, plate, cutters)
    };
    let (s, (k, b)) = timed(setup, |(mut k, plate, cutters)| {
        let cut = unite(&mut k, cutters).unwrap();
        let b = match boolean_2d(&mut k, plate, cut, Boolean2d::Difference).unwrap() {
            Boolean2dOutcome::Result(r) => r,
            Boolean2dOutcome::Unassigned { .. } => panic!("2D boolean left its operands"),
        };
        (k, b)
    });
    report(case, "kernel boolean_2d", s, area(&k, b), exact);

    let setup = || {
        let mut cutters: Vec<Region> = xs
            .iter()
            .map(|&x| {
                let [p, _, q, _] = slot(x);
                region::rect(p[0], p[1], q[0], q[1])
            })
            .collect();
        cutters.extend(holes.iter().map(|&c| region::disk(c, hole_r, 0.01)));
        (region::rect(0.0, 0.0, w, h), cutters)
    };
    let (s, part) = timed(setup, |(plate, cutters)| {
        region::difference(&plate, &region::union_all(&cutters))
    });
    report(case, "flat::region", s, region::area(&part), exact);
}
