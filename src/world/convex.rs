//! The collision shapes of wedges and corner wedges (world/query.rs `Shape::Wedge`, `Shape::CornerWedge`), as a playing client's
//! queries meet them (measured, Studio 0.747, a client and a server: tools/parity/probes/query_wedges.luau), and the convex
//! sweeps and overlaps of boxes against them.
//!
//! - **A wedge** (a WedgePart, or a Part whose Shape is Wedge) is its box without the top front edge: its bottom (-Y) and back
//!   (+Z) faces whole, its sides triangles, its slope rising from the bottom front edge to the top back one (outward normal
//!   (0, Z, -Y) over its length for half extents X, Y, Z: (0, 0.8, -0.6) for a 4 x 6 x 8 wedge).
//! - **A corner wedge** (a CornerWedgePart, or a Part whose Shape is CornerWedge) is its bottom face and the corner at (+X, +Y, -Z):
//!   its right (+X) and front (-Z) faces triangles, two slopes from the apex down to the bottom's left and back edges (normals
//!   (-Y, X, 0) and (0, Z, Y) over their lengths).
//! - GetPartsInPart, Raycast, Blockcast, Spherecast, Shapecast, ArePartsTouchingOthers, GetPartsObscuringTarget and the
//!   PlayerMouse meet these shapes; GetPartBoundsInBox, GetPartBoundsInRadius and the Region3 queries meet the part's box
//!   (measured: a point above a slope inside the box is found by the first two, a region beside a turned wedge's corners but in
//!   its box's world box by the Region3 query), as the spatial grid files it.
//! - A ray enters through the face it crosses last among those it enters, the first of those in [`Hull`]'s order when two are
//!   crossed at once (measured: a ray down onto a corner wedge's ridge takes the left slope); a ray lying in a face's plane misses
//!   (along a wedge's side or back face, down a corner wedge's right face), as does one that only touches an edge (up a wedge's
//!   bottom front edge). Not matched: a ray that only touches a corner wedge's apex from the side hits it on a client.
//! - A sphere meets the shape grown by its radius with rounded edges and corners, in doubles (as query.rs sweep_sphere_box does for
//!   a box). A client's own contacts carry iteration errors of up to 2e-5 studs in the distance and 3e-4 in an edge's normal.
//! - A box meets it by separating axes (both shapes' face normals and their edges' cross products): the distance and normal are
//!   exact; the position is worked out from the two shapes' touching features ([`contact`]). A client's position along a contact
//!   edge or face varies with its iterations (a cube on a slope: 5 of 8 positions match).

use glam::{Affine3A, DVec3, Mat3, Vec3};

/// Which of the two shapes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wedge,
    CornerWedge,
}

/// A face of a [`Hull`]: its outward unit normal, its plane (`normal . p <= offset` inside) and its corners, counter-clockwise seen
/// from outside
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub normal: Vec3,
    pub offset: f32,
    corners: [u8; 4],
    count: u8,
    /// One of its box's faces (not a slope): a ray lying in its plane misses
    boxed: bool,
}

impl Face {
    fn corners(&self) -> &[u8] {
        &self.corners[..self.count as usize]
    }
}

/// A wedge's or a corner wedge's hull in its part's frame (its centre at the origin): its corners and faces. The faces' order is
/// the order a ray's tie between two faces is decided in.
#[derive(Clone, Copy, Debug)]
pub struct Hull {
    pub kind: Kind,
    pub half: Vec3,
    vertices: [Vec3; 6],
    vertex_count: u8,
    faces: [Face; 5],
}

/// The edges of each kind's hull, as corner pairs
const WEDGE_EDGES: [(u8, u8); 9] = [(0, 1), (1, 2), (2, 3), (3, 0), (0, 4), (1, 5), (2, 5), (3, 4), (4, 5)];
const CORNER_EDGES: [(u8, u8); 8] = [(0, 1), (1, 2), (2, 3), (3, 0), (0, 4), (1, 4), (2, 4), (3, 4)];

impl Hull {
    /// The hull of `kind` with half extents `half`
    pub fn new(kind: Kind, half: Vec3) -> Hull {
        let Vec3 { x, y, z } = half;
        let face = |normal: Vec3, offset: f32, corners: &[u8]| {
            let mut array = [0u8; 4];
            array[..corners.len()].copy_from_slice(corners);
            Face { normal, offset, corners: array, count: corners.len() as u8, boxed: offset != 0.0 }
        };
        let unit = |v: Vec3| v.normalize_or(Vec3::Y);
        match kind {
            // corners: 0 (-x,-y,-z), 1 (x,-y,-z), 2 (x,-y,z), 3 (-x,-y,z), 4 (-x,y,z), 5 (x,y,z)
            Kind::Wedge => Hull {
                kind,
                half,
                vertices: [Vec3::new(-x, -y, -z), Vec3::new(x, -y, -z), Vec3::new(x, -y, z), Vec3::new(-x, -y, z), Vec3::new(-x, y, z), Vec3::new(x, y, z)],
                vertex_count: 6,
                faces: [
                    face(Vec3::X, x, &[1, 5, 2]),
                    face(Vec3::NEG_X, x, &[0, 3, 4]),
                    face(Vec3::NEG_Y, y, &[0, 1, 2, 3]),
                    face(Vec3::Z, z, &[3, 2, 5, 4]),
                    face(unit(Vec3::new(0.0, z, -y)), 0.0, &[0, 4, 5, 1]),
                ],
            },
            // corners: 0 (-x,-y,-z), 1 (x,-y,-z), 2 (x,-y,z), 3 (-x,-y,z), 4 the apex (x,y,-z)
            Kind::CornerWedge => Hull {
                kind,
                half,
                vertices: [Vec3::new(-x, -y, -z), Vec3::new(x, -y, -z), Vec3::new(x, -y, z), Vec3::new(-x, -y, z), Vec3::new(x, y, -z), Vec3::ZERO],
                vertex_count: 5,
                faces: [
                    face(Vec3::X, x, &[1, 4, 2]),
                    face(Vec3::NEG_Y, y, &[0, 1, 2, 3]),
                    face(Vec3::NEG_Z, z, &[0, 4, 1]),
                    face(unit(Vec3::new(-y, x, 0.0)), 0.0, &[0, 3, 4]),
                    face(unit(Vec3::new(0.0, z, y)), 0.0, &[3, 2, 4]),
                ],
            },
        }
    }

    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices[..self.vertex_count as usize]
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    /// Its corners in doubles
    fn dvertices(&self) -> ([DVec3; 6], usize) {
        let mut out = [DVec3::ZERO; 6];
        for (to, from) in out.iter_mut().zip(self.vertices()) {
            *to = from.as_dvec3();
        }
        (out, self.vertex_count as usize)
    }

    fn edges(&self) -> &'static [(u8, u8)] {
        match self.kind {
            Kind::Wedge => &WEDGE_EDGES,
            Kind::CornerWedge => &CORNER_EDGES,
        }
    }

    /// Where a ray from `o` along `d` (its length the reach) first enters it: (fraction of `d`, the face's normal), or None. It
    /// enters through the last face it crosses into, the first in the faces' order of those crossed at once (within a millionth);
    /// one lying in the plane of one of its box's faces misses, one lying in a slope's plane is inside it there (measured: along a
    /// wedge's slope through its centre it enters the side face it reaches); one starting inside or on it, only touching it or
    /// reaching it only at its end misses. Worked in doubles, the fraction a float.
    pub fn ray(&self, o: Vec3, d: Vec3) -> Option<(f32, Vec3)> {
        let (o, d) = (o.as_dvec3(), d.as_dvec3());
        let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
        let mut normal = Vec3::ZERO;
        for face in &self.faces {
            let n = face.normal.as_dvec3();
            let distance = n.dot(o) - f64::from(face.offset);
            let speed = n.dot(d);
            if speed.abs() < 1e-12 {
                if distance > 0.0 || (face.boxed && distance >= 0.0) {
                    return None;
                }
                continue;
            }
            let t = -distance / speed;
            if speed < 0.0 {
                if t > enter + 1e-6 * t.abs().max(1.0) || !enter.is_finite() {
                    enter = t;
                    normal = face.normal;
                }
            } else if t < exit {
                exit = t;
            }
        }
        (enter > 0.0 && enter < 1.0 && enter < exit - 1e-9 * exit.abs().max(1.0)).then_some((enter as f32, normal))
    }

    /// The point of it nearest `p`, in doubles, and whether `p` is inside (then `p` itself)
    fn closest(&self, p: DVec3) -> (DVec3, bool) {
        let (all, count) = self.dvertices();
        let vertices = &all[..count];
        if self.faces.iter().all(|face| face.normal.as_dvec3().dot(p) <= f64::from(face.offset)) {
            return (p, true);
        }
        let mut best = (f64::INFINITY, p);
        let mut keep = |q: DVec3| {
            let distance = (q - p).length_squared();
            if distance < best.0 {
                best = (distance, q);
            }
        };
        for face in &self.faces {
            let n = face.normal.as_dvec3();
            let q = p - n * (n.dot(p) - f64::from(face.offset));
            if inside_face(vertices, face.corners(), n, q) {
                keep(q);
            }
        }
        for &(a, b) in self.edges() {
            keep(closest_on_segment(vertices[a as usize], vertices[b as usize], p));
        }
        for &v in vertices {
            keep(v);
        }
        (best.1, false)
    }

    /// How far `p` is from it: positive outside, the negative depth inside (to the nearest face)
    pub fn signed_distance(&self, p: Vec3) -> f32 {
        let (q, inside) = self.closest(p.as_dvec3());
        if inside {
            -self.faces.iter().map(|face| face.offset - face.normal.dot(p)).fold(f32::INFINITY, f32::min)
        } else {
            (q - p.as_dvec3()).length() as f32
        }
    }

    /// A sphere of `radius` swept from `o` along `d` against it: the hull grown by the radius with rounded edges and corners, in
    /// doubles. The first contact's fraction and normal (from the hull to the sphere's centre); None when it starts touching or
    /// overlapping, or touches only at the end.
    pub fn sweep_sphere(&self, o: Vec3, d: Vec3, radius: f32) -> Option<(f32, Vec3)> {
        let (o, d, r) = (o.as_dvec3(), d.as_dvec3(), f64::from(radius));
        let (near, inside) = self.closest(o);
        if inside || (near - o).length_squared() <= r * r {
            return None;
        }
        let (all, count) = self.dvertices();
        let vertices = &all[..count];
        let mut best: Option<f64> = None;
        let mut keep = |t: f64| {
            if t > 0.0 && t < 1.0 && best.is_none_or(|b| t < b) {
                best = Some(t);
            }
        };
        // the faces, moved out by the radius
        for face in &self.faces {
            let n = face.normal.as_dvec3();
            let speed = n.dot(d);
            if speed >= 0.0 {
                continue;
            }
            let t = (f64::from(face.offset) + r - n.dot(o)) / speed;
            let touch = o + d * t - n * r;
            if inside_face(vertices, face.corners(), n, touch) {
                keep(t);
            }
        }
        // the edges (a cylinder of the radius about each)
        for &(a, b) in self.edges() {
            let (a, b) = (vertices[a as usize], vertices[b as usize]);
            let axis = (b - a).normalize();
            let (from, along) = (o - a, d);
            let (from_across, along_across) = (from - axis * from.dot(axis), along - axis * along.dot(axis));
            let qa = along_across.length_squared();
            if qa <= 0.0 {
                continue;
            }
            let qb = from_across.dot(along_across);
            let qc = from_across.length_squared() - r * r;
            let discriminant = qb * qb - qa * qc;
            if discriminant < 0.0 {
                continue;
            }
            let t = (-qb - discriminant.sqrt()) / qa;
            let s = (from + along * t).dot(axis);
            if (0.0..=(b - a).length()).contains(&s) {
                keep(t);
            }
        }
        // the corners (a sphere of the radius about each)
        let qa = d.length_squared();
        for &v in vertices {
            let from = o - v;
            let qb = from.dot(d);
            let discriminant = qb * qb - qa * (from.length_squared() - r * r);
            if qa > 0.0 && discriminant >= 0.0 {
                keep((-qb - discriminant.sqrt()) / qa);
            }
        }
        let t = best?;
        let center = o + d * t;
        let (touch, _) = self.closest(center);
        Some((t as f32, (center - touch).normalize_or(DVec3::Y).as_vec3()))
    }
}

/// Whether `q` (on the plane of the face with normal `n` and these corners) is inside the face or on its edges
fn inside_face(vertices: &[DVec3], corners: &[u8], n: DVec3, q: DVec3) -> bool {
    let count = corners.len();
    (0..count).all(|i| {
        let (a, b) = (vertices[corners[i] as usize], vertices[corners[(i + 1) % count] as usize]);
        (b - a).cross(q - a).dot(n) >= -1e-9 * (1.0 + (b - a).length_squared())
    })
}

fn closest_on_segment(a: DVec3, b: DVec3, p: DVec3) -> DVec3 {
    let ab = b - a;
    let length = ab.length_squared();
    if length <= 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / length).clamp(0.0, 1.0)
}

/// A convex shape in the world for the separating-axis tests: a box (a Block's, or the box a ball or cylinder is cast as) or a
/// wedge's or corner wedge's hull, with its corners, its faces' normals and its edges' directions
#[derive(Clone, Copy, Debug)]
pub struct Polytope {
    pub center: DVec3,
    /// Its frame's axes (unit) and its box's half extents (a hull's box)
    pub axes: [DVec3; 3],
    pub half: DVec3,
    vertices: [DVec3; 8],
    vertex_count: u8,
    normals: [DVec3; 5],
    normal_count: u8,
    edges: [DVec3; 6],
    edge_count: u8,
}

impl Polytope {
    /// The box of half extents `half` (any sign: taken as its size) in `frame` (unit axes)
    pub fn of_box(frame: &Affine3A, half: Vec3) -> Polytope {
        let axes = axes_of_frame(frame);
        let center = Vec3::from(frame.translation).as_dvec3();
        let half = half.abs().as_dvec3();
        let mut vertices = [DVec3::ZERO; 8];
        for (corner, vertex) in vertices.iter_mut().enumerate() {
            let sign = |bit: usize| if corner & bit == 0 { -1.0 } else { 1.0 };
            *vertex = center + axes[0] * (sign(1) * half.x) + axes[1] * (sign(2) * half.y) + axes[2] * (sign(4) * half.z);
        }
        let mut normals = [DVec3::ZERO; 5];
        normals[..3].copy_from_slice(&axes);
        let mut edges = [DVec3::ZERO; 6];
        edges[..3].copy_from_slice(&axes);
        Polytope { center, axes, half, vertices, vertex_count: 8, normals, normal_count: 3, edges, edge_count: 3 }
    }

    /// `hull` in `frame` (unit axes)
    pub fn of_hull(frame: &Affine3A, hull: &Hull) -> Polytope {
        let axes = axes_of_frame(frame);
        let center = Vec3::from(frame.translation).as_dvec3();
        let mut vertices = [DVec3::ZERO; 8];
        for (vertex, local) in vertices.iter_mut().zip(hull.vertices()) {
            let local = local.as_dvec3();
            *vertex = center + axes[0] * local.x + axes[1] * local.y + axes[2] * local.z;
        }
        let turn = |v: Vec3| {
            let v = v.as_dvec3();
            axes[0] * v.x + axes[1] * v.y + axes[2] * v.z
        };
        // (the distinct face normals, and the distinct edge directions)
        let mut normals = [DVec3::ZERO; 5];
        let mut normal_count = 0;
        for face in hull.faces() {
            let n = turn(face.normal).normalize();
            if !normals[..normal_count].iter().any(|m| m.cross(n).length_squared() < 1e-14) {
                normals[normal_count] = n;
                normal_count += 1;
            }
        }
        let mut edges = [DVec3::ZERO; 6];
        let mut edge_count = 0;
        for &(a, b) in hull.edges() {
            let e = (vertices[b as usize] - vertices[a as usize]).normalize_or_zero();
            if edge_count < 6 && !edges[..edge_count].iter().any(|f| f.cross(e).length_squared() < 1e-14) {
                edges[edge_count] = e;
                edge_count += 1;
            }
        }
        Polytope { center, axes, half: hull.half.as_dvec3(), vertices, vertex_count: hull.vertices().len() as u8, normals, normal_count: normal_count as u8, edges, edge_count: edge_count as u8 }
    }

    pub fn vertices(&self) -> &[DVec3] {
        &self.vertices[..self.vertex_count as usize]
    }

    /// Its extent along `axis`: the least and most of its corners' projections
    pub fn interval(&self, axis: DVec3) -> (f64, f64) {
        self.vertices().iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            let at = v.dot(axis);
            (lo.min(at), hi.max(at))
        })
    }

    /// The same shape moved by `by`
    pub fn moved(&self, by: DVec3) -> Polytope {
        let mut moved = *self;
        moved.center += by;
        for vertex in &mut moved.vertices[..self.vertex_count as usize] {
            *vertex += by;
        }
        moved
    }

    /// Half its extent along `axis` (a box's projected half extents)
    pub fn radius(&self, axis: DVec3) -> f64 {
        (0..3).map(|i| self.axes[i].dot(axis).abs() * self.half[i]).sum()
    }
}

/// A frame's axes, made unit, in doubles
fn axes_of_frame(frame: &Affine3A) -> [DVec3; 3] {
    [Vec3::from(frame.matrix3.x_axis), Vec3::from(frame.matrix3.y_axis), Vec3::from(frame.matrix3.z_axis)].map(|axis| axis.as_dvec3().normalize_or(DVec3::X))
}

/// The separating axes of two convex shapes: `a`'s face normals, `b`'s, then their edges' cross products (unit), into `out`
fn axes_of(a: &Polytope, b: &Polytope, out: &mut [DVec3; 48]) -> usize {
    let mut count = 0;
    for n in a.normals[..a.normal_count as usize].iter().chain(&b.normals[..b.normal_count as usize]) {
        out[count] = *n;
        count += 1;
    }
    for x in &a.edges[..a.edge_count as usize] {
        for y in &b.edges[..b.edge_count as usize] {
            let cross = x.cross(*y);
            if cross.length_squared() > 1e-12 {
                out[count] = cross.normalize();
                count += 1;
            }
        }
    }
    count
}

/// Whether two convex shapes overlap (touching counts)
pub fn overlap(a: &Polytope, b: &Polytope) -> bool {
    let mut axes = [DVec3::ZERO; 48];
    let count = axes_of(a, b, &mut axes);
    axes[..count].iter().all(|axis| {
        let (a0, a1) = a.interval(*axis);
        let (b0, b1) = b.interval(*axis);
        a0 <= b1 && b0 <= a1
    })
}

/// How far two convex shapes overlap: the least overlap along their separating axes, negative when apart
pub fn depth(a: &Polytope, b: &Polytope) -> f32 {
    let mut axes = [DVec3::ZERO; 48];
    let count = axes_of(a, b, &mut axes);
    axes[..count]
        .iter()
        .map(|axis| {
            let (a0, a1) = a.interval(*axis);
            let (b0, b1) = b.interval(*axis);
            a1.min(b1) - a0.max(b0)
        })
        .fold(f64::INFINITY, f64::min) as f32
}

/// `a` moving along `direction` against `b`: the fraction of the direction at first contact and the contact normal (from `b`
/// towards `a`); None when it never touches, overlaps at the start, or touches only at the end.
pub fn sweep(a: &Polytope, direction: Vec3, b: &Polytope) -> Option<(f64, DVec3)> {
    let direction = direction.as_dvec3();
    let mut axes = [DVec3::ZERO; 48];
    let count = axes_of(a, b, &mut axes);
    let (mut enter, mut exit) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut normal = DVec3::ZERO;
    for axis in &axes[..count] {
        let (a0, a1) = a.interval(*axis);
        let (b0, b1) = b.interval(*axis);
        let speed = direction.dot(*axis);
        if speed.abs() < 1e-12 {
            if a0 > b1 || a1 < b0 {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((b0 - a1) / speed, (b1 - a0) / speed);
        let (near, far) = if t0 < t1 { (t0, t1) } else { (t1, t0) };
        if near > enter + 1e-9 * near.abs().max(1.0) || !enter.is_finite() {
            enter = near;
            // (+ 0: no negative zero in a component)
            normal = if speed > 0.0 { -*axis } else { *axis } + DVec3::ZERO;
        }
        exit = exit.min(far);
        if enter > exit {
            return None;
        }
    }
    (enter > 0.0 && enter < 1.0).then_some((enter, normal))
}

/// Where `a` (at its contact) touches `b` across `normal` (from `b` to `a`): the part of `b`'s touching feature (its corners
/// furthest along the normal) that `a`'s (its corners furthest against it) covers, a point of it chosen as a client's box contact
/// is (query.rs box_contact): along each of `b`'s box axes lying in the contact plane, the point dividing the two centres as the
/// shapes' half extents do, kept within the touching part; a corner where either feature is one.
pub fn contact(a: &Polytope, normal: DVec3, b: &Polytope) -> Vec3 {
    let feature = |shape: &Polytope, sign: f64| -> ([DVec3; 8], usize) {
        let best = shape.vertices().iter().map(|v| sign * v.dot(normal)).fold(f64::NEG_INFINITY, f64::max);
        let mut out = [DVec3::ZERO; 8];
        let mut count = 0;
        for v in shape.vertices() {
            if sign * v.dot(normal) >= best - 1e-6 * (1.0 + best.abs()) {
                out[count] = *v;
                count += 1;
            }
        }
        (out, count)
    };
    let (theirs, their_count) = feature(b, 1.0);
    let (mine, my_count) = feature(a, -1.0);
    let plane = theirs[0].dot(normal);
    if their_count == 1 {
        return theirs[0].as_vec3();
    }
    if my_count == 1 {
        return (mine[0] - normal * (mine[0].dot(normal) - plane)).as_vec3();
    }
    // the contact plane's basis, the two features in it, and their common part
    let u = normal.any_orthonormal_vector();
    let v = normal.cross(u);
    let flat = |p: DVec3| glam::DVec2::new(p.dot(u), p.dot(v));
    let theirs = hull_2d(theirs[..their_count].iter().map(|p| flat(*p)));
    let mine = hull_2d(mine[..my_count].iter().map(|p| flat(*p)));
    let common = clip_2d(&mine, &theirs);
    let lift = |q: glam::DVec2| u * q.x + v * q.y + normal * plane;
    if common.is_empty() {
        return lift(centroid_2d(&theirs)).as_vec3();
    }
    // a point of the common part: its centroid, then along each of b's box axes in the plane the dividing point
    let mut point = centroid_2d(&common);
    for (axis, half) in b.axes.iter().zip(b.half.to_array()) {
        if axis.dot(normal).abs() > 1e-6 {
            continue;
        }
        let offset = (a.center - b.center).dot(*axis);
        let ha = a.radius(*axis);
        let target = b.center.dot(*axis) + if ha + half > 0.0 { offset - ha * offset / (ha + half) } else { offset };
        let along = flat(*axis).normalize_or_zero();
        let now = lift(point).dot(*axis);
        point += along * (target - now);
        point = nearest_in_2d(&common, point);
    }
    lift(point).as_vec3()
}

/// The convex hull of a few points in a plane, counter-clockwise (a segment or a point when they are in a line or one)
fn hull_2d(points: impl Iterator<Item = glam::DVec2>) -> Vec<glam::DVec2> {
    let mut points: Vec<glam::DVec2> = points.collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| a.distance_squared(*b) < 1e-14);
    if points.len() < 3 {
        return points;
    }
    let cross = |o: glam::DVec2, a: glam::DVec2, b: glam::DVec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<glam::DVec2> = Vec::new();
    for p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 1e-9 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<glam::DVec2> = Vec::new();
    for p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 1e-9 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The part of `subject` (a point, segment or convex polygon) inside `clip` (likewise)
fn clip_2d(subject: &[glam::DVec2], clip: &[glam::DVec2]) -> Vec<glam::DVec2> {
    match (subject.len(), clip.len()) {
        (_, 0) | (0, _) => Vec::new(),
        (_, 1) => if nearest_in_2d(subject, clip[0]).distance_squared(clip[0]) < 1e-8 { vec![clip[0]] } else { Vec::new() },
        (1, _) => if nearest_in_2d(clip, subject[0]).distance_squared(subject[0]) < 1e-8 { vec![subject[0]] } else { Vec::new() },
        (2, 2) => {
            // two segments: their overlap when in a line, else where they cross
            let (a, b, c, d) = (subject[0], subject[1], clip[0], clip[1]);
            let (r, s) = (b - a, d - c);
            let denominator = r.perp_dot(s);
            if denominator.abs() < 1e-9 * (r.length() * s.length()).max(1e-12) {
                let along = r.normalize_or_zero();
                let (lo, hi) = (((c - a).dot(along)).min((d - a).dot(along)), ((c - a).dot(along)).max((d - a).dot(along)));
                let (lo, hi) = (lo.max(0.0), hi.min(r.length()));
                if lo > hi + 1e-6 {
                    return Vec::new();
                }
                return vec![a + along * lo, a + along * hi];
            }
            let t = (c - a).perp_dot(s) / denominator;
            let w = (c - a).perp_dot(r) / denominator;
            if (-1e-6..=1.0 + 1e-6).contains(&t) && (-1e-6..=1.0 + 1e-6).contains(&w) { vec![a + r * t] } else { Vec::new() }
        }
        (_, 2) => clip_2d(clip, subject),
        _ => {
            // Sutherland-Hodgman against each of the clip polygon's edges (a segment subject stays a segment)
            let mut out = subject.to_vec();
            let count = clip.len();
            for i in 0..count {
                let (p, q) = (clip[i], clip[(i + 1) % count]);
                let inside = |x: glam::DVec2| (q - p).perp_dot(x - p) >= -1e-6;
                let input = std::mem::take(&mut out);
                if input.is_empty() {
                    break;
                }
                let n = input.len();
                let closed = n > 2;
                for j in 0..n {
                    let (cur, next) = (input[j], input[(j + 1) % n]);
                    if !closed && j == n - 1 {
                        if inside(cur) {
                            out.push(cur);
                        }
                        break;
                    }
                    let (cin, nin) = (inside(cur), inside(next));
                    if cin {
                        out.push(cur);
                    }
                    if cin != nin {
                        let e = next - cur;
                        let denominator = (q - p).perp_dot(e);
                        if denominator.abs() > 1e-12 {
                            let t = (q - p).perp_dot(p - cur) / -denominator;
                            out.push(cur + e * t.clamp(0.0, 1.0));
                        }
                    }
                }
            }
            out
        }
    }
}

fn centroid_2d(points: &[glam::DVec2]) -> glam::DVec2 {
    points.iter().copied().sum::<glam::DVec2>() / points.len().max(1) as f64
}

/// The point of `shape` (a point, segment or convex polygon, counter-clockwise) nearest `p`
fn nearest_in_2d(shape: &[glam::DVec2], p: glam::DVec2) -> glam::DVec2 {
    match shape.len() {
        0 => p,
        1 => shape[0],
        2 => {
            let (a, b) = (shape[0], shape[1]);
            let ab = b - a;
            let length = ab.length_squared();
            if length <= 0.0 { a } else { a + ab * ((p - a).dot(ab) / length).clamp(0.0, 1.0) }
        }
        count => {
            if (0..count).all(|i| (shape[(i + 1) % count] - shape[i]).perp_dot(p - shape[i]) >= -1e-6) {
                return p;
            }
            (0..count).map(|i| nearest_in_2d(&[shape[i], shape[(i + 1) % count]], p)).min_by(|x, y| x.distance_squared(p).total_cmp(&y.distance_squared(p))).unwrap_or(p)
        }
    }
}

/// A shape's volume, its centre of mass and the inertia tensor of its volume (density 1) about that centre, all in its part's
/// frame: a wedge's or corner wedge's own (measured, Studio 0.747, client and server: a 2 x 4 x 6 wedge weighs half the block and
/// its centre of mass is (0, -2/3, 1) from the part's, its inertia (48.53, 39.2, 20.53) with -11.2 between Y and Z at the
/// Plastic density; a corner wedge a third, (0.25, -1, -0.75)), worked out over the hull's faces in doubles.
pub fn mass_properties(kind: Kind, half: Vec3) -> (f64, DVec3, [[f64; 3]; 3]) {
    let hull = Hull::new(kind, half);
    let (all, count) = hull.dvertices();
    let vertices = &all[..count];
    let (mut volume, mut first, mut second) = (0.0f64, DVec3::ZERO, [[0.0f64; 3]; 3]);
    for face in hull.faces() {
        let corners = face.corners();
        for k in 1..corners.len() - 1 {
            let (a, b, c) = (vertices[corners[0] as usize], vertices[corners[k] as usize], vertices[corners[k + 1] as usize]);
            // the tetrahedron from the origin to the triangle: signed volume, first and second moments
            let v = a.dot(b.cross(c)) / 6.0;
            volume += v;
            first += (a + b + c) * (v / 4.0);
            let s = a + b + c;
            for (i, row) in second.iter_mut().enumerate() {
                for (j, entry) in row.iter_mut().enumerate() {
                    *entry += v / 20.0 * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j] + s[i] * s[j]);
                }
            }
        }
    }
    let centre = first / volume;
    let mut inertia = [[0.0f64; 3]; 3];
    // the second moments about the centre, then the inertia tensor: trace(C) I - C
    let mut central = second;
    for (i, row) in central.iter_mut().enumerate() {
        for (j, entry) in row.iter_mut().enumerate() {
            *entry -= volume * centre[i] * centre[j];
        }
    }
    let trace = central[0][0] + central[1][1] + central[2][2];
    for (i, row) in inertia.iter_mut().enumerate() {
        for (j, entry) in row.iter_mut().enumerate() {
            *entry = if i == j { trace - central[i][j] } else { -central[i][j] };
        }
    }
    (volume, centre, inertia)
}

/// [`mass_properties`]' tensor as a float matrix (columns), times `scale`
pub fn inertia_matrix(inertia: &[[f64; 3]; 3], scale: f64) -> Mat3 {
    Mat3::from_cols_array_2d(&[
        [(inertia[0][0] * scale) as f32, (inertia[1][0] * scale) as f32, (inertia[2][0] * scale) as f32],
        [(inertia[0][1] * scale) as f32, (inertia[1][1] * scale) as f32, (inertia[2][1] * scale) as f32],
        [(inertia[0][2] * scale) as f32, (inertia[1][2] * scale) as f32, (inertia[2][2] * scale) as f32],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wedges_mass_properties_are_a_wedges() {
        // a 2 x 4 x 6 wedge at the Plastic density (0.7): as a client reads them
        let (volume, centre, inertia) = mass_properties(Kind::Wedge, Vec3::new(1.0, 2.0, 3.0));
        let mass = volume * 0.7;
        assert!((mass - 16.8).abs() < 1e-9);
        assert!((centre - DVec3::new(0.0, -2.0 / 3.0, 1.0)).length() < 1e-9);
        let scaled = |i: usize, j: usize| inertia[i][j] * 0.7;
        assert!((scaled(0, 0) - 48.533333).abs() < 1e-5 && (scaled(1, 1) - 39.2).abs() < 1e-9 && (scaled(2, 2) - 20.533333).abs() < 1e-5);
        assert!((scaled(1, 2) + 11.2).abs() < 1e-9 && scaled(0, 1).abs() < 1e-9 && scaled(0, 2).abs() < 1e-9);
        // a corner wedge of the same size
        let (volume, centre, inertia) = mass_properties(Kind::CornerWedge, Vec3::new(1.0, 2.0, 3.0));
        assert!((volume * 0.7 - 11.2).abs() < 1e-9);
        assert!((centre - DVec3::new(0.25, -1.0, -0.75)).length() < 1e-9);
        let scaled = |i: usize, j: usize| inertia[i][j] * 0.7;
        for (i, j, want) in [(0, 0, 30.66), (1, 1, 26.6), (2, 2, 9.38), (0, 1, -1.68), (0, 2, 1.26), (1, 2, 5.04)] {
            assert!((scaled(i, j) - want).abs() < 1e-6, "{i}{j}: {} for {want}", scaled(i, j));
        }
    }

    #[test]
    fn rays_meet_a_wedges_slope_and_miss_along_its_faces() {
        let hull = Hull::new(Kind::Wedge, Vec3::new(2.0, 3.0, 4.0));
        // down onto the slope at z = 2: y = 1.5
        let (t, n) = hull.ray(Vec3::new(0.0, 13.0, 2.0), Vec3::new(0.0, -40.0, 0.0)).unwrap();
        assert!((13.0 - 40.0 * t - 1.5).abs() < 1e-5 && (n - Vec3::new(0.0, 0.8, -0.6)).length() < 1e-6);
        // along the right face's plane, and touching only the bottom front edge
        assert!(hull.ray(Vec3::new(2.0, 10.0, 0.0), Vec3::new(0.0, -20.0, 0.0)).is_none());
        assert!(hull.ray(Vec3::new(0.0, -10.0, -4.0), Vec3::new(0.0, 20.0, 0.0)).is_none());
        // from inside the box but above the slope: the slope
        assert!(hull.ray(Vec3::new(0.0, 2.5, -3.0), Vec3::new(0.0, -10.0, 0.0)).is_some());
        // a corner wedge's ridge: the left slope
        let corner = Hull::new(Kind::CornerWedge, Vec3::new(2.0, 3.0, 4.0));
        let (_, n) = corner.ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -20.0, 0.0)).unwrap();
        assert!(n.x < -0.8 && n.z == 0.0);
    }

    #[test]
    fn a_sphere_meets_a_wedges_edge_rounded() {
        // a unit sphere along +x at y = 0, z = -1.2 past a 4 x 6 x 8 wedge's left slope edge: Distance 5.30601 and the edge's
        // normal (-0.694, 0.576, -0.432) (a client: 5.30603, its iteration's error)
        let hull = Hull::new(Kind::Wedge, Vec3::new(2.0, 3.0, 4.0));
        let (t, n) = hull.sweep_sphere(Vec3::new(-8.0, 0.0, -1.2), Vec3::new(20.0, 0.0, 0.0), 1.0).unwrap();
        assert!((20.0 * t - 5.30601).abs() < 2e-5, "{}", 20.0 * t);
        assert!((n - Vec3::new(-0.69399, 0.57599, -0.43199)).length() < 1e-4, "{n}");
    }
}
