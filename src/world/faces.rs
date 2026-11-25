//! The flat faces Workspace:JoinToOutsiders welds across (world/joints.rs join_to_outsiders), and when a listed part's face
//! touches another part's, as a playing client and its server decide it (roblox-sim, Studio 0.747, both alike; several hundred
//! cases: tools/parity/probes/join_to_outsiders.luau):
//!
//! - **Faces.** A part's faces are the faces of its box that its shape fills, with their outlines: a block's six (a Part of
//!   any other class: MeshPart, TrussPart, Seat, SpawnLocation and the like, whatever their mesh), a Cylinder's two ends (as
//!   the squares of its box, not discs: an end whose disc misses the other face still joins over the square's corner), a
//!   wedge's bottom and back squares and its two side triangles, a corner wedge's bottom square and its two upright triangles
//!   (+X and -Z); a Ball and Terrain none. A WedgePart and a Part shaped Wedge are the same, a CornerWedgePart and a Part
//!   shaped CornerWedge too.
//! - **Aligned.** Each of the listed part's axes is within 0.01 (a sine: 0.573 degrees) of one of the other part's: a turn about the
//!   faces' normal of 0.572 degrees joins and 0.573 does not, a tilt the same, and a turn about both by 0.6 and 0.8 of an angle
//!   stops at the same angle (each axis counts, not one face's normal and edge apart); 90, 180 and 270 degrees join.
//! - **Facing.** A face of the listed part and one of the other's whose normals are opposite.
//! - **Overlap.** The two outlines, the listed part's laid on the other's plane, overlap by more than 0.1 stud along each of
//!   their edges' normals, measured as a separating-axis test measures it (along an axis, the less of how far each outline
//!   reaches past the other's near end): two 4-stud faces offset by 3.88 join and by 3.9 do not; a cube of 0.02 joins while
//!   its far side is 0.1 inside the other face's edge, though it overlaps by 0.02 (a 0.05 cube on a 0.05 cube never joins);
//!   a wedge's triangle counts as a triangle (a cube on its empty half does not join).
//! - **Close.** Over the overlap (the listed part's outline clipped to the other's), the listed part's face is within 0.01 of
//!   the other's plane, a gap or an overlap: a face tilted about its edge on another joins while the overlap's far point is
//!   0.01 off, whichever part is tilted and however far the faces reach beyond the overlap. Float32 world coordinates, so
//!   `gap 0.01` joins near y = 0.5 and 50 and not near 10, 100 and 1,000, where the positions round past it.
//!
//! The weld's C0 is the listed part's face: its centre, turned so its Z axis is the face's outward normal (the six frames below,
//! their negative zeros as a client gives them); C1 is `Part1.CFrame:Inverse() * (Part0.CFrame * C0)` in a script's float32
//! CFrame arithmetic (lune_roblox's), which a client's C1 matched to the bit on 16 of 21 random turned pairs and all axis-aligned
//! ones; the other five differ in the rotation's last bits (up to 3e-7: a client's own frames of those parts differ from what their
//! CFrame reads), the position matching on all 21.

use lune_roblox::datatypes::types::float32 as f32m;

/// What JoinToOutsiders reads of a part: its stored frame (rotation rows, position), half its size and its shape.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub rot: f32m::Rot,
    pub pos: [f32; 3],
    pub half: [f32; 3],
    pub shape: Shape,
}

/// A part's shape as its faces go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Block,
    Ball,
    Cylinder,
    Wedge,
    CornerWedge,
    /// No faces (Terrain)
    Faceless,
}

/// A face of a part: the axis it is across (0 X, 1 Y, 2 Z) and its side (+1, -1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Face {
    pub axis: usize,
    pub side: i8,
}

const SQUARE: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
/// A wedge's side (X faces) over (Y, Z): where y <= z
const WEDGE_SIDE: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
/// A corner wedge's +X face over (Y, Z): where y <= -z
const CORNER_X: &[[f32; 2]] = &[[-1.0, -1.0], [-1.0, 1.0], [1.0, -1.0]];
/// A corner wedge's -Z face over (X, Y): where y <= x
const CORNER_Z: &[[f32; 2]] = &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0]];

/// The outline of `shape`'s face `face`, in its two other axes ((axis + 1) % 3, then (axis + 2) % 3) as fractions of the half
/// size; none where the shape has no face.
fn outline(shape: Shape, face: Face) -> Option<&'static [[f32; 2]]> {
    match (shape, face.axis, face.side) {
        (Shape::Block, _, _) => Some(SQUARE),
        (Shape::Cylinder, 0, _) => Some(SQUARE),
        (Shape::Wedge, 1, -1) | (Shape::Wedge, 2, 1) => Some(SQUARE),
        (Shape::Wedge, 0, _) => Some(WEDGE_SIDE),
        (Shape::CornerWedge, 1, -1) => Some(SQUARE),
        (Shape::CornerWedge, 0, 1) => Some(CORNER_X),
        (Shape::CornerWedge, 2, -1) => Some(CORNER_Z),
        _ => None,
    }
}

/// Every face of a box, in the order the listed part's are tried.
const FACES: [Face; 6] = [
    Face { axis: 0, side: 1 },
    Face { axis: 1, side: 1 },
    Face { axis: 2, side: 1 },
    Face { axis: 0, side: -1 },
    Face { axis: 1, side: -1 },
    Face { axis: 2, side: -1 },
];

/// The sine within which two axes count as one (0.573 degrees): the length of an axis's part across the other's
const ALIGNED: f32 = 0.01;
/// How far apart two faces may be (a gap or an overlap)
const CLOSE: f32 = 0.01;
/// How much two faces must overlap along each edge's normal: more than 0.1 less 1e-7 (in float32, 0.0999999046), which parts the
/// client's outcomes at 41 limit cases exactly as this code works the depth out (the deepest that does not join 0.0999999046,
/// the shallowest that joins 0.0999999419, both below 0.1)
const OVERLAP: f32 = 0.1 - 1e-7;

fn column(rot: &f32m::Rot, axis: usize) -> [f32; 3] {
    f32m::column(rot, axis)
}

/// A point of `body` given in its own frame, in the world (CFrame * Vector3, float32)
fn to_world(body: &Body, local: [f32; 3]) -> [f32; 3] {
    let rotated = f32m::mat_vec(&body.rot, local);
    [rotated[0] + body.pos[0], rotated[1] + body.pos[1], rotated[2] + body.pos[2]]
}

/// The centre of `face` in `body`'s frame
fn face_centre(body: &Body, face: Face) -> [f32; 3] {
    let mut centre = [0.0f32; 3];
    centre[face.axis] = if face.side > 0 { body.half[face.axis] } else { -body.half[face.axis] };
    centre
}

/// The face of `listed` that touches `other` (both [`Body`]), if one does: aligned axes, opposite normals, an overlap of more
/// than [`OVERLAP`] along every edge's normal, and the listed face within [`CLOSE`] of the other's plane over the overlap.
pub fn touching(listed: &Body, other: &Body) -> Option<Face> {
    if !has_faces(listed.shape) || !has_faces(other.shape) {
        return None;
    }
    let matched = matched(listed, other)?;
    for face in FACES {
        let Some(mine) = outline(listed.shape, face) else { continue };
        let (axis, sign) = matched[face.axis];
        // (the other's face whose normal is opposite)
        let side = if (face.side as f32) * sign > 0.0 { -1 } else { 1 };
        let theirs = Face { axis, side };
        let Some(their_outline) = outline(other.shape, theirs) else { continue };
        if meets(listed, face, mine, other, theirs, their_outline) {
            return Some(face);
        }
    }
    None
}

/// Each of the listed part's axes matched to the other's it is nearest, with the sign between them, when its part across that
/// axis is [`ALIGNED`] long at most (in the other's frame: the root of the sum of the squares of its two other components).
fn matched(listed: &Body, other: &Body) -> Option<[(usize, f32); 3]> {
    let mut matched = [(0usize, 1.0f32); 3];
    for (axis, slot) in matched.iter_mut().enumerate() {
        let mine = column(&listed.rot, axis);
        let dots = [0, 1, 2].map(|theirs| f32m::dot3(mine, column(&other.rot, theirs)));
        let theirs = (0..3).max_by(|&a, &b| dots[a].abs().total_cmp(&dots[b].abs()))?;
        let (b, c) = (dots[(theirs + 1) % 3], dots[(theirs + 2) % 3]);
        if !((b * b + c * c).sqrt() <= ALIGNED) {
            return None;
        }
        *slot = (theirs, dots[theirs].signum());
    }
    Some(matched)
}

/// Whether `listed`'s axes line up with `other`'s ([`ALIGNED`]), both having faces: the first of [`touching`]'s tests.
pub fn aligned(listed: &Body, other: &Body) -> bool {
    has_faces(listed.shape) && has_faces(other.shape) && matched(listed, other).is_some()
}

fn has_faces(shape: Shape) -> bool {
    !matches!(shape, Shape::Ball | Shape::Faceless)
}

/// Whether the listed part's face `face` (outline `mine`) and the other's face `theirs` (outline `their_outline`) overlap and lie
/// close, worked out in the other face's plane.
fn meets(listed: &Body, face: Face, mine: &[[f32; 2]], other: &Body, theirs: Face, their_outline: &[[f32; 2]]) -> bool {
    let centre = to_world(other, face_centre(other, theirs));
    let (a, b) = ((theirs.axis + 1) % 3, (theirs.axis + 2) % 3);
    let (u, v) = (column(&other.rot, a), column(&other.rot, b));
    let normal = column(&other.rot, theirs.axis).map(|x| if theirs.side > 0 { x } else { -x });
    let flat: Vec<[f32; 2]> = their_outline.iter().map(|&[x, y]| [x * other.half[a], y * other.half[b]]).collect();
    // (the listed face's corners laid on the other's plane, each with its height above it)
    let (ma, mb) = ((face.axis + 1) % 3, (face.axis + 2) % 3);
    let laid: Vec<([f32; 2], f32)> = mine
        .iter()
        .map(|&[x, y]| {
            let mut local = face_centre(listed, face);
            local[ma] = x * listed.half[ma];
            local[mb] = y * listed.half[mb];
            let at = to_world(listed, local);
            let off = [at[0] - centre[0], at[1] - centre[1], at[2] - centre[2]];
            ([f32m::dot3(off, u), f32m::dot3(off, v)], f32m::dot3(off, normal))
        })
        .collect();
    let points: Vec<[f32; 2]> = laid.iter().map(|&(point, _)| point).collect();
    if !overlaps(&flat, &points) {
        return false;
    }
    let clipped = clip(&laid, &flat);
    !clipped.is_empty() && clipped.iter().all(|&(_, height)| height.abs() <= CLOSE)
}

/// The separating-axis overlap of two convex outlines: along each edge's normal of both, the less of how far each reaches past
/// the other's near end is more than [`OVERLAP`].
fn overlaps(first: &[[f32; 2]], second: &[[f32; 2]]) -> bool {
    for outline in [first, second] {
        for (index, &[x0, y0]) in outline.iter().enumerate() {
            let [x1, y1] = outline[(index + 1) % outline.len()];
            // (the edge's unit normal: an edge along an axis gives that axis exactly, so the depth is the coordinates' difference)
            let (dx, dy) = (x1 - x0, y1 - y0);
            let axis = if dx == 0.0 {
                [1.0, 0.0]
            } else if dy == 0.0 {
                [0.0, 1.0]
            } else {
                let length = (dx * dx + dy * dy).sqrt();
                [dy / length, -dx / length]
            };
            let span = |points: &[[f32; 2]]| {
                points.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &[x, y]| {
                    let along = if axis[1] == 0.0 { x } else if axis[0] == 0.0 { y } else { x * axis[0] + y * axis[1] };
                    (low.min(along), high.max(along))
                })
            };
            let ((low0, high0), (low1, high1)) = (span(first), span(second));
            let depth = (high0 - low1).min(high1 - low0);
            if !(depth > OVERLAP) {
                return false;
            }
        }
    }
    true
}

/// `subject` (points with a height) clipped to the convex outline `window` (Sutherland-Hodgman), the heights carried along
/// its edges.
fn clip(subject: &[([f32; 2], f32)], window: &[[f32; 2]]) -> Vec<([f32; 2], f32)> {
    let count = window.len() as f32;
    let middle = window.iter().fold([0.0f32; 2], |sum, point| [sum[0] + point[0] / count, sum[1] + point[1] / count]);
    let mut points = subject.to_vec();
    for (index, &start) in window.iter().enumerate() {
        let end = window[(index + 1) % window.len()];
        let side = |point: [f32; 2]| (end[0] - start[0]) * (point[1] - start[1]) - (end[1] - start[1]) * (point[0] - start[0]);
        let inward = side(middle).signum();
        let inside = |point: [f32; 2]| side(point) * inward >= 0.0;
        let input = std::mem::take(&mut points);
        for (at, &(point, height)) in input.iter().enumerate() {
            let (previous, previous_height) = input[(at + input.len() - 1) % input.len()];
            let cross = |(a, ha): ([f32; 2], f32), (b, hb): ([f32; 2], f32)| {
                let (sa, sb) = (side(a), side(b));
                let t = sa / (sa - sb);
                ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], ha + (hb - ha) * t)
            };
            match (inside(point), inside(previous)) {
                (true, true) => points.push((point, height)),
                (true, false) => {
                    points.push(cross((previous, previous_height), (point, height)));
                    points.push((point, height));
                }
                (false, true) => points.push(cross((previous, previous_height), (point, height))),
                (false, false) => {}
            }
        }
        if points.is_empty() {
            break;
        }
    }
    points
}

/// The weld's C0 for the listed part's face `face`: the face's centre, turned so its Z axis is the face's outward normal (rows,
/// as a client gives them, negative zeros included).
pub fn weld_c0(body: &Body, face: Face) -> ([f32; 3], f32m::Rot) {
    let rows = match (face.axis, face.side > 0) {
        (0, true) => [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, -0.0, -0.0]],
        (0, false) => [[-0.0, -0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        (1, true) => [[-1.0, -0.0, -0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        (1, false) => [[1.0, 0.0, 0.0], [-0.0, -0.0, -1.0], [0.0, 1.0, 0.0]],
        (_, true) => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        (_, false) => [[-1.0, -0.0, -0.0], [0.0, 1.0, 0.0], [-0.0, -0.0, -1.0]],
    };
    (face_centre(body, face), rows)
}

/// `a * b` as a script's CFrame multiplication (lune_roblox's float32 order)
fn mul(a: ([f32; 3], f32m::Rot), b: ([f32; 3], f32m::Rot)) -> ([f32; 3], f32m::Rot) {
    let rotated = f32m::mat_vec(&a.1, b.0);
    ([rotated[0] + a.0[0], rotated[1] + a.0[1], rotated[2] + a.0[2]], f32m::mat_mul(&a.1, &b.1))
}

/// `a:Inverse()` as a script's CFrame:Inverse (lune_roblox's float32 order)
fn inverse(a: ([f32; 3], f32m::Rot)) -> ([f32; 3], f32m::Rot) {
    let (p, t) = (a.0, f32m::transpose(&a.1));
    (std::array::from_fn(|i| 0.0 - ((t[i][0] * p[0] + t[i][1] * p[1]) + t[i][2] * p[2])), t)
}

/// The weld's C1: `Part1.CFrame:Inverse() * (Part0.CFrame * C0)`, Part0 the listed part.
pub fn weld_c1(listed: &Body, other: &Body, c0: ([f32; 3], f32m::Rot)) -> ([f32; 3], f32m::Rot) {
    mul(inverse((other.pos, other.rot)), mul((listed.pos, listed.rot), c0))
}

/// The world box (low, high corners) of `body`'s box, grown by `margin` on each side.
pub fn world_box(body: &Body, margin: f32) -> ([f32; 3], [f32; 3]) {
    let reach: [f32; 3] = std::array::from_fn(|i| (0..3).map(|j| body.rot[i][j].abs() * body.half[j]).sum::<f32>() + margin);
    (std::array::from_fn(|i| body.pos[i] - reach[i]), std::array::from_fn(|i| body.pos[i] + reach[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(pos: [f32; 3], size: [f32; 3]) -> Body {
        Body { rot: f32m::IDENTITY, pos, half: size.map(|x| x * 0.5), shape: Shape::Block }
    }

    /// The clip keeps a face wholly inside the window, cuts one that crosses its edge at the edge, carrying the heights.
    #[test]
    fn clipping_carries_heights() {
        let window = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let inside = [([-0.5, -0.5], 0.0), ([0.5, -0.5], 0.0), ([0.5, 0.5], 0.0), ([-0.5, 0.5], 0.0)];
        assert_eq!(clip(&inside, &window).len(), 4);
        let across = [([0.0, -0.5], 0.0), ([2.0, -0.5], 2.0), ([2.0, 0.5], 2.0), ([0.0, 0.5], 0.0)];
        let cut = clip(&across, &window);
        assert!(cut.iter().all(|&([x, _], height)| x <= 1.0 + 1e-6 && (height - x).abs() < 1e-6), "{cut:?}");
    }

    /// A 4 x 1 x 4 block on another joins by its bottom, and not across a 0.0101 gap, at 0.12 of overlap but not 0.1.
    #[test]
    fn flush_blocks_touch_by_the_rules() {
        let below = block([0.0, 0.5, 0.0], [4.0, 1.0, 4.0]);
        assert_eq!(touching(&block([0.0, 1.5, 0.0], [4.0, 1.0, 4.0]), &below), Some(Face { axis: 1, side: -1 }));
        assert_eq!(touching(&block([0.0, 1.5101, 0.0], [4.0, 1.0, 4.0]), &below), None);
        assert_eq!(touching(&block([3.88, 1.5, 0.0], [4.0, 1.0, 4.0]), &below), Some(Face { axis: 1, side: -1 }));
        assert_eq!(touching(&block([3.9, 1.5, 0.0], [4.0, 1.0, 4.0]), &below), None);
    }
}
