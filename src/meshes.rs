//! EditableMesh offline: meshes scripts build or load from mesh assets, and the MeshParts that show
//! them. A MeshPart bound to an EditableMesh (CreateMeshPartAsync, ApplyMesh) points at a FileMesh
//! 4.00 snapshot of it under `<state dir>/editable-meshes/`, written again (named by its content)
//! before each frame the recorder samples after the mesh changed, so ror draws what the script made
//! as Roblox shows it: the mesh's own coordinates from the part's origin, scaled by Size / MeshSize
//! (InitialSize keeps MeshSize: ror scales by it), and skinned by its bones.
//!
//! Measured in Studio (2026-10-16): ids carry their kind in the high 32 bits (vertex 1, normal 2,
//! colour 3, uv 4, face 5, bone 6) over a slot from 1; a clone keeps them. A removed element's slot
//! is handed out again, the last freed first, with a generation over the kind's 3 bits (a face slot's
//! first reuse reads `13:3`), and the Get* lists are dense arrays a removed element's place in is
//! taken by the last (measured in a 0.747 client, tools/parity/probes/editable_mesh_vertex_batch.luau,
//! as is all below unless said). A vertex's faces and attributes (GetVertexFaces, BatchGetVertexAttributes)
//! list one entry per corner over it, the newest first: a corner is new when its face is made, its
//! face's vertices are set or its normal, uv or colour is set to another one. A new face's corner
//! (AddTriangle, BatchAdd) takes the normal, uv and colour the newest corner over its vertex uses,
//! else a new automatic normal, a white opaque colour and a (0, 0) uv
//! (tools/parity/probes/editable_mesh_batch.luau). An automatic normal is the sum, over the corners
//! that use it in the faces' order, of the corner's unit normal weighed by 1 - cos^2 of its angle
//! (the cosine of its unit edges), normalised, in single precision as that client computes it (bit
//! for bit on 17 of 17 shared normals); one no face uses reads (0, 0, 0). BatchRemove removes faces
//! only; on a fixed-size mesh (CreateEditableMeshAsync's default) every add and remove is refused.
//! Bone weights keep 1/255 steps. A mesh asset loads one vertex
//! per file vertex (none merged), each with its own normal, uv and colour, the file's bones and bind
//! CFrames, and four bone slots a skinned vertex (zero weights included). MeshSize is the mesh's box
//! when the part is made (at least 0.001) and stays when the mesh changes; ApplyMesh copies MeshSize
//! and keeps Size. RemoveUnused returns the ids it removed.
//!
//! Measured in Studio 0.748 (a play-test client, tools/parity/probes/mesh_extents.luau): CreateMeshPartAsync
//! of an EditableMesh whose box is wider than 2048 studs on any axis raises "Mesh extents size too large.
//! Maximum is (2048, 2048, 2048)." and makes no part; 2048 itself passes (2048.01 and 2049 raise). The box
//! is every vertex's, a face's or not (max - min, as MeshSize), wherever it lies: 100 studs at x = 5000
//! passes, 2112 from x = 0 raises. A mesh that grows past the limit after its part is made raises nothing
//! and leaves the part's MeshSize and Size as they were, and ApplyMesh applies it as it was (measured in a 0.747 client).
//! Not measured and not checked: a mesh asset's own size.
//!
//! Quads, the queries (GetFacesWith*, GetVerticesWith*, FindClosest*, FindVerticesWithinSphere, RaycastLocal), MergeVertices,
//! IdDebugString, bone names and FACS poses: measured in a 0.747 client (tools/parity/probes/editable_mesh_quads.luau,
//! editable_mesh_queries.luau, editable_mesh_bones.luau, editable_mesh_texts.luau); the rules are at each method.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    hash::{Hash, Hasher},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};
#[cfg(feature = "meshes")]
use std::sync::{Mutex, OnceLock};

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::{self as dom, Variant};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use crate::{
    Engine, InstanceId, NameId, ScriptError, State, files,
    instance as core,
    lua::{Args, Flow, Ret, convert, instance_value, none, values},
    services::{Behaviors, FIRST, method, strings},
    value::{ContentValue, Str, Value},
};

const VERTEX: u64 = 1;
const NORMAL: u64 = 2;
const COLOR: u64 = 3;
const UV: u64 = 4;
const FACE: u64 = 5;
const BONE: u64 = 6;

/// The widest box, on each axis, of an EditableMesh a MeshPart is made from (the module's doc: 2048
/// studs passes, 2048.01 raises this)
const MAX_EXTENT: f32 = 2048.0;
const EXTENTS_TOO_LARGE: &str = "Mesh extents size too large. Maximum is (2048, 2048, 2048).";

/// The id of slot `slot` of `kind` in its `generation`: the high 32 bits are the generation over the kind's 3 bits, the
/// low ones the slot from 1 (measured: a face slot's first reuse reads `13:3`, its second `21:3`).
fn make_id(kind: u64, generation: u32, slot: u32) -> f64 {
    (((((generation as u64) << 3) | kind) << 32) | (slot as u64 + 1)) as f64
}

/// An id's kind (0 and 7 name none), generation and slot from 1 (0: none).
fn parts(id: i64) -> (u64, u32, u32) {
    let id = id as u64;
    let high = id >> 32;
    (high & 7, (high >> 3) as u32, id as u32)
}

/// A number read as an id as the client reads one: truncated toward zero, NaN and numbers past the int64 range the
/// integer indefinite (a C cast on x86). Measured: an id plus 0.5 reads the id, NaN reads as no id (not as 0).
fn id_of_number(n: f64) -> i64 {
    if n.is_nan() || n >= 9_223_372_036_854_775_808.0 || n < -9_223_372_036_854_775_808.0 { i64::MIN } else { n as i64 }
}

const KIND_NAMES: [&str; 8] = ["", "Vertex", "Normal", "Color", "UV", "Face", "Bone", "Unknown"];

/// The client's error for `id` where an id of `kind` was expected (measured: kind 7 is "Unknown", kind 0 no id at all).
fn wrong_kind(kind: u64, id: i64) -> Flow {
    let got = match parts(id).0 {
        0 => "value that doesn't appear to be an id".to_string(),
        other => format!("{} id", KIND_NAMES[other as usize]),
    };
    err(format!("Unexpected EditableMesh id type. Expected {} id, received {got}", KIND_NAMES[kind as usize]))
}

/// Items addressed by slot, as the client keeps a mesh's vertices, normals, colours, uvs, faces and bones (measured in a
/// 0.747 client, tools/parity/probes/editable_mesh_vertex_batch.luau; bones by analogy): a removed item's slot is handed
/// out again, the last freed first, under its next generation, and the Get* lists read a dense array where a new item
/// goes at the end and a removed one's place is taken by the last one.
#[derive(Clone, Debug)]
struct Slots<T> {
    items: Vec<Option<T>>,
    /// Per slot, how often it was freed: the generation its ids carry
    generations: Vec<u32>,
    /// Slots to hand out again, the last pushed first
    free: Vec<u32>,
    /// The live slots in listing order
    listed: Vec<u32>,
    /// Per slot, its place in `listed` while it is live
    places: Vec<u32>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self { items: Vec::new(), generations: Vec::new(), free: Vec::new(), listed: Vec::new(), places: Vec::new() }
    }
}

impl<T> Slots<T> {
    fn grow(&mut self) -> u32 {
        self.items.push(None);
        self.generations.push(0);
        self.places.push(0);
        (self.items.len() - 1) as u32
    }
    /// A slot for a new item: the last one freed, else a new one.
    fn take(&mut self) -> u32 {
        self.free.pop().unwrap_or_else(|| self.grow())
    }
    /// Makes `count` slots free, new ones pushed after those already free (BatchAdd's faces take them so: measured, a
    /// batch of three on a mesh with one free slot reads two new ids backwards, then the free one).
    fn reserve(&mut self, count: usize) {
        while self.free.len() < count {
            let slot = self.grow();
            self.free.push(slot);
        }
    }
    /// Puts `item` in `slot` (one [`Slots::take`] gave) at the end of the list.
    fn put(&mut self, slot: u32, item: T) {
        self.items[slot as usize] = Some(item);
        self.places[slot as usize] = self.listed.len() as u32;
        self.listed.push(slot);
    }
    fn add(&mut self, item: T) -> u32 {
        let slot = self.take();
        self.put(slot, item);
        slot
    }
    fn get(&self, slot: u32) -> Option<&T> {
        self.items.get(slot as usize).and_then(Option::as_ref)
    }
    fn get_mut(&mut self, slot: u32) -> Option<&mut T> {
        self.items.get_mut(slot as usize).and_then(Option::as_mut)
    }
    /// Frees `slot` for reuse under its next generation, leaving it listed until [`Slots::unlist`].
    fn release(&mut self, slot: u32) -> Option<T> {
        let item = self.items.get_mut(slot as usize)?.take()?;
        let generation = &mut self.generations[slot as usize];
        *generation = (*generation + 1) & 0x1FFF_FFFF;
        self.free.push(slot);
        Some(item)
    }
    /// Takes `slot` out of the list, the last listed item into its place.
    fn unlist(&mut self, slot: u32) {
        let place = self.places[slot as usize] as usize;
        self.listed.swap_remove(place);
        if let Some(&moved) = self.listed.get(place) {
            self.places[moved as usize] = place as u32;
        }
    }
    fn remove(&mut self, slot: u32) -> Option<T> {
        let item = self.release(slot)?;
        self.unlist(slot);
        Some(item)
    }
    /// Where `slot` is in the list.
    fn place(&self, slot: u32) -> u32 {
        self.places[slot as usize]
    }
    /// The live items in listing order (the Get* lists').
    fn live(&self) -> impl Iterator<Item = (u32, &T)> {
        self.listed.iter().map(|&slot| (slot, self.items[slot as usize].as_ref().expect("listed slots are live")))
    }
    /// The live items in slot order (a snapshot's).
    fn by_slot(&self) -> impl Iterator<Item = (u32, &T)> {
        self.items.iter().enumerate().filter_map(|(slot, item)| item.as_ref().map(|item| (slot as u32, item)))
    }
    fn count(&self) -> usize {
        self.listed.len()
    }
    /// The id of live slot `slot`.
    fn id(&self, kind: u64, slot: u32) -> f64 {
        make_id(kind, self.generations.get(slot as usize).copied().unwrap_or(0), slot)
    }
    /// The live slot `id` names, when it is of `kind` and of the slot's current generation.
    fn find(&self, kind: u64, id: i64) -> Option<u32> {
        let (k, generation, low) = parts(id);
        if k != kind || low == 0 {
            return None;
        }
        let slot = low - 1;
        (self.get(slot).is_some() && self.generations[slot as usize] == generation).then_some(slot)
    }
    /// [`Slots::find`], else the client's "Invalid id".
    fn at(&self, kind: u64, id: i64) -> Result<u32, Flow> {
        self.find(kind, id).ok_or_else(|| err("Invalid id"))
    }
}

/// A face: a triangle or a quad (measured in a 0.747 client: AddFace, BatchAdd and SetFaceVertices take 3 or 4 vertices, and a
/// quad is one face of four corners; tools/parity/probes/editable_mesh_quads.luau).
#[derive(Clone, Copy, Debug)]
struct Face {
    /// Its corners: 3 or 4
    count: u8,
    vertices: [u32; 4],
    normals: [u32; 4],
    colors: [u32; 4],
    uvs: [u32; 4],
    /// Per corner, when it last came to the front of its vertex's corners (higher is newer): see [`Mesh::corners_of`]
    stamps: [u32; 4],
}

impl Face {
    fn len(&self) -> usize {
        self.count as usize
    }
    /// The corner before `corner` and the one after it.
    fn around(&self, corner: usize) -> (usize, usize) {
        let n = self.len();
        ((corner + n - 1) % n, (corner + 1) % n)
    }
    /// Its edges as vertex pairs, corner to next corner.
    fn edges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (0..self.len()).map(|corner| (self.vertices[corner], self.vertices[(corner + 1) % self.len()]))
    }
}

/// How a quad is split along a diagonal (measured in a 0.747 client, Triangulate and a mesh's FileMesh copy: the shorter
/// diagonal, the one from the first corner on a tie): true for the one from the second corner to the fourth.
fn second_diagonal(p: [Vec3; 4]) -> bool {
    (p[3] - p[1]).length_squared() < (p[2] - p[0]).length_squared()
}

#[derive(Clone, Debug)]
struct Bone {
    name: String,
    /// CFrame components: position then the 3x3 rotation row by row (CFrame.new's order)
    cframe: [f32; 12],
    parent: Option<u32>,
    virtual_bone: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    positions: Slots<Vec3>,
    /// None: automatic (the faces' normal)
    normals: Slots<Option<Vec3>>,
    colors: Slots<([f32; 3], f32)>,
    uvs: Slots<[f32; 2]>,
    faces: Slots<Face>,
    bones: Slots<Bone>,
    /// Per vertex index: its bones (indices) and weights, when it has any
    skin: FxHashMap<u32, (Vec<u32>, Vec<f32>)>,
    /// The next corner stamp
    next_stamp: u32,
    /// Per vertex index, what the corner at the front of its corners uses: (its stamp, [normal, uv, colour]); kept up to
    /// date as faces are added or their corners set; None after a change that can take a vertex's front corner away (made
    /// again from the faces when next needed)
    corners: Option<FxHashMap<u32, (u32, [u32; 3])>>,
    /// FACS poses: per FacsActionUnit value, its bones (slots) and CFrames
    facs: BTreeMap<u32, Vec<(u32, [f32; 12])>>,
    /// Corrective FACS poses: per sorted set of 2 or 3 action values
    correctives: BTreeMap<Vec<u32>, Vec<(u32, [f32; 12])>>,
}

impl Mesh {
    fn face_normal(&self, face: &Face) -> Vec3 {
        let p = |i: usize| self.positions.get(face.vertices[i]).copied().unwrap_or(Vec3::ZERO);
        if face.len() == 4 {
            return (p(2) - p(0)).cross(p(3) - p(1));
        }
        (p(1) - p(0)).cross(p(2) - p(0))
    }

    /// What corner `corner` of `face` adds to an automatic normal it uses: a unit normal times 1 - cos^2 of the corner's angle
    /// (the cosine of its unit edges to the next corner and the one before), with the operations in the client's order
    /// (glam's normalise multiplies by the length's reciprocal, as it does). A triangle's unit normal is the corner's own
    /// (its edges' cross product), a quad's the face's: the cross product of its diagonals, first corner's to second's
    /// (measured in a 0.747 client: bit for bit on 16 of 16 corners of 4 quads, tools/parity/probes/editable_mesh_quads.luau).
    fn corner_weight(&self, face: &Face, corner: usize) -> Vec3 {
        let p = |i: usize| self.positions.get(face.vertices[i]).copied().unwrap_or(Vec3::ZERO);
        let at = p(corner);
        let (before, after) = face.around(corner);
        let (e1, e2) = (p(after) - at, p(before) - at);
        let cos = e1.normalize_or_zero().dot(e2.normalize_or_zero());
        let unit = if face.len() == 4 { self.face_normal(face).normalize_or_zero() } else { e1.cross(e2).normalize_or_zero() };
        unit * (1.0 - cos * cos)
    }

    /// The live faces from the last listed back: the order an automatic normal's sum adds its corners in, each face's in
    /// corner order (measured in a 0.747 client: bit for bit on 44 of 44 normals that triangles and quads share, where the
    /// listing order misses 3; tools/parity/probes/editable_mesh_quads.luau).
    fn faces_in_order(&self) -> impl Iterator<Item = &Face> {
        self.faces.listed.iter().rev().map(|&slot| self.faces.items[slot as usize].as_ref().expect("listed slots are live"))
    }

    /// Every automatic normal's value: the normalised sum of its corners' weights; (0, 0, 0) when no face uses it.
    fn automatic_normals(&self) -> FxHashMap<u32, Vec3> {
        let mut sums: FxHashMap<u32, Vec3> = FxHashMap::default();
        for face in self.faces_in_order() {
            for (corner, &n) in face.normals[..face.len()].iter().enumerate() {
                if matches!(self.normals.get(n), Some(None)) {
                    *sums.entry(n).or_insert(Vec3::ZERO) += self.corner_weight(face, corner);
                }
            }
        }
        sums.into_iter().map(|(n, sum)| (n, sum.normalize_or_zero())).collect()
    }

    fn normal_value(&self, index: u32) -> Option<Vec3> {
        match self.normals.get(index)? {
            Some(normal) => Some(*normal),
            None => {
                let mut sum = Vec3::ZERO;
                for face in self.faces_in_order() {
                    for (corner, &n) in face.normals[..face.len()].iter().enumerate() {
                        if n == index {
                            sum += self.corner_weight(face, corner);
                        }
                    }
                }
                Some(sum.normalize_or_zero())
            }
        }
    }

    /// The corners over vertex `v`, front first: (face slot, corner). Measured in a 0.747 client
    /// (tools/parity/probes/editable_mesh_vertex_batch.luau): a vertex's faces and attributes list one entry per corner
    /// (a face over it twice, twice), and a corner comes to the front when its face is made (its corners in order, so a
    /// face over a vertex twice lists its second corner first), when its face's vertices are set (all its corners, the same
    /// ones too), or when its normal, uv or colour is set to another one; a removed face's corners leave the rest in order.
    fn corners_of(&self, v: u32) -> Vec<(u32, usize)> {
        self.corners_where(|face, corner| face.vertices[corner] == v)
    }

    /// The corners `uses(face, corner)` picks, front first (by their stamps, as [`Mesh::corners_of`]).
    fn corners_where(&self, mut uses: impl FnMut(&Face, usize) -> bool) -> Vec<(u32, usize)> {
        let mut found: Vec<(u32, u32, usize)> = Vec::new();
        for (f, face) in self.faces.live() {
            for corner in 0..face.len() {
                if uses(face, corner) {
                    found.push((face.stamps[corner], f, corner));
                }
            }
        }
        crate::sort::by_key(&mut found, |entry| std::cmp::Reverse(entry.0));
        found.into_iter().map(|(_, f, corner)| (f, corner)).collect()
    }

    /// [`Mesh::corners_of`] for several vertices at once (one pass over the faces).
    fn corners_of_all(&self, vertices: &[u32]) -> FxHashMap<u32, Vec<(u32, usize)>> {
        let mut found: FxHashMap<u32, Vec<(u32, u32, usize)>> = vertices.iter().map(|&v| (v, Vec::new())).collect();
        for (f, face) in self.faces.live() {
            for corner in 0..face.len() {
                if let Some(list) = found.get_mut(&face.vertices[corner]) {
                    list.push((face.stamps[corner], f, corner));
                }
            }
        }
        found
            .into_iter()
            .map(|(v, mut list)| {
                crate::sort::by_key(&mut list, |entry| std::cmp::Reverse(entry.0));
                (v, list.into_iter().map(|(_, f, corner)| (f, corner)).collect())
            })
            .collect()
    }

    /// The normal, uv and colour the corner at the front of vertex `v`'s corners uses, if any face uses `v`.
    fn corner_of(&mut self, v: u32) -> Option<[u32; 3]> {
        if self.corners.is_none() {
            let mut corners: FxHashMap<u32, (u32, [u32; 3])> = FxHashMap::default();
            for (_, face) in self.faces.live() {
                for corner in 0..face.len() {
                    let here = (face.stamps[corner], [face.normals[corner], face.uvs[corner], face.colors[corner]]);
                    let entry = corners.entry(face.vertices[corner]).or_insert(here);
                    if here.0 >= entry.0 {
                        *entry = here;
                    }
                }
            }
            self.corners = Some(corners);
        }
        self.corners.as_ref().and_then(|corners| corners.get(&v)).map(|(_, attributes)| *attributes)
    }

    fn stamp(&mut self) -> u32 {
        self.next_stamp += 1;
        self.next_stamp
    }

    /// Brings corner `corner` of face `f` to the front of its vertex's corners (see [`Mesh::corners_of`]).
    fn touch(&mut self, f: u32, corner: usize) {
        let stamp = self.stamp();
        let Some(face) = self.faces.get_mut(f) else { return };
        face.stamps[corner] = stamp;
        let face = *face;
        if let Some(corners) = self.corners.as_mut() {
            corners.insert(face.vertices[corner], (stamp, [face.normals[corner], face.uvs[corner], face.colors[corner]]));
        }
    }

    /// Sets attribute `which` (1 normal, 2 colour, 3 uv) of corner `corner` of face `f` to `slot`; a corner given another
    /// one comes to the front of its vertex's corners.
    fn set_corner(&mut self, f: u32, corner: usize, which: u32, slot: u32) {
        let Some(face) = self.faces.get_mut(f) else { return };
        let target = match which {
            1 => &mut face.normals[corner],
            2 => &mut face.colors[corner],
            _ => &mut face.uvs[corner],
        };
        if *target != slot {
            *target = slot;
            self.touch(f, corner);
        }
    }

    /// Sets face `f`'s vertices (3 or 4): every corner comes to the front of its (new) vertex's corners. A corner keeps
    /// its normal, uv and colour; a fourth corner a triangle gains takes what a new face's would (measured in a 0.747
    /// client: the front corner's over its vertex, else new ones), and a quad given three drops its fourth.
    fn set_face_vertices(&mut self, f: u32, vertices: &[u32]) {
        let Some(old) = self.faces.get(f).copied() else { return };
        let mut face = old;
        face.count = vertices.len() as u8;
        face.vertices[..vertices.len()].copy_from_slice(vertices);
        if vertices.len() > old.len() {
            let corner = old.len();
            let v = vertices[corner];
            let attributes = match vertices[..corner].iter().position(|&earlier| earlier == v) {
                Some(earlier) => [face.normals[earlier], face.uvs[earlier], face.colors[earlier]],
                None => match self.corner_of(v) {
                    Some(found) => found,
                    None => [self.normals.add(None), self.uvs.add([0.0; 2]), self.colors.add(([1.0; 3], 1.0))],
                },
            };
            face.normals[corner] = attributes[0];
            face.uvs[corner] = attributes[1];
            face.colors[corner] = attributes[2];
        }
        for corner in 0..face.len() {
            face.stamps[corner] = self.stamp();
        }
        *self.faces.get_mut(f).expect("live") = face;
        self.faces_moved();
    }

    /// After a change that can take a vertex's front corner away (a face removed, a face's vertices set).
    fn faces_moved(&mut self) {
        self.corners = None;
    }

    /// A new face over `vertices` (3 or 4): each corner takes what the corner at the front of its vertex's corners uses (a
    /// vertex twice in the face shares it), else a new automatic normal, a (0, 0) uv and a white opaque colour, made corner
    /// by corner.
    fn new_face(&mut self, vertices: &[u32]) -> Face {
        let count = vertices.len();
        let mut attributes = [[0u32; 3]; 4];
        for corner in 0..count {
            let v = vertices[corner];
            attributes[corner] = match vertices[..corner].iter().position(|&earlier| earlier == v) {
                Some(earlier) => attributes[earlier],
                None => match self.corner_of(v) {
                    Some(found) => found,
                    None => [self.normals.add(None), self.uvs.add([0.0; 2]), self.colors.add(([1.0; 3], 1.0))],
                },
            };
        }
        let mut stamps = [0u32; 4];
        for stamp in &mut stamps[..count] {
            *stamp = self.stamp();
        }
        if let Some(corners) = self.corners.as_mut() {
            for corner in 0..count {
                corners.insert(vertices[corner], (stamps[corner], attributes[corner]));
            }
        }
        let mut face = Face { count: count as u8, vertices: [0; 4], normals: [0; 4], uvs: [0; 4], colors: [0; 4], stamps };
        for corner in 0..count {
            face.vertices[corner] = vertices[corner];
            face.normals[corner] = attributes[corner][0];
            face.uvs[corner] = attributes[corner][1];
            face.colors[corner] = attributes[corner][2];
        }
        face
    }

    /// A quad's two triangles as Triangulate and a FileMesh copy make them: along the shorter diagonal ([`second_diagonal`]),
    /// (c0, c1, c2) and (c2, c3, c0), or (c1, c2, c3) and (c3, c0, c1) (measured in a 0.747 client); a triangle as it is.
    fn split(&self, face: &Face) -> ([usize; 3], Option<[usize; 3]>) {
        if face.len() == 3 {
            return ([0, 1, 2], None);
        }
        let p = |i: usize| self.positions.get(face.vertices[i]).copied().unwrap_or(Vec3::ZERO);
        if second_diagonal([p(0), p(1), p(2), p(3)]) { ([1, 2, 3], Some([3, 0, 1])) } else { ([0, 1, 2], Some([2, 3, 0])) }
    }

    /// The triangles the queries (FindClosestPointOnSurface, RaycastLocal) test of a face: a quad as a fan from the end of its
    /// shorter diagonal, (c0, c1, c2) and (c0, c2, c3), or (c1, c2, c3) and (c1, c3, c0) (measured in a 0.747 client: the
    /// vertex ids those answer).
    fn query_triangles(&self, face: &Face) -> ([usize; 3], Option<[usize; 3]>) {
        match self.split(face) {
            ([1, 2, 3], Some(_)) => ([1, 2, 3], Some([1, 3, 0])),
            ([0, 1, 2], Some(_)) => ([0, 1, 2], Some([0, 2, 3])),
            other => other,
        }
    }

    /// Removes faces (BatchRemove, RemoveFace): measured, in the order of their places in the list from the last, each one's
    /// slot freed and its place taken by the last listed face.
    fn remove_faces(&mut self, faces: &[u32]) {
        let mut order: Vec<u32> = faces.to_vec();
        crate::sort::by_key(&mut order, |&f| std::cmp::Reverse(self.faces.place(f)));
        order.dedup();
        for f in order {
            self.faces.remove(f);
        }
        self.faces_moved();
    }

    /// Triangulate: each quad becomes the two triangles [`Mesh::split`] makes of it, which keep its corners' vertices, normals,
    /// uvs and colours; the quads go as BatchRemove takes faces and the triangles come as BatchAdd makes a batch of faces
    /// (measured in a 0.747 client: two quads' triangles read `5:5`, `5:4`, `13:2`, `13:3`).
    fn triangulate(&mut self) {
        let quads: Vec<u32> = self.faces.live().filter(|(_, face)| face.len() == 4).map(|(slot, _)| slot).collect();
        if quads.is_empty() {
            return;
        }
        let mut triangles = Vec::with_capacity(quads.len() * 2);
        for &quad in &quads {
            let face = *self.faces.get(quad).expect("live");
            let (first, second) = self.split(&face);
            for corners in std::iter::once(first).chain(second) {
                let mut triangle = face;
                triangle.count = 3;
                for (at, &corner) in corners.iter().enumerate() {
                    triangle.vertices[at] = face.vertices[corner];
                    triangle.normals[at] = face.normals[corner];
                    triangle.uvs[at] = face.uvs[corner];
                    triangle.colors[at] = face.colors[corner];
                    triangle.stamps[at] = face.stamps[corner];
                }
                triangles.push(triangle);
            }
        }
        self.remove_faces(&quads);
        self.faces.reserve(triangles.len());
        let slots: Vec<u32> = triangles.iter().map(|_| self.faces.take()).collect();
        for (triangle, slot) in triangles.into_iter().zip(slots) {
            self.faces.put(slot, triangle);
        }
        self.faces_moved();
    }

    /// The corners using normal, uv or colour `slot` (of `kind`), front first.
    fn attribute_corners(&self, kind: u64, slot: u32) -> Vec<(u32, usize)> {
        self.corners_where(|face, corner| match kind {
            NORMAL => face.normals[corner] == slot,
            COLOR => face.colors[corner] == slot,
            _ => face.uvs[corner] == slot,
        })
    }

    /// FindClosestPointOnSurface's and RaycastLocal's answer: the face's id, the point, its barycentric weights in the triangle
    /// `vertices` (vertex slots) and their ids.
    fn query_answer(&self, lua: &Lua, engine: &Engine, f: u32, vertices: [u32; 3], point: Vec3) -> Ret {
        let [a, b, c] = vertices.map(|v| self.positions.get(v).copied().unwrap_or(Vec3::ZERO));
        let weights = barycentric(point, a, b, c);
        Ok(LuaMultiValue::from_iter([
            LuaValue::Number(self.faces.id(FACE, f)),
            vector_value(lua, engine, point)?,
            vector_value(lua, engine, weights)?,
            LuaValue::Number(self.positions.id(VERTEX, vertices[0])),
            LuaValue::Number(self.positions.id(VERTEX, vertices[1])),
            LuaValue::Number(self.positions.id(VERTEX, vertices[2])),
        ]))
    }

    /// MergeVertices: each vertex in slot order goes to the first one before it (in slot order, merged or not) whose squared
    /// distance is at most the tolerance squared: its corners move to that vertex, each coming to the front of its corners in
    /// the order they were listed, and keep their normals, uvs and colours; every vertex stays (RemoveUnused takes the ones no
    /// face uses). The answer maps the ids of the vertices merged into one that took two or more, by their id's text, to that
    /// one's id (measured in a 0.747 client, 16 meshes: a vertex that took one alone is left out of it, a chain of merges goes
    /// one step each). Also the vertices compared, for the price.
    fn merge_vertices(&mut self, tolerance: f32) -> (Vec<(f64, f64)>, usize) {
        let reach = tolerance * tolerance;
        let cell = tolerance.abs().max(1e-6);
        let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
        let vertices: Vec<(u32, Vec3)> = self.positions.by_slot().map(|(slot, &p)| (slot, p)).collect();
        let mut grid: FxHashMap<(i64, i64, i64), Vec<u32>> = FxHashMap::default();
        let mut targets: Vec<(u32, u32)> = Vec::new();
        for &(slot, p) in &vertices {
            let (x, y, z) = key(p);
            let mut found: Option<u32> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &other in grid.get(&(x + dx, y + dy, z + dz)).into_iter().flatten() {
                            let q = *self.positions.get(other).expect("live");
                            if (p - q).length_squared() <= reach && found.is_none_or(|first| other < first) {
                                found = Some(other);
                            }
                        }
                    }
                }
            }
            if let Some(target) = found {
                targets.push((slot, target));
                for (f, corner) in self.corners_of(slot) {
                    if let Some(face) = self.faces.get_mut(f) {
                        face.vertices[corner] = target;
                    }
                    self.touch(f, corner);
                }
            }
            grid.entry((x, y, z)).or_default().push(slot);
        }
        self.faces_moved();
        let mut taken: FxHashMap<u32, usize> = FxHashMap::default();
        for &(_, target) in &targets {
            *taken.entry(target).or_default() += 1;
        }
        let map = targets
            .into_iter()
            .filter(|(_, target)| taken[target] >= 2)
            .map(|(slot, target)| (self.positions.id(VERTEX, slot), self.positions.id(VERTEX, target)))
            .collect();
        (map, vertices.len())
    }

    /// Takes bone `slot` out of every FACS pose (it was removed or made not virtual).
    fn drop_bone_poses(&mut self, slot: u32) {
        for pose in self.facs.values_mut().chain(self.correctives.values_mut()) {
            pose.retain(|(bone, _)| *bone != slot);
        }
        self.facs.retain(|_, pose| !pose.is_empty());
        self.correctives.retain(|_, pose| !pose.is_empty());
    }

    fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut out: Option<(Vec3, Vec3)> = None;
        for (_, &p) in self.positions.live() {
            out = Some(match out {
                Some((min, max)) => (min.min(p), max.max(p)),
                None => (p, p),
            });
        }
        out
    }

    /// The mesh's box size as MeshSize reads it (each axis at least 0.001).
    pub fn size(&self) -> Vec3 {
        self.bounds().map_or(Vec3::ZERO, |(min, max)| max - min).max(Vec3::splat(0.001))
    }

    /// Whether a part made from it is skinned (HasSkinnedMesh): some vertex weighs a bone above 0
    /// (measured: bones that no vertex weighs leave it false).
    pub fn skinned(&self) -> bool {
        self.skin.iter().any(|(&vertex, (bones, weights))| {
            self.positions.get(vertex).is_some()
                && bones.iter().zip(weights).any(|(&bone, &weight)| weight > 0.0 && self.bones.get(bone).is_some())
        })
    }

    /// Removes what no face uses: vertices, normals, colours, uvs (Studio's RemoveUnused); the ids, each kind in listing
    /// order. Measured in a 0.747 client: the slots are freed in that order (the last of them handed out first) while the
    /// list is closed up from its end (each removed place taken by the last listed item, the last place first).
    fn remove_unused(&mut self) -> Vec<f64> {
        fn sweep<T>(slots: &mut Slots<T>, kind: u64, used: &FxHashSet<u32>, removed: &mut Vec<f64>) -> Vec<u32> {
            let unused: Vec<u32> = slots.live().map(|(slot, _)| slot).filter(|slot| !used.contains(slot)).collect();
            removed.extend(unused.iter().map(|&slot| slots.id(kind, slot)));
            for &slot in &unused {
                slots.release(slot);
            }
            for &slot in unused.iter().rev() {
                slots.unlist(slot);
            }
            unused
        }
        let mut used = [FxHashSet::default(), FxHashSet::default(), FxHashSet::default(), FxHashSet::default()];
        for (_, face) in self.faces.live() {
            used[0].extend(&face.vertices[..face.len()]);
            used[1].extend(&face.normals[..face.len()]);
            used[2].extend(&face.colors[..face.len()]);
            used[3].extend(&face.uvs[..face.len()]);
        }
        let mut removed = Vec::new();
        for vertex in sweep(&mut self.positions, VERTEX, &used[0], &mut removed) {
            self.skin.remove(&vertex);
        }
        sweep(&mut self.normals, NORMAL, &used[1], &mut removed);
        sweep(&mut self.colors, COLOR, &used[2], &mut removed);
        sweep(&mut self.uvs, UV, &used[3], &mut removed);
        removed
    }

    /// A mesh from a decoded FileMesh: its first LOD's faces over every file vertex.
    #[cfg(feature = "meshes")]
    fn from_file(file: &ror_mesh::Mesh) -> Mesh {
        let mut mesh = Mesh::default();
        let count = file.positions.len();
        for index in 0..count {
            mesh.positions.add(Vec3::from(file.positions[index]));
            mesh.normals.add(Some(file.normals.get(index).map_or(Vec3::ZERO, |n| Vec3::from(*n))));
            let color = file.colors.get(index).copied().unwrap_or([255; 4]);
            mesh.colors.add(([color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0], color[3] as f32 / 255.0));
            mesh.uvs.add(file.uvs.get(index).copied().unwrap_or([0.0; 2]));
        }
        let range = file.lods.first().cloned().unwrap_or(0..file.indices.len() as u32);
        for corners in file.indices[range.start as usize..range.end as usize].chunks_exact(3) {
            let c = [corners[0], corners[1], corners[2]];
            if c.iter().all(|&i| (i as usize) < count) {
                let stamps = [mesh.stamp(), mesh.stamp(), mesh.stamp(), 0];
                let c = [c[0], c[1], c[2], 0];
                mesh.faces.add(Face { count: 3, vertices: c, normals: c, colors: c, uvs: c, stamps });
            }
        }
        if let Some(skinning) = &file.skinning {
            for bone in &skinning.bones {
                let r = bone.rotation;
                let p = bone.position;
                mesh.bones.add(Bone {
                    name: bone.name.clone(),
                    cframe: [p[0], p[1], p[2], r[0][0], r[0][1], r[0][2], r[1][0], r[1][1], r[1][2], r[2][0], r[2][1], r[2][2]],
                    parent: bone.parent.map(u32::from),
                    virtual_bone: false,
                });
            }
            for (vertex, (joints, weights)) in skinning.joints.iter().zip(&skinning.weights).enumerate() {
                if weights.iter().any(|&w| w > 0.0) {
                    mesh.skin.insert(vertex as u32, (joints.iter().map(|&j| j as u32).collect(), weights.to_vec()));
                }
            }
        }
        mesh
    }
}

/// A FileMesh 4.00 of `mesh` (Roblox's layout: ror-mesh's filemesh.rs and skinning.rs read it):
/// one file vertex per distinct (vertex, normal, uv, colour) corner, skinned meshes in subsets of
/// at most 26 bones.
pub fn encode(mesh: &Mesh) -> Vec<u8> {
    let automatic = mesh.automatic_normals();
    // (faces and bones in slot order, as snapshots have always had them)
    let bone_order: Vec<u32> = mesh.bones.by_slot().map(|(i, _)| i).collect();
    let bone_slot: FxHashMap<u32, u16> = bone_order.iter().enumerate().map(|(slot, &i)| (i, slot as u16)).collect();
    let skinned = !bone_order.is_empty();
    // (a quad as the two triangles Triangulate makes of it: Mesh::split)
    let mut faces: Vec<Face> = Vec::with_capacity(mesh.faces.count());
    for (_, face) in mesh.faces.by_slot() {
        let (first, second) = mesh.split(face);
        for corners in std::iter::once(first).chain(second) {
            let mut triangle = *face;
            triangle.count = 3;
            for (at, &corner) in corners.iter().enumerate() {
                triangle.vertices[at] = face.vertices[corner];
                triangle.normals[at] = face.normals[corner];
                triangle.uvs[at] = face.uvs[corner];
                triangle.colors[at] = face.colors[corner];
            }
            faces.push(triangle);
        }
    }

    // Subsets: faces in order, a new subset when a face's bones would pass 26
    let vertex_bones = |vertex: u32| -> Vec<u16> {
        mesh.skin.get(&vertex).map_or(Vec::new(), |(bones, weights)| {
            bones.iter().zip(weights).filter(|(_, w)| **w > 0.0).filter_map(|(b, _)| bone_slot.get(b).copied()).collect()
        })
    };
    let mut groups: Vec<(Vec<usize>, Vec<u16>)> = vec![(Vec::new(), Vec::new())];
    for (index, face) in faces.iter().enumerate() {
        let mut needed: Vec<u16> = Vec::new();
        if skinned {
            for &v in &face.vertices[..3] {
                for bone in vertex_bones(v) {
                    if !needed.contains(&bone) {
                        needed.push(bone);
                    }
                }
            }
        }
        let group = groups.last_mut().unwrap();
        let extra = needed.iter().filter(|b| !group.1.contains(b)).count();
        if group.1.len() + extra > 26 && !group.0.is_empty() {
            groups.push((Vec::new(), Vec::new()));
        }
        let group = groups.last_mut().unwrap();
        for bone in needed {
            if !group.1.contains(&bone) {
                group.1.push(bone);
            }
        }
        group.0.push(index);
    }

    let mut vertices: Vec<u8> = Vec::new();
    let mut envelopes: Vec<u8> = Vec::new();
    let mut indices: Vec<u32> = Vec::with_capacity(faces.len() * 3);
    let mut subsets: Vec<[u32; 4]> = Vec::new();
    let mut culling = vec![0f32; bone_order.len()];
    let mut vertex_count = 0u32;
    for (group_faces, group_bones) in &groups {
        let first_vertex = vertex_count;
        let first_face = (indices.len() / 3) as u32;
        let mut seen: FxHashMap<[u32; 4], u32> = FxHashMap::default();
        for &face_index in group_faces {
            let face = faces[face_index];
            for corner in 0..3 {
                let key = [face.vertices[corner], face.normals[corner], face.uvs[corner], face.colors[corner]];
                let index = *seen.entry(key).or_insert_with(|| {
                    let position = mesh.positions.get(key[0]).copied().unwrap_or(Vec3::ZERO);
                    let normal = match mesh.normals.get(key[1]) {
                        Some(Some(normal)) => *normal,
                        Some(None) => automatic.get(&key[1]).copied().unwrap_or(Vec3::ZERO),
                        None => mesh.face_normal(&face).normalize_or_zero(),
                    };
                    let uv = mesh.uvs.get(key[2]).copied().unwrap_or([0.0; 2]);
                    let (rgb, alpha) = mesh.colors.get(key[3]).copied().unwrap_or(([1.0; 3], 1.0));
                    for value in [position.x, position.y, position.z, normal.x, normal.y, normal.z, uv[0], uv[1]] {
                        vertices.extend_from_slice(&value.to_le_bytes());
                    }
                    vertices.extend_from_slice(&[127, 127, 127, 127]);
                    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    vertices.extend_from_slice(&[byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), byte(alpha)]);
                    if skinned {
                        let mut slots = [0u8; 4];
                        let mut bytes = [0u8; 4];
                        if let Some((bones, weights)) = mesh.skin.get(&key[0]) {
                            let pairs: Vec<(u16, f32)> = bones.iter().zip(weights).filter(|(_, w)| **w > 0.0).filter_map(|(b, w)| bone_slot.get(b).map(|s| (*s, *w))).take(4).collect();
                            let total: f32 = pairs.iter().map(|(_, w)| w).sum();
                            if total > 0.0 {
                                let mut left = 255i32;
                                for (slot, &(bone, weight)) in pairs.iter().enumerate() {
                                    let value = if slot + 1 == pairs.len() { left } else { ((weight / total) * 255.0).round() as i32 };
                                    let value = value.clamp(0, left);
                                    left -= value;
                                    slots[slot] = group_bones.iter().position(|b| *b == bone).unwrap_or(0) as u8;
                                    bytes[slot] = value as u8;
                                    let reach = (position - bone_position(&mesh.bones, bone_order[bone as usize])).length();
                                    culling[bone as usize] = culling[bone as usize].max(reach);
                                }
                            }
                        }
                        envelopes.extend_from_slice(&slots);
                        envelopes.extend_from_slice(&bytes);
                    }
                    vertex_count += 1;
                    vertex_count - 1
                });
                indices.push(index);
            }
        }
        subsets.push([first_face, (indices.len() / 3) as u32 - first_face, first_vertex, vertex_count - first_vertex]);
    }

    let mut names: Vec<u8> = Vec::new();
    let mut bones: Vec<u8> = Vec::new();
    for (slot, &index) in bone_order.iter().enumerate() {
        let bone = mesh.bones.get(index).unwrap();
        let offset = names.len() as u32;
        names.extend_from_slice(bone.name.as_bytes());
        names.push(0);
        let parent = bone.parent.and_then(|p| bone_slot.get(&p).copied()).unwrap_or(0xFFFF);
        bones.extend_from_slice(&offset.to_le_bytes());
        bones.extend_from_slice(&parent.to_le_bytes());
        bones.extend_from_slice(&parent.to_le_bytes());
        bones.extend_from_slice(&culling[slot].to_le_bytes());
        for value in &bone.cframe[3..12] {
            bones.extend_from_slice(&value.to_le_bytes());
        }
        for value in &bone.cframe[0..3] {
            bones.extend_from_slice(&value.to_le_bytes());
        }
    }

    let face_count = (indices.len() / 3) as u32;
    let subset_count = if skinned { subsets.len() as u16 } else { 0 };
    let mut out = Vec::with_capacity(13 + 24 + vertices.len() + envelopes.len() + indices.len() * 4 + 8 + bones.len() + names.len() + subsets.len() * 72);
    out.extend_from_slice(b"version 4.00\n");
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&vertex_count.to_le_bytes());
    out.extend_from_slice(&face_count.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&(bone_order.len() as u16).to_le_bytes());
    out.extend_from_slice(&(names.len() as u32).to_le_bytes());
    out.extend_from_slice(&subset_count.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&vertices);
    out.extend_from_slice(&envelopes);
    for index in &indices {
        out.extend_from_slice(&index.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&face_count.to_le_bytes());
    out.extend_from_slice(&bones);
    out.extend_from_slice(&names);
    if skinned {
        for (subset, (_, group_bones)) in subsets.iter().zip(&groups) {
            for value in subset {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.extend_from_slice(&(group_bones.len() as u32).to_le_bytes());
            for slot in 0..26 {
                out.extend_from_slice(&group_bones.get(slot).copied().unwrap_or(0xFFFF).to_le_bytes());
            }
        }
    }
    out
}

fn bone_position(bones: &Slots<Bone>, index: u32) -> Vec3 {
    bones.get(index).map_or(Vec3::ZERO, |b| Vec3::new(b.cframe[0], b.cframe[1], b.cframe[2]))
}

/// The engine's meshes, the parts bound to them and the meshes changed since their last snapshot.
#[derive(Default)]
pub(crate) struct MeshExtension {
    meshes: RefCell<FxHashMap<InstanceId, Mesh>>,
    users: RefCell<FxHashMap<InstanceId, Vec<InstanceId>>>,
    /// Each bound part's mesh
    bound: RefCell<FxHashMap<InstanceId, InstanceId>>,
    dirty: RefCell<FxHashSet<InstanceId>>,
    names: RefCell<Option<Rc<Names>>>,
    /// Decoded asset meshes by what their content id names ([`asset_key`]; cached per engine for session wait parity)
    #[cfg_attr(not(feature = "meshes"), allow(dead_code))]
    assets: RefCell<FxHashMap<String, Arc<Mesh>>>,
}

impl crate::Extension for MeshExtension {
    fn teardown(&self) {
        self.meshes.borrow_mut().clear();
        self.users.borrow_mut().clear();
        self.bound.borrow_mut().clear();
        self.dirty.borrow_mut().clear();
        self.assets.borrow_mut().clear();
    }
}

fn extension(engine: &Engine) -> &MeshExtension {
    engine.extension_or_insert_with(MeshExtension::default)
}

struct Names {
    mesh_id: NameId,
    mesh_size: NameId,
    initial_size: NameId,
    size: NameId,
    fixed_size: NameId,
    render_fidelity: NameId,
    collision_fidelity: NameId,
    has_skinned_mesh: NameId,
    editable_mesh: crate::ClassId,
    mesh_part: crate::ClassId,
}

fn names(engine: &Engine) -> Option<Rc<Names>> {
    extension(engine).names.borrow().clone()
}

/// A colour's alpha as a mesh keeps it (measured: 1/255 steps, 0.25 reads 64/255).
fn alpha(value: f64) -> f32 {
    ((value.clamp(0.0, 1.0) * 255.0).round() / 255.0) as f32
}

fn err(message: impl Into<String>) -> Flow {
    // (an engine method's error: no script position, as Roblox raises them)
    Flow::Error(ScriptError::plain(message))
}

fn vec3_of(value: &LuaValue) -> Option<Vec3> {
    match convert::from_lua(value) {
        Value::Vector3(v) => Some(Vec3::new(v.x, v.y, v.z)),
        _ => None,
    }
}

fn vec2_of(value: &LuaValue) -> Option<[f32; 2]> {
    match convert::from_lua(value) {
        Value::Vector2(v) => Some([v.x, v.y]),
        _ => None,
    }
}

fn color_of(value: &LuaValue) -> Option<[f32; 3]> {
    match convert::from_lua(value) {
        Value::Color3(c) => Some([c.r, c.g, c.b]),
        _ => None,
    }
}

fn cframe_of(value: &LuaValue) -> Option<[f32; 12]> {
    match convert::from_lua(value) {
        Value::Dom(variant) => match &*variant {
            Variant::CFrame(c) => {
                let o = c.orientation;
                Some([c.position.x, c.position.y, c.position.z, o.x.x, o.x.y, o.x.z, o.y.x, o.y.y, o.y.z, o.z.x, o.z.y, o.z.z])
            }
            _ => None,
        },
        _ => None,
    }
}

fn vector_value(lua: &Lua, engine: &Engine, v: Vec3) -> LuaResult<LuaValue> {
    convert::to_lua(lua, engine, &Value::Vector3(dom::Vector3::new(v.x, v.y, v.z)))
}

fn ids_value(lua: &Lua, ids: impl Iterator<Item = f64>) -> LuaResult<LuaValue> {
    Ok(LuaValue::Table(lua.create_sequence_from(ids)?))
}

/// The array part of `table`, 1 to `#table` (a hole is nil).
fn sequence(table: &LuaTable) -> Vec<LuaValue> {
    (1..=table.raw_len()).map(|index| table.raw_get::<LuaValue>(index).unwrap_or(LuaValue::Nil)).collect()
}

/// The vertex a BatchAdd face names (measured: the client's errors for an id of another kind, a number that is no id,
/// and a vertex that is gone).
fn face_vertex(m: &Mesh, value: &LuaValue) -> Result<u32, Flow> {
    let id = match value {
        LuaValue::Number(n) => id_of_number(*n),
        LuaValue::Integer(n) => i64::from(*n),
        _ => 0,
    };
    if parts(id).0 != VERTEX {
        return Err(wrong_kind(VERTEX, id));
    }
    m.positions.at(VERTEX, id)
}

/// A table's array part as numbers (other values dropped).
fn numbers_of(value: &LuaValue) -> Vec<f64> {
    match value {
        LuaValue::Table(table) => table.sequence_values::<f64>().filter_map(Result::ok).collect(),
        _ => Vec::new(),
    }
}

/// A table's array part as ids, unreadable values dropped.
fn ids_of(value: &LuaValue) -> Vec<i64> {
    numbers_of(value).into_iter().map(id_of_number).collect()
}

/// The slots of the live ids of `kind` among `ids`.
fn slots_of<T>(slots: &Slots<T>, kind: u64, ids: &[i64]) -> Vec<u32> {
    ids.iter().filter_map(|&id| slots.find(kind, id)).collect()
}

/// An int64 argument as the client casts one (measured: a number rounds half away from zero, a numeric string reads as
/// its number and a boolean as 1 or 0; another string is "Unable to cast string to int64", nil "Argument 2 missing or
/// nil"; other values unmeasured, cast errors naming their type).
fn int64_arg(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<i64, Flow> {
    let value = args.get(at);
    let number = match value {
        LuaValue::Nil => return Err(err(format!("Argument {at} missing or nil"))),
        LuaValue::Integer(n) => return Ok(i64::from(*n)),
        LuaValue::Number(n) => *n,
        LuaValue::Boolean(b) => return Ok(*b as i64),
        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten().ok_or_else(|| err("Unable to cast string to int64"))?,
        other => return Err(err(format!("Unable to cast {} to int64", crate::lua::typeof_value(engine, other)))),
    };
    Ok(id_of_number(number.round()))
}

/// Argument `at`, an id of `kind` (the client's cast, then its checks: 0 is "Invalid id" and an id of another kind is
/// named; whether its slot is live is the caller's [`Slots::at`]).
fn id_arg(lua: &Lua, engine: &Engine, args: &Args, at: usize, kind: u64) -> Result<i64, Flow> {
    let id = int64_arg(lua, engine, args, at)?;
    check_kind(id, kind)?;
    Ok(id)
}

/// The client's first checks of an id it reads: 0 is "Invalid id", an id of another kind is named.
fn check_kind(id: i64, kind: u64) -> Result<(), Flow> {
    if id == 0 {
        return Err(err("Invalid id"));
    }
    if parts(id).0 != kind {
        return Err(wrong_kind(kind, id));
    }
    Ok(())
}

/// Element `index` (from 1) of an id array a per-vertex batch reads: a number, a numeric string, else "<method>: <name>
/// at index <index> is not a number" (measured; an id plus 0.5 reads the id).
fn element_id(lua: &Lua, value: &LuaValue, method: &str, name: &str, index: usize) -> Result<i64, Flow> {
    let number = match value {
        LuaValue::Integer(n) => return Ok(i64::from(*n)),
        LuaValue::Number(n) => Some(*n),
        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten(),
        _ => None,
    };
    number.map(id_of_number).ok_or_else(|| err(format!("{method}: {name} at index {index} is not a number")))
}

/// The refusal of every add and remove on a fixed-size mesh (measured in a 0.747 client: AddVertex, AddNormal, AddUV,
/// AddColor, AddTriangle, AddBone, BatchAdd, RemoveFace, BatchRemove, RemoveUnused and RemoveBone, each before its other
/// checks, BatchAdd and BatchRemove of an empty array too).
fn refuse_fixed(engine: &Engine, id: InstanceId) -> Result<(), Flow> {
    if fixed(engine, id) {
        return Err(err("EditableMesh is fixed-size, adding or removing elements is not allowed in-experience"));
    }
    Ok(())
}

/// A MeshAttribute argument (measured: an item of another enum is "Invalid type 'KeyCode' for enum 'MeshAttribute'",
/// anything else, its name as a string included, "Invalid value for enum 'MeshAttribute'"): Vertex 0, Normal 1, Color 2,
/// UV 3, Face 4.
fn mesh_attribute(engine: &Engine, value: &LuaValue) -> Result<u32, Flow> {
    match convert::from_lua(value) {
        Value::EnumItem(item) => {
            let kind = engine.reflection.enum_item_type_name(item);
            if kind == "MeshAttribute" {
                Ok(engine.reflection.enum_item_info(item).value as u32)
            } else {
                Err(err(format!("Invalid type '{kind}' for enum 'MeshAttribute'")))
            }
        }
        _ => Err(err("Invalid value for enum 'MeshAttribute'")),
    }
}

/// Runs `f` on the mesh `id`; `changed` marks it for a new snapshot, also when `f` raises (a batch that raised part way
/// keeps what it set before).
fn with_mesh<T>(engine: &Engine, id: InstanceId, changed: bool, f: impl FnOnce(&mut Mesh) -> Result<T, Flow>) -> Result<T, Flow> {
    let mut f = Some(f);
    let mut out: Option<T> = None;
    with_mesh_dyn(engine, id, changed, &mut |mesh| {
        out = Some((f.take().expect("called once"))(mesh)?);
        Ok(())
    })?;
    Ok(out.expect("set on Ok"))
}

#[inline(never)]
fn with_mesh_dyn(engine: &Engine, id: InstanceId, changed: bool, f: &mut dyn FnMut(&mut Mesh) -> Result<(), Flow>) -> Result<(), Flow> {
    let ext = extension(engine);
    let result = {
        let mut meshes = ext.meshes.borrow_mut();
        let mesh = meshes.get_mut(&id).ok_or_else(|| err("This EditableMesh has been destroyed"))?;
        f(mesh)
    };
    if changed {
        ext.dirty.borrow_mut().insert(id);
    }
    result
}

fn fixed(engine: &Engine, id: InstanceId) -> bool {
    let Some(names) = names(engine) else { return false };
    engine.state.borrow().dom.boolean(id, names.fixed_size).unwrap_or(false)
}

fn new_mesh(lua: &Lua, engine: &Rc<Engine>, mesh: Mesh, fixed_size: bool) -> Ret {
    let id = new_mesh_id(engine, mesh, fixed_size)?;
    Ok(values([instance_value(lua, engine, id)?]))
}

fn new_mesh_id(engine: &Rc<Engine>, mesh: Mesh, fixed_size: bool) -> Result<InstanceId, Flow> {
    let names = names(engine).ok_or_else(|| err("EditableMesh is not available"))?;
    let id = {
        let mut state = engine.state.borrow_mut();
        let id = core::create(&mut state, &engine.reflection, names.editable_mesh);
        core::raw_set_silent(&mut state, &engine.reflection, id, names.fixed_size, Value::Bool(fixed_size));
        id
    };
    extension(engine).meshes.borrow_mut().insert(id, mesh);
    Ok(id)
}

/// `options.FixedSize` of a CreateEditableMeshAsync options table: true unless it says false (measured in a 0.747 client
/// for an EditableMesh's Content: no options make a fixed-size copy). CreateEditableMesh takes none: a mesh it makes is
/// never fixed-size (measured: `{ FixedSize = true }` reads false and takes vertices).
fn fixed_option(value: &LuaValue) -> bool {
    match value {
        LuaValue::Table(table) => table.get::<Option<bool>>("FixedSize").ok().flatten().unwrap_or(true),
        _ => true,
    }
}

/// The EditableMesh a Content holds (Content.fromObject), if any.
fn content_object(engine: &Engine, value: &LuaValue) -> Option<InstanceId> {
    match convert::from_lua(value) {
        Value::Content(content) => match &*content {
            ContentValue::Object(id) => extension(engine).meshes.borrow().contains_key(id).then_some(*id),
            _ => None,
        },
        _ => None,
    }
}

fn content_uri(value: &LuaValue) -> Option<String> {
    match convert::from_lua(value) {
        // (an Opaque Content's copy: CreateDataModelContentAsync)
        Value::Content(content) => content.location().map(|uri| uri.to_string()),
        Value::String(uri) => Some(uri.to_string()),
        Value::Number(id) if id > 0.0 => Some(format!("rbxassetid://{}", id as u64)),
        _ => None,
    }
}

/// What the mesh content id `uri` names, as the decoded meshes are kept by: a local file's path, else the asset version
/// a client reads ([`crate::content_id::asset`]: two spellings of one version are one mesh), else the id as written
/// (each kind marked, so `rbxassetid://N?version=2`, which loads nothing, is not version 2's key).
fn asset_key(engine: &Engine, uri: &str) -> String {
    match engine.options.local_file(uri) {
        Some(path) => format!("file {}", path.display()),
        None => crate::content_id::asset(uri).map_or_else(|| format!("unread {uri}"), |asset| format!("asset {}", asset.file_stem())),
    }
}

/// Decoded mesh files kept per process by path and stamp (so warm services and batch workers decode each once).
#[cfg(feature = "meshes")]
static KEPT_ASSETS: OnceLock<Mutex<FxHashMap<PathBuf, (files::Stamp, Arc<Mesh>)>>> = OnceLock::new();

/// Drops cached decoded mesh assets whose files changed or disappeared since they were decoded.
#[cfg(feature = "meshes")]
pub(crate) fn forget_stale_meshes() {
    if let Some(cache) = KEPT_ASSETS.get() {
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|path, (stamp, _)| files::stamp(path) == Some(*stamp));
        }
    }
}

/// The mesh asset `uri`, decoded (cached per engine and per process by file stamp).
#[cfg(feature = "meshes")]
fn load_asset(engine: &Engine, uri: &str) -> Result<Arc<Mesh>, String> {
    let key = asset_key(engine, uri);
    if let Some(mesh) = extension(engine).assets.borrow().get(&key) {
        return Ok(mesh.clone());
    }
    // a local file (a relative one the project's), else an asset downloaded once: `<state dir>/meshes/<id>`, a pinned
    // version `<id>-v<version>`
    let path = match engine.options.local_file(uri) {
        Some(path) => path,
        None => {
            let asset = crate::content_id::asset(uri).ok_or_else(|| format!("{uri}: not an asset id"))?;
            let cache = crate::state_dir(&engine.options.repo).join("meshes");
            let cached = cache.join(asset.file_stem());
            if !files::is_file(&cached) {
                if let Err(error) = crate::animation::source::fetch(engine, uri, asset, &cache, &cached) {
                    files::note_unchecked("a mesh failed to load");
                    return Err(error);
                }
            }
            cached
        }
    };
    files::note(&path);
    let stamp = files::stamp(&path);
    if let Some(stamp) = stamp {
        if let Some(mesh) = KEPT_ASSETS.get().and_then(|cache| cache.lock().ok()?.get(&path).filter(|(kept, _)| *kept == stamp).map(|(_, mesh)| mesh.clone())) {
            extension(engine).assets.borrow_mut().insert(key, mesh.clone());
            return Ok(mesh);
        }
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let file = ror_mesh::decode_mesh(&bytes).map_err(|error| format!("{uri}: {error}"))?;
    let mesh = crate::alloc::long_lived(|| Arc::new(Mesh::from_file(&file)));
    if let Some(stamp) = stamp {
        let cache = KEPT_ASSETS.get_or_init(Default::default);
        if let Ok(mut cache) = cache.lock() {
            crate::alloc::long_lived(|| cache.insert(path, (stamp, mesh.clone())));
        }
    }
    extension(engine).assets.borrow_mut().insert(key, mesh.clone());
    Ok(mesh)
}

#[cfg(not(feature = "meshes"))]
fn load_asset(_: &Engine, uri: &str) -> Result<Arc<Mesh>, String> {
    Err(format!("{uri}: this build decodes no mesh assets (feature `meshes`)"))
}

/// Writes `mesh`'s snapshot (named by its content) and returns its path.
fn snapshot(engine: &Engine, mesh: &Mesh) -> Result<PathBuf, String> {
    let bytes = encode(mesh);
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    let directory = crate::state_dir(&engine.options.repo).join("editable-meshes");
    let path = directory.join(format!("{:016x}-{}.mesh", hasher.finish(), bytes.len()));
    if !files::is_file(&path) {
        std::fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
        if let Err(error) = files::write_atomic(&path, &bytes) {
            if !files::is_file(&path) {
                return Err(format!("{}: {error}", path.display()));
            }
        }
    }
    files::note(&path);
    Ok(path)
}

fn content_of(path: &std::path::Path) -> Value {
    Value::String(Str::from(format!("file://{}", path.display()).as_str()))
}

fn size_value(v: Vec3) -> Value {
    Value::Vector3(dom::Vector3::new(v.x, v.y, v.z))
}

/// Binds `part` to `mesh` (its later snapshots go to the part) and points it at `content`.
fn bind(engine: &Engine, part: InstanceId, mesh: Option<InstanceId>) {
    let ext = extension(engine);
    if let Some(old) = ext.bound.borrow_mut().remove(&part) {
        if let Some(users) = ext.users.borrow_mut().get_mut(&old) {
            users.retain(|&u| u != part);
        }
    }
    if let Some(mesh) = mesh {
        ext.bound.borrow_mut().insert(part, mesh);
        let mut users = ext.users.borrow_mut();
        let list = users.entry(mesh).or_default();
        if !list.contains(&part) {
            list.push(part);
        }
    }
}

/// Points every part bound to a mesh changed since its last snapshot at a new snapshot. The recorder
/// calls it before each sample.
pub fn flush(engine: &Engine) {
    let ext = extension(engine);
    let dirty: Vec<InstanceId> = ext.dirty.borrow_mut().drain().collect();
    if dirty.is_empty() {
        return;
    }
    let Some(names) = names(engine) else { return };
    for mesh_id in dirty {
        let users: Vec<InstanceId> = ext.users.borrow().get(&mesh_id).cloned().unwrap_or_default();
        let users: Vec<InstanceId> = {
            let state = engine.state.borrow();
            users.into_iter().filter(|&u| state.dom.get(u).is_some()).collect()
        };
        if users.is_empty() {
            continue;
        }
        let path = {
            let meshes = ext.meshes.borrow();
            let Some(mesh) = meshes.get(&mesh_id) else { continue };
            snapshot(engine, mesh)
        };
        match path {
            Ok(path) => {
                let hooks = engine.hooks.borrow();
                let mut state = engine.state.borrow_mut();
                for part in users {
                    core::raw_set(&mut state, &engine.reflection, &hooks, part, names.mesh_id, content_of(&path));
                }
            }
            Err(problem) => engine.warn(format!("EditableMesh: its snapshot could not be written ({problem})")),
        }
    }
}

/// AssetService:CreateDataModelContentAsync of an EditableMesh's Content: a FileMesh copy of the mesh as it is now (the
/// snapshot file, which later edits do not reach), its `file://` URI. A mesh without faces is refused as a 0.747 client
/// refuses an empty one.
pub fn data_model_copy(engine: &Engine, id: InstanceId) -> Result<String, String> {
    let ext = extension(engine);
    let meshes = ext.meshes.borrow();
    let mesh = meshes.get(&id).ok_or_else(|| "This EditableMesh has been destroyed".to_string())?;
    if mesh.faces.live().next().is_none() {
        return Err("Error creating FileMeshData: Empty Result".into());
    }
    let path = snapshot(engine, mesh)?;
    Ok(format!("file://{}", path.display()))
}

/// A new MeshPart showing `mesh` (CreateMeshPartAsync of an EditableMesh's Content).
fn mesh_part(lua: &Lua, engine: &Rc<Engine>, mesh_id: InstanceId, options: &LuaValue) -> Ret {
    let names = names(engine).ok_or_else(|| err("MeshPart is not available"))?;
    let (path, size, skinned) = {
        let ext = extension(engine);
        let meshes = ext.meshes.borrow();
        let mesh = meshes.get(&mesh_id).ok_or_else(|| err("This EditableMesh has been destroyed"))?;
        // Studio's error for a mesh too wide for a part, before anything is made (the module's doc)
        if mesh.bounds().is_some_and(|(min, max)| (max - min).max_element() > MAX_EXTENT) {
            return Err(err(EXTENTS_TOO_LARGE));
        }
        (snapshot(engine, mesh).map_err(|problem| err(format!("CreateMeshPartAsync: {problem}")))?, mesh.size(), mesh.skinned())
    };
    let part = {
        let hooks = engine.hooks.borrow();
        let mut state = engine.state.borrow_mut();
        let reflection = &engine.reflection;
        let part = core::create(&mut state, reflection, names.mesh_part);
        core::raw_set(&mut state, reflection, &hooks, part, names.mesh_id, content_of(&path));
        for name in [names.mesh_size, names.initial_size, names.size] {
            core::raw_set(&mut state, reflection, &hooks, part, name, size_value(size));
        }
        core::raw_set(&mut state, reflection, &hooks, part, names.has_skinned_mesh, Value::Bool(skinned));
        if let LuaValue::Table(table) = options {
            for (key, name) in [("RenderFidelity", names.render_fidelity), ("CollisionFidelity", names.collision_fidelity)] {
                if let Ok(value) = table.get::<LuaValue>(key) {
                    if let Value::EnumItem(item) = convert::from_lua(&value) {
                        core::raw_set(&mut state, reflection, &hooks, part, name, Value::EnumItem(item));
                    }
                }
            }
        }
        part
    };
    bind(engine, part, Some(mesh_id));
    Ok(values([instance_value(lua, engine, part)?]))
}

/// MeshPart.MeshSize: what it was given, else the mesh's size a file stores as InitialSize (files
/// keep no MeshSize; Roblox reads it from the mesh), else Size when it has a mesh, else 0, 0, 0
fn mesh_size_of(state: &State, names: &Names, id: InstanceId) -> Value {
    let vector = |value: Option<&Value>| match value {
        Some(Value::Vector3(v)) if v.x > 0.0 || v.y > 0.0 || v.z > 0.0 => Some(Value::Vector3(*v)),
        _ => None,
    };
    let from_file = || {
        state.dom.at(id).unmodeled.as_ref().and_then(|unmodeled| {
            unmodeled.iter().find(|(name, _)| *name == "InitialSize").and_then(|(_, value)| match value {
                Variant::Vector3(v) => Some(Value::Vector3(v)),
                _ => None,
            })
        })
    };
    // (a part with no mesh reads 0, 0, 0, whatever its Size: measured in a 0.747 client, tools/parity/probes/
    // editable_mesh_texts.luau; one with a mesh that gave no size, its Size)
    let meshed = matches!(state.dom.raw_get(id, names.mesh_id), Some(Value::String(uri)) if !uri.is_empty());
    vector(state.dom.raw_get(id, names.mesh_size))
        .or_else(|| vector(state.dom.raw_get(id, names.initial_size)))
        .or_else(from_file)
        .or_else(|| meshed.then(|| state.dom.raw_get(id, names.size).cloned()).flatten())
        .unwrap_or(Value::Vector3(dom::Vector3::new(0.0, 0.0, 0.0)))
}

/// The id of what corner `corner` of `face` uses of `kind` (VERTEX, NORMAL, COLOR or UV).
fn corner_id(m: &Mesh, face: &Face, corner: usize, kind: u64) -> f64 {
    match kind {
        VERTEX => m.positions.id(kind, face.vertices[corner]),
        NORMAL => m.normals.id(kind, face.normals[corner]),
        COLOR => m.colors.id(kind, face.colors[corner]),
        _ => m.uvs.id(kind, face.uvs[corner]),
    }
}

/// For a corner over a vertex (face slot `f`, corner `corner`): its face's id for FACE, else what it uses of `kind`.
fn vertex_corner_id(m: &Mesh, f: u32, corner: usize, kind: u64) -> f64 {
    if kind == FACE {
        return m.faces.id(FACE, f);
    }
    corner_id(m, m.faces.get(f).expect("live"), corner, kind)
}

/// The live normal, colour or uv slot `id` names ("Invalid id" else).
fn attribute_slot(m: &Mesh, kind: u64, id: i64) -> Result<u32, Flow> {
    match kind {
        NORMAL => m.normals.at(kind, id),
        COLOR => m.colors.at(kind, id),
        _ => m.uvs.at(kind, id),
    }
}

/// The corner of face `f` over vertex `v` a single-item corner method names: the face checked (0 and gone "Invalid id",
/// another kind named), then its first corner over the vertex, the vertex id never checked (measured).
fn face_corner(m: &Mesh, v: i64, f: i64) -> Result<(u32, usize), Flow> {
    let f = m.faces.at(FACE, f)?;
    let vertex = m.positions.find(VERTEX, v);
    let face = m.faces.get(f).expect("live");
    let corner = face.vertices[..face.len()].iter().position(|&x| Some(x) == vertex);
    Ok((f, corner.ok_or_else(|| err("vertexId not found on requested face"))?))
}

/// Pair `index` (from 1) of a per-vertex batch's vertexIds and faceIds, read as numbers (measured: both are read before
/// either is checked).
fn read_pair(lua: &Lua, method: &str, index: usize, vertex: &LuaValue, face: &LuaValue) -> Result<(i64, i64), Flow> {
    Ok((element_id(lua, vertex, method, "vertexId", index)?, element_id(lua, face, method, "faceId", index)?))
}

/// The corner a read pair names: the vertex checked (0 and gone "Invalid id", another kind named), then the face, then its
/// first corner over the vertex ("<method>: vertexId not found on requested face at index <index>"), as measured.
fn pair_corner(m: &Mesh, method: &str, index: usize, v: i64, f: i64) -> Result<(u32, usize), Flow> {
    check_kind(v, VERTEX)?;
    let v = m.positions.at(VERTEX, v)?;
    check_kind(f, FACE)?;
    let f = m.faces.at(FACE, f)?;
    let face = m.faces.get(f).expect("live");
    let corner = face.vertices[..face.len()].iter().position(|&x| x == v);
    Ok((f, corner.ok_or_else(|| err(format!("{method}: vertexId not found on requested face at index {index}")))?))
}

/// Whether the running C function was called by its name (read as a value, `Mesh.AddFace(Mesh, ...)`) rather than through
/// `__namecall` (`Mesh:AddFace(...)`): a Luau-style argument error names the function only then (lua/services/query.rs).
fn called_by_name(lua: &Lua, method: &str) -> bool {
    let frame = |level: usize| lua.inspect_stack(level, |debug| (debug.names().name.map(|name| name.into_owned()), debug.source().what == "C"));
    match frame(0) {
        Some((Some(name), _)) if name == method => true,
        Some((Some(name), _)) if name == "__namecall" => false,
        _ => matches!(frame(1), Some((Some(name), true)) if name == method),
    }
}

/// Argument `at` (1 is the first after self) of a method a client binds the Luau way (AddFace, GetFacesWith*,
/// GetVerticesWith*): a number, rounded half away from zero to an int64, else "invalid argument #<at + 1> (int64 expected,
/// got <typeof>)" from the calling line, " to '<method>'" after the number when the method was read as a value (measured in a
/// 0.747 client: a numeric string is refused too).
fn luau_int64(lua: &Lua, engine: &Engine, args: &Args, at: usize, method: &str) -> Result<i64, Flow> {
    match args.get(at) {
        LuaValue::Integer(n) => Ok(i64::from(*n)),
        LuaValue::Number(n) => Ok(id_of_number(n.round())),
        other => {
            let to = if called_by_name(lua, method) { format!(" to '{method}'") } else { String::new() };
            crate::lua::raise_at_caller(format!("invalid argument #{}{to} (int64 expected, got {})", at + 1, crate::lua::typeof_value(engine, other)))
        }
    }
}

/// A Vector3 argument as a client casts one: nil is "Argument <at> missing or nil", anything else that is no Vector3 "Unable
/// to cast <type> to Vector3".
fn vector3_arg(engine: &Engine, args: &Args, at: usize) -> Result<Vec3, Flow> {
    match args.get(at) {
        LuaValue::Nil => Err(err(format!("Argument {at} missing or nil"))),
        value => vec3_of(value).ok_or_else(|| err(format!("Unable to cast {} to Vector3", crate::services::tween::cast_name(engine, value)))),
    }
}

/// A float argument as a client casts one: a number, a numeric string, a boolean as 1 or 0; nil "Argument <at> missing or
/// nil", another string "Unable to cast string to float", anything else "Unable to cast <type> to float".
fn float_arg(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<f32, Flow> {
    let value = args.get(at);
    match value {
        LuaValue::Nil => Err(err(format!("Argument {at} missing or nil"))),
        LuaValue::Integer(n) => Ok(*n as f32),
        LuaValue::Number(n) => Ok(*n as f32),
        LuaValue::Boolean(on) => Ok(f32::from(u8::from(*on))),
        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten().map(|n| n as f32).ok_or_else(|| err("Unable to cast string to float")),
        other => Err(err(format!("Unable to cast {} to float", crate::services::tween::cast_name(engine, other)))),
    }
}

/// A bool argument as a client casts one: a boolean, a number (not 0 is true); nil "Argument <at> missing or nil", anything
/// else "Unable to cast <type> to bool" (measured: a string is refused).
fn bool_arg(engine: &Engine, args: &Args, at: usize) -> Result<bool, Flow> {
    match args.get(at) {
        LuaValue::Nil => Err(err(format!("Argument {at} missing or nil"))),
        LuaValue::Boolean(on) => Ok(*on),
        LuaValue::Integer(n) => Ok(*n != 0),
        LuaValue::Number(n) => Ok(*n != 0.0),
        other => Err(err(format!("Unable to cast {} to bool", crate::services::tween::cast_name(engine, other)))),
    }
}

/// A CFrame argument as a client casts one: nil "Argument <at> missing or nil", anything else that is no CFrame "Unable to
/// cast <type> to CoordinateFrame".
fn cframe_arg(engine: &Engine, args: &Args, at: usize) -> Result<[f32; 12], Flow> {
    match args.get(at) {
        LuaValue::Nil => Err(err(format!("Argument {at} missing or nil"))),
        value => cframe_of(value).ok_or_else(|| err(format!("Unable to cast {} to CoordinateFrame", crate::services::tween::cast_name(engine, value)))),
    }
}

/// An array argument as a client casts one: nil "Argument <at> missing or nil", anything else that is no table "Unable to cast
/// <type> to Array".
fn array_arg(engine: &Engine, args: &Args, at: usize) -> Result<Vec<LuaValue>, Flow> {
    match args.get(at) {
        LuaValue::Nil => Err(err(format!("Argument {at} missing or nil"))),
        LuaValue::Table(table) => Ok(sequence(table)),
        value => Err(err(format!("Unable to cast {} to Array", crate::services::tween::cast_name(engine, value)))),
    }
}

/// An element of an id array a setter reads as the client reads one: a number truncated as [`id_of_number`], a numeric string
/// as its number, anything else as 0 (which the checks then name as no id: measured, "x" in SetFaceVertices' ids is "Invalid
/// id").
fn loose_id(lua: &Lua, value: &LuaValue) -> i64 {
    match value {
        LuaValue::Integer(n) => i64::from(*n),
        LuaValue::Number(n) => id_of_number(*n),
        LuaValue::String(_) => lua.coerce_number(value.clone()).ok().flatten().map_or(0, id_of_number),
        _ => 0,
    }
}

/// The vertex slot a vertex id of AddTriangle and AddFace names, as a client reads one (measured in a 0.747 client): a live
/// vertex's; a live normal's id reads as the vertex in the normal's slot (the client does not check that it names a vertex);
/// kind 7 raises "Unexpected Attribute Type"; anything else, ids of the other kinds included, "Unable to add new triangle,
/// vertex id <id> is invalid".
fn triangle_vertex(m: &Mesh, id: i64) -> Result<u32, Flow> {
    let invalid = || err(format!("Unable to add new triangle, vertex id {id} is invalid"));
    match parts(id).0 {
        7 => Err(err("Unexpected Attribute Type")),
        VERTEX => m.positions.find(VERTEX, id).ok_or_else(invalid),
        NORMAL => m.normals.find(NORMAL, id).filter(|&slot| m.positions.get(slot).is_some()).ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

/// A bone name as a client takes one (measured in a 0.747 client): cut at its first NUL byte (a C string), "EditableMesh Error:
/// Bone name missing" when that is empty, "EditableMesh Error: Bone name too long" past 100 bytes, "EditableMesh Error: Bone name
/// already exists in the mesh" when another bone has it.
fn bone_name(m: &Mesh, bytes: &[u8], bone: Option<u32>) -> Result<String, Flow> {
    let bytes = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
    if bytes.is_empty() {
        return Err(err("EditableMesh Error: Bone name missing"));
    }
    if bytes.len() > 100 {
        return Err(err("EditableMesh Error: Bone name too long"));
    }
    let name = String::from_utf8_lossy(bytes).into_owned();
    if m.bones.live().any(|(slot, other)| Some(slot) != bone && other.name == name) {
        return Err(err("EditableMesh Error: Bone name already exists in the mesh"));
    }
    Ok(name)
}

/// The kind letters IdDebugString writes (measured: kind 0 "?", kind 7 nothing).
const KIND_LETTERS: [&str; 8] = ["?", "v", "n", "c", "u", "f", "b", ""];

/// EditableMesh:IdDebugString of `id` (measured in a 0.747 client): "Invalid" for 0 (and an id past the int64 range), else its
/// kind's letter, the id's low 32 bits and, when its generation is not 0, a comma and the generation plus 1 (`f3`, `f3,2`;
/// generations from 65,536 on read otherwise there, not followed).
fn id_debug_string(id: i64) -> String {
    if id == 0 || id == i64::MIN {
        return "Invalid".into();
    }
    let (kind, generation, low) = parts(id);
    let mut text = format!("{}{low}", KIND_LETTERS[kind as usize]);
    if generation != 0 {
        text.push_str(&format!(",{}", (generation + 1) & 0x1FFF_FFFF));
    }
    text
}

/// Ericson's closest point of triangle (a, b, c) to `p` (Real-Time Collision Detection 5.1.5), in single precision as a client
/// computes it (measured: the points FindClosestPointOnSurface answers, bit for bit on 14 of 14).
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// The barycentric weights of `p` in triangle (a, b, c), the weight of a first (Ericson 3.4). A client's own differ from these
/// in their last bits (within 2e-7 on the measured points, not followed).
fn barycentric(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let denom = d00 * d11 - d01 * d01;
    if denom == 0.0 {
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    Vec3::new(1.0 - v - w, v, w)
}

/// Where a ray from `origin` along `direction` meets triangle (a, b, c) (Moller-Trumbore, both sides, edges included), as a
/// fraction of `direction`: a client hits from 0 up to but not including 1 (measured: a ray whose end touches the face misses).
fn ray_triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inverse = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inverse;
    (0.0..1.0).contains(&t).then_some(t)
}

/// A FacsActionUnit argument as a client casts one: an item of FacsActionUnit, an item's name or its value; nil "Argument <at>
/// missing or nil", an item of another enum "Unable to cast <Enum> to FacsActionUnit", another string "Unable to cast string
/// to FacsActionUnit".
fn facs_unit(lua: &Lua, engine: &Engine, value: &LuaValue, at: usize) -> Result<u32, Flow> {
    if value.is_nil() {
        return Err(err(format!("Argument {at} missing or nil")));
    }
    let facs = engine.reflection.enum_id("FacsActionUnit").ok_or_else(|| err("FacsActionUnit is not available"))?;
    match convert::coerce_enum_arg(lua, engine, facs, value) {
        Some(item) => Ok(engine.reflection.enum_item_info(item).value as u32),
        None => Err(err(format!("Unable to cast {} to FacsActionUnit", crate::services::tween::cast_name(engine, value)))),
    }
}

/// The FacsActionUnit item of value `value`.
fn facs_item(engine: &Engine, value: u32) -> Option<Value> {
    let facs = engine.reflection.enum_id("FacsActionUnit")?;
    engine.reflection.enums[facs as usize].by_value.get(&i64::from(value)).map(|&item| Value::EnumItem(item))
}

/// The actions of a corrective pose, sorted (measured: 2 or 3, "Correctives must have 2 or 3 FACS controls", no action twice,
/// "Unable to use actions together in a corrective"; a pose is found whatever order its actions are given in).
fn corrective_actions(lua: &Lua, engine: &Engine, args: &Args, at: usize) -> Result<Vec<u32>, Flow> {
    let items = array_arg(engine, args, at)?;
    let mut actions = Vec::with_capacity(items.len());
    for item in &items {
        actions.push(facs_unit(lua, engine, item, at)?);
    }
    if !(2..=3).contains(&actions.len()) {
        return Err(err("Correctives must have 2 or 3 FACS controls"));
    }
    actions.sort_unstable();
    if actions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(err("Unable to use actions together in a corrective"));
    }
    Ok(actions)
}

/// A FACS pose's bones and CFrames from SetFacsPose's / SetFacsCorrectivePose's arrays (measured in a 0.747 client: the arrays the
/// same size, "boneIds and cframes size mismatch (<n> and <m>)"; each pair in turn: the id a number ("Variant cast failed"), a
/// live bone (0 and gone "Invalid id", another kind named), a virtual one ('Only virtual bones can be used in Facs Poses, unable
/// to use bone "<name>", id <id>'), then its CFrame ("Variant cast failed"); a bone given twice composes its CFrames).
fn facs_pose(m: &Mesh, ids: &[LuaValue], frames: &[LuaValue]) -> Result<Vec<(u32, [f32; 12])>, Flow> {
    if ids.len() != frames.len() {
        return Err(err(format!("boneIds and cframes size mismatch ({} and {})", ids.len(), frames.len())));
    }
    let mut pose: Vec<(u32, [f32; 12])> = Vec::with_capacity(ids.len());
    for (id, frame) in ids.iter().zip(frames) {
        let id = match id {
            LuaValue::Integer(n) => i64::from(*n),
            LuaValue::Number(n) => id_of_number(n.round()),
            _ => return Err(err("Variant cast failed")),
        };
        let bone = facs_bone(m, id)?;
        let frame = cframe_of(frame).ok_or_else(|| err("Variant cast failed"))?;
        match pose.iter_mut().find(|(slot, _)| *slot == bone) {
            Some((_, held)) => *held = compose(*held, frame),
            None => pose.push((bone, frame)),
        }
    }
    Ok(pose)
}

/// A bone a FACS pose may use: a live virtual one (see [`facs_pose`]).
fn facs_bone(m: &Mesh, id: i64) -> Result<u32, Flow> {
    check_kind(id, BONE)?;
    let slot = m.bones.at(BONE, id)?;
    let bone = m.bones.get(slot).expect("live");
    if !bone.virtual_bone {
        return Err(err(format!("Only virtual bones can be used in Facs Poses, unable to use bone \"{}\", id {id}", bone.name)));
    }
    Ok(slot)
}

const IDENTITY: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// `a * b` of two CFrames as components (position, then the rotation row by row).
fn compose(a: [f32; 12], b: [f32; 12]) -> [f32; 12] {
    let r = |m: &[f32; 12], row: usize, col: usize| m[3 + row * 3 + col];
    let mut out = [0f32; 12];
    for row in 0..3 {
        out[row] = a[row] + r(&a, row, 0) * b[0] + r(&a, row, 1) * b[1] + r(&a, row, 2) * b[2];
        for col in 0..3 {
            out[3 + row * 3 + col] = r(&a, row, 0) * r(&b, 0, col) + r(&a, row, 1) * r(&b, 1, col) + r(&a, row, 2) * r(&b, 2, col);
        }
    }
    out
}

/// A pose as kept: its identity CFrames dropped (measured: a pose of identities reads as none), sorted by bone id.
fn settle_pose(m: &Mesh, mut pose: Vec<(u32, [f32; 12])>) -> Vec<(u32, [f32; 12])> {
    pose.retain(|(_, frame)| *frame != IDENTITY);
    crate::sort::by_key(&mut pose, |(slot, _)| m.bones.id(BONE, *slot) as u64);
    pose
}

fn cframe_value(c: &[f32; 12]) -> Value {
    let cframe = dom::CFrame::new(dom::Vector3::new(c[0], c[1], c[2]), dom::Matrix3::new(dom::Vector3::new(c[3], c[4], c[5]), dom::Vector3::new(c[6], c[7], c[8]), dom::Vector3::new(c[9], c[10], c[11])));
    Value::Dom(Rc::new(Variant::CFrame(cframe)))
}

/// A pose as GetFacsPose answers it: its bones' ids and their CFrames, two arrays.
fn pose_values(lua: &Lua, engine: &Engine, m: &Mesh, pose: Option<&Vec<(u32, [f32; 12])>>) -> LuaResult<LuaMultiValue> {
    let ids = lua.create_table()?;
    let frames = lua.create_table()?;
    for (index, (slot, frame)) in pose.into_iter().flatten().enumerate() {
        ids.raw_set(index + 1, m.bones.id(BONE, *slot))?;
        frames.raw_set(index + 1, convert::to_lua(lua, engine, &cframe_value(frame))?)?;
    }
    Ok(LuaMultiValue::from_iter([LuaValue::Table(ids), LuaValue::Table(frames)]))
}

/// A method's second argument a string parameter (services::strings)
const SECOND: &[crate::services::StringParam] = &[crate::services::StringParam::at(2)];

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let n = |name: &str| reflection.name_id(name);
    let (Some(mesh_id), Some(mesh_size), Some(initial_size), Some(size), Some(fixed_size), Some(render_fidelity), Some(collision_fidelity), Some(has_skinned_mesh), Some(editable_mesh), Some(mesh_part_class)) = (
        n("MeshId"),
        n("MeshSize"),
        n("InitialSize"),
        n("Size"),
        n("FixedSize"),
        n("RenderFidelity"),
        n("CollisionFidelity"),
        n("HasSkinnedMesh"),
        reflection.class_id("EditableMesh"),
        reflection.class_id("MeshPart"),
    ) else {
        return;
    };
    *extension(engine).names.borrow_mut() = Some(Rc::new(Names {
        mesh_id,
        mesh_size,
        initial_size,
        size,
        fixed_size,
        render_fidelity,
        collision_fidelity,
        has_skinned_mesh,
        editable_mesh,
        mesh_part: mesh_part_class,
    }));

    // A freed mesh or part drops its data and bindings
    let freed = Rc::downgrade(engine);
    engine.hooks.borrow_mut().instance_freed.push(Box::new(move |_, id| {
        let Some(engine) = freed.upgrade() else { return };
        let ext = extension(&engine);
        if ext.meshes.borrow_mut().remove(&id).is_some() {
            ext.users.borrow_mut().remove(&id);
            ext.dirty.borrow_mut().remove(&id);
        }
        if let Some(mesh) = ext.bound.borrow_mut().remove(&id) {
            if let Some(users) = ext.users.borrow_mut().get_mut(&mesh) {
                users.retain(|&u| u != id);
            }
        }
    }));

    behaviors.add_getter(mesh_part_class, mesh_size, crate::services::getter(move |lua, engine, id| {
        let names = names(engine).ok_or_else(|| err("MeshPart is not available"))?;
        let value = mesh_size_of(&engine.state.borrow(), &names, id);
        Ok(convert::to_lua(lua, engine, &value)?)
    }));

    let mut add = |class: &str, name: &str, f: crate::services::MethodFn| {
        if let (Some(class), Some(name)) = (reflection.class_id(class), reflection.name_id(name)) {
            behaviors.add_method(class, name, f);
        }
    };

    // AssetService
    add("AssetService", "CreateEditableMesh", method(|lua, engine, _, _| new_mesh(lua, engine, Mesh::default(), false)));
    // CreateEditableMeshAsync(content[, options]) (measured in a 0.747 client, tools/parity/probes/editable_mesh_texts.luau and
    // arg_checks.luau: the arguments cast last to first, before the call refuses to wait: options no table "Unable to cast
    // <type> to Dictionary"; content nil "Argument 1 missing or nil", a string read as a Content's uri, anything else no
    // Content "Unable to cast <type> to Content"; a Content with no uri (Content.none, "") "Failed to load mesh ")
    add("AssetService", "CreateEditableMeshAsync", method(|lua, engine, _, args| {
        match args.get(2) {
            LuaValue::Nil | LuaValue::Table(_) => {}
            other => return Err(err(format!("Unable to cast {} to Dictionary", crate::services::tween::cast_name(engine, other)))),
        }
        let fixed_size = fixed_option(args.get(2));
        match args.get(1) {
            LuaValue::Nil => return Err(err("Argument 1 missing or nil")),
            LuaValue::String(_) => {}
            other if matches!(convert::from_lua(other), Value::Content(_)) => {}
            other => return Err(err(format!("Unable to cast {} to Content", crate::services::tween::cast_name(engine, other)))),
        }
        if let Some(source) = content_object(engine, args.get(1)) {
            let mesh = extension(engine).meshes.borrow().get(&source).cloned().unwrap_or_default();
            engine.state.borrow().sched.charge(mesh.positions.count() as f64 * crate::cost::MESH_COPY_VERTEX);
            return new_mesh(lua, engine, mesh, fixed_size);
        }
        let uri = content_uri(args.get(1)).unwrap_or_default();
        if uri.is_empty() {
            return Err(err("Failed to load mesh "));
        }
        // (an asset's mesh waits for its load: a thread that cannot yield raises before anything is made, by the rule
        // measured for the engine's other yielding calls, lua/mod.rs NOT_YIELDABLE; a client in a place that is not published
        // raises UnpublishedPlaceNotInStudio here instead, which ree, a published game's engine, does not)
        crate::lua::refuse_unyieldable(&lua.current_thread(), crate::lua::NOT_YIELDABLE)?;
        // (measured in Studio 0.747's client, 2026-11-24: an id of no protocol it knows raises; another it cannot load
        // was refused there for the place, so it is not known what that raises)
        if engine.options.local_file(&uri).is_none() && crate::content_id::unknown_protocol(&uri) {
            return Err(err(format!("Failed to load mesh {uri}")));
        }
        // Studio's wait (crate::cost): the asset's first fetch in the session, else its decode
        let first = !extension(engine).assets.borrow().contains_key(&asset_key(engine, &uri));
        let mesh = match load_asset(engine, &uri) {
            Ok(mesh) => (*mesh).clone(),
            Err(problem) => {
                engine.warn(format!("EditableMesh: {problem}; it starts empty"));
                Mesh::default()
            }
        };
        let wait = if first { crate::cost::MESH_FETCH } else { mesh.positions.count() as f64 * crate::cost::MESH_DECODE_VERTEX };
        let id = new_mesh_id(engine, mesh, fixed_size)?;
        engine.state.borrow_mut().sched.schedule_task(crate::scheduler::TimerKind::Delay, wait, lua.current_thread(), vec![Value::Instance(id)]);
        Err(Flow::Yield)
    }));
    add("AssetService", "CreateMeshPartAsync", method(|lua, engine, _, args| {
        if let Some(source) = content_object(engine, args.get(1)) {
            let vertices = extension(engine).meshes.borrow().get(&source).map_or(0, |mesh| mesh.positions.count());
            engine.state.borrow().sched.charge(vertices as f64 * crate::cost::MESH_PART_VERTEX);
            return mesh_part(lua, engine, source, args.get(2));
        }
        let Some(uri) = content_uri(args.get(1)) else {
            return Err(err("CreateMeshPartAsync: expected a mesh Content"));
        };
        let names = names(engine).ok_or_else(|| err("MeshPart is not available"))?;
        // Studio's error for content it cannot load (measured 2026-11-03), with the reason after it
        let (size, skinned) = match load_asset(engine, &uri) {
            Ok(mesh) => (mesh.size(), mesh.skinned()),
            Err(problem) => return Err(Flow::Error(ScriptError::plain(format!("Failed to load mesh asset. {problem}")))),
        };
        let part = {
            let hooks = engine.hooks.borrow();
            let mut state = engine.state.borrow_mut();
            let part = core::create(&mut state, &engine.reflection, names.mesh_part);
            core::raw_set(&mut state, &engine.reflection, &hooks, part, names.mesh_id, Value::String(Str::from(uri.as_str())));
            for name in [names.mesh_size, names.initial_size, names.size] {
                core::raw_set(&mut state, &engine.reflection, &hooks, part, name, size_value(size));
            }
            core::raw_set(&mut state, &engine.reflection, &hooks, part, names.has_skinned_mesh, Value::Bool(skinned));
            part
        };
        Ok(values([instance_value(lua, engine, part)?]))
    }));

    // MeshPart:ApplyMesh: the other part's mesh (and its binding) with MeshSize and HasSkinnedMesh
    // (measured: a skinned part's onto a fresh one makes it true, an unskinned one's onto a skinned
    // one false); Size stays
    add("MeshPart", "ApplyMesh", method(|_, engine, id, args| {
        let names = names(engine).ok_or_else(|| err("MeshPart is not available"))?;
        // (measured in a 0.747 client: nil or an instance that is no MeshPart "ApplyMesh requires an MeshPart object", another
        // value "Unable to cast value to Object"; a mesh grown past MeshPart's limit since is applied as it was)
        let source = match args.get(1) {
            LuaValue::Nil => return Err(err("ApplyMesh requires an MeshPart object")),
            value => crate::lua::raw::instance_id(value).ok_or_else(|| err("Unable to cast value to Object"))?,
        };
        if engine.state.borrow().dom.get(source).map(|instance| instance.class) != Some(names.mesh_part) {
            return Err(err("ApplyMesh requires an MeshPart object"));
        }
        let mesh = extension(engine).bound.borrow().get(&source).copied();
        {
            let hooks = engine.hooks.borrow();
            let mut state = engine.state.borrow_mut();
            let content = state.dom.raw_get(source, names.mesh_id).cloned().unwrap_or(Value::Nil);
            // as MeshSize reads it: a part from a file holds its mesh's size as the file's InitialSize
            let mesh_size = mesh_size_of(&state, &names, source);
            let skinned = matches!(state.dom.raw_get(source, names.has_skinned_mesh), Some(Value::Bool(true)));
            core::raw_set(&mut state, &engine.reflection, &hooks, id, names.mesh_id, content);
            core::raw_set(&mut state, &engine.reflection, &hooks, id, names.mesh_size, mesh_size.clone());
            core::raw_set(&mut state, &engine.reflection, &hooks, id, names.initial_size, mesh_size);
            core::raw_set(&mut state, &engine.reflection, &hooks, id, names.has_skinned_mesh, Value::Bool(skinned));
        }
        bind(engine, id, mesh);
        Ok(none())
    }));

    let mut add = |name: &str, f: crate::services::MethodFn| {
        if let (Some(class), Some(name)) = (reflection.class_id("EditableMesh"), reflection.name_id(name)) {
            behaviors.add_method(class, name, f);
        }
    };

    // Adding (each refused on a fixed-size mesh after its arguments' casts, measured)
    add("AddVertex", method(|_, engine, id, args| {
        let p = vec3_of(args.get(1)).ok_or_else(|| err("AddVertex: Vector3 expected"))?;
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.positions.add(p);
            Ok(values([LuaValue::Number(m.positions.id(VERTEX, slot))]))
        })
    }));
    add("AddNormal", method(|_, engine, id, args| {
        let n = vec3_of(args.get(1));
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.normals.add(n);
            Ok(values([LuaValue::Number(m.normals.id(NORMAL, slot))]))
        })
    }));
    add("AddUV", method(|_, engine, id, args| {
        let uv = vec2_of(args.get(1)).ok_or_else(|| err("AddUV: Vector2 expected"))?;
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.uvs.add(uv);
            Ok(values([LuaValue::Number(m.uvs.id(UV, slot))]))
        })
    }));
    add("AddColor", method(|_, engine, id, args| {
        let c = color_of(args.get(1)).ok_or_else(|| err("AddColor: Color3 expected"))?;
        let a = alpha(args.number(2).unwrap_or(1.0));
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.colors.add((c, a));
            Ok(values([LuaValue::Number(m.colors.id(COLOR, slot))]))
        })
    }));
    // (measured: each vertex id in turn, as triangle_vertex reads it)
    add("AddTriangle", method(|lua, engine, id, args| {
        let ids = [int64_arg(lua, engine, args, 1)?, int64_arg(lua, engine, args, 2)?, int64_arg(lua, engine, args, 3)?];
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let mut v = [0u32; 3];
            for (slot, &vertex) in v.iter_mut().zip(&ids) {
                *slot = triangle_vertex(m, vertex)?;
            }
            let face = m.new_face(&v);
            let slot = m.faces.add(face);
            Ok(values([LuaValue::Number(m.faces.id(FACE, slot))]))
        })
    }));
    // AddFace(v0, v1, v2[, v3]): a triangle or a quad (measured in a 0.747 client, tools/parity/probes/editable_mesh_quads.luau:
    // refused on a fixed-size mesh first; a table first is "AddFace takes vertex IDs as separate arguments, e.g. AddFace(v0, v1,
    // v2)."; another count "AddFace called with <n> vertices, currently only supports 3 or 4 vertices"; then each id cast the
    // Luau way and read as AddTriangle reads it)
    add("AddFace", method(|lua, engine, id, args| {
        refuse_fixed(engine, id)?;
        if matches!(args.get(1), LuaValue::Table(_)) {
            return Err(err("AddFace takes vertex IDs as separate arguments, e.g. AddFace(v0, v1, v2)."));
        }
        let count = args.len().saturating_sub(1);
        if !(3..=4).contains(&count) {
            return Err(err(format!("AddFace called with {count} vertices, currently only supports 3 or 4 vertices")));
        }
        let mut ids = Vec::with_capacity(count);
        for at in 1..=count {
            ids.push(luau_int64(lua, engine, args, at, "AddFace")?);
        }
        with_mesh(engine, id, true, |m| {
            let mut v = Vec::with_capacity(count);
            for &vertex in &ids {
                v.push(triangle_vertex(m, vertex)?);
            }
            let face = m.new_face(&v);
            let slot = m.faces.add(face);
            Ok(values([LuaValue::Number(m.faces.id(FACE, slot))]))
        })
    }));
    // Triangulate: each quad becomes two triangles (Mesh::triangulate; allowed on a fixed-size mesh, measured)
    add("Triangulate", method(|_, engine, id, _| {
        with_mesh(engine, id, true, |m| {
            m.triangulate();
            Ok(none())
        })
    }));
    // AddBone(properties) (measured in a 0.747 client: nil "Argument 1 missing or nil", no table "Unable to cast <type> to
    // Dictionary"; Name, CFrame and Virtual are each required, in that order ("Missing bone CFrame value"; one of another type
    // "Variant cast failed"); another key "Unexpected entry in table: <key>"; the name as bone_name takes it)
    add("AddBone", method(|lua, engine, id, args| {
        let params = match args.get(1) {
            LuaValue::Nil => return Err(err("Argument 1 missing or nil")),
            LuaValue::Table(params) => params.clone(),
            other => return Err(err(format!("Unable to cast {} to Dictionary", crate::services::tween::cast_name(engine, other)))),
        };
        refuse_fixed(engine, id)?;
        let field = |name: &str| params.raw_get::<LuaValue>(name).unwrap_or(LuaValue::Nil);
        let missing = |name: &str| err(format!("Missing bone {name} value"));
        let name = match field("Name") {
            LuaValue::Nil => return Err(missing("Name")),
            LuaValue::String(text) => text.as_bytes().to_vec(),
            _ => return Err(err("Variant cast failed")),
        };
        let cframe = match field("CFrame") {
            LuaValue::Nil => return Err(missing("CFrame")),
            value => cframe_of(&value).ok_or_else(|| err("Variant cast failed"))?,
        };
        let virtual_bone = match field("Virtual") {
            LuaValue::Nil => return Err(missing("Virtual")),
            LuaValue::Boolean(value) => value,
            _ => return Err(err("Variant cast failed")),
        };
        for pair in params.pairs::<LuaValue, LuaValue>() {
            let (key, _) = pair?;
            let text = match &key {
                LuaValue::String(text) => text.to_string_lossy(),
                other => String::from_utf8_lossy(&convert::cast_text(lua, engine, other)?).into_owned(),
            };
            if !matches!(text.as_str(), "Name" | "CFrame" | "Virtual") {
                return Err(err(format!("Unexpected entry in table: {text}")));
            }
        }
        with_mesh(engine, id, true, |m| {
            let name = bone_name(m, &name, None)?;
            let slot = m.bones.add(Bone { name, cframe, parent: None, virtual_bone });
            Ok(values([LuaValue::Number(m.bones.id(BONE, slot))]))
        })
    }));

    // Listing (in the order of the client's dense lists: see Slots)
    for (name, kind) in [("GetVertices", VERTEX), ("GetNormals", NORMAL), ("GetColors", COLOR), ("GetUVs", UV), ("GetFaces", FACE), ("GetBones", BONE)] {
        add(name, method(move |lua, engine, id, _| {
            with_mesh(engine, id, false, |m| {
                let ids: Vec<f64> = match kind {
                    VERTEX => m.positions.live().map(|(i, _)| m.positions.id(kind, i)).collect(),
                    NORMAL => m.normals.live().map(|(i, _)| m.normals.id(kind, i)).collect(),
                    COLOR => m.colors.live().map(|(i, _)| m.colors.id(kind, i)).collect(),
                    UV => m.uvs.live().map(|(i, _)| m.uvs.id(kind, i)).collect(),
                    // (quads first, then triangles, each in listing order: measured in a 0.747 client)
                    FACE => {
                        let quads = m.faces.live().filter(|(_, face)| face.len() == 4);
                        quads.chain(m.faces.live().filter(|(_, face)| face.len() == 3)).map(|(i, _)| m.faces.id(kind, i)).collect()
                    }
                    _ => m.bones.live().map(|(i, _)| m.bones.id(kind, i)).collect(),
                };
                let each = if kind == VERTEX { crate::cost::LISTED_VERTEX } else { crate::cost::LISTED };
                engine.state.borrow().sched.charge(ids.len() as f64 * each);
                Ok(values([ids_value(lua, ids.into_iter())?]))
            })
        }));
    }

    // Values
    add("GetPosition", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        let p = with_mesh(engine, id, false, |m| Ok(m.positions.get(m.positions.at(VERTEX, v)?).copied().unwrap_or(Vec3::ZERO)))?;
        Ok(values([vector_value(lua, engine, p)?]))
    }));
    add("SetPosition", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        let p = vec3_of(args.get(2)).ok_or_else(|| err("SetPosition: Vector3 expected"))?;
        with_mesh(engine, id, true, |m| {
            let slot = m.positions.at(VERTEX, v)?;
            *m.positions.get_mut(slot).expect("live") = p;
            Ok(none())
        })
    }));
    add("GetNormal", method(|lua, engine, id, args| {
        let n = id_arg(lua, engine, args, 1, NORMAL)?;
        let value = with_mesh(engine, id, false, |m| Ok(m.normal_value(m.normals.at(NORMAL, n)?).unwrap_or(Vec3::ZERO)))?;
        Ok(values([vector_value(lua, engine, value)?]))
    }));
    add("SetNormal", method(|lua, engine, id, args| {
        let n = id_arg(lua, engine, args, 1, NORMAL)?;
        let value = vec3_of(args.get(2));
        with_mesh(engine, id, true, |m| {
            let slot = m.normals.at(NORMAL, n)?;
            *m.normals.get_mut(slot).expect("live") = value;
            Ok(none())
        })
    }));
    add("GetUV", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, UV)?;
        let uv = with_mesh(engine, id, false, |m| Ok(m.uvs.get(m.uvs.at(UV, i)?).copied().unwrap_or([0.0; 2])))?;
        Ok(values([convert::to_lua(lua, engine, &Value::Vector2(dom::Vector2::new(uv[0], uv[1])))?]))
    }));
    add("SetUV", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, UV)?;
        let uv = vec2_of(args.get(2)).ok_or_else(|| err("SetUV: Vector2 expected"))?;
        with_mesh(engine, id, true, |m| {
            let slot = m.uvs.at(UV, i)?;
            *m.uvs.get_mut(slot).expect("live") = uv;
            Ok(none())
        })
    }));
    add("GetColor", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, COLOR)?;
        let (c, _) = with_mesh(engine, id, false, |m| Ok(m.colors.get(m.colors.at(COLOR, i)?).copied().unwrap_or(([1.0; 3], 1.0))))?;
        Ok(values([convert::to_lua(lua, engine, &Value::Color3(dom::Color3::new(c[0], c[1], c[2])))?]))
    }));
    add("SetColor", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, COLOR)?;
        let c = color_of(args.get(2)).ok_or_else(|| err("SetColor: Color3 expected"))?;
        with_mesh(engine, id, true, |m| {
            let slot = m.colors.at(COLOR, i)?;
            m.colors.get_mut(slot).expect("live").0 = c;
            Ok(none())
        })
    }));
    add("GetColorAlpha", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, COLOR)?;
        let (_, a) = with_mesh(engine, id, false, |m| Ok(m.colors.get(m.colors.at(COLOR, i)?).copied().unwrap_or(([1.0; 3], 1.0))))?;
        Ok(values([LuaValue::Number(a as f64)]))
    }));
    add("SetColorAlpha", method(|lua, engine, id, args| {
        let i = id_arg(lua, engine, args, 1, COLOR)?;
        let a = alpha(args.number(2).unwrap_or(1.0));
        with_mesh(engine, id, true, |m| {
            let slot = m.colors.at(COLOR, i)?;
            m.colors.get_mut(slot).expect("live").1 = a;
            Ok(none())
        })
    }));

    // Faces: a list per corner, 3 or 4
    for (name, kind) in [("GetFaceVertices", VERTEX), ("GetFaceNormals", NORMAL), ("GetFaceColors", COLOR), ("GetFaceUVs", UV)] {
        add(name, method(move |lua, engine, id, args| {
            let f = id_arg(lua, engine, args, 1, FACE)?;
            with_mesh(engine, id, false, |m| {
                let face = *m.faces.get(m.faces.at(FACE, f)?).expect("live");
                Ok(values([ids_value(lua, (0..face.len()).map(|corner| corner_id(m, &face, corner, kind)))?]))
            })
        }));
    }
    // SetFaceVertices(face, ids): 3 or 4 vertices, the face's corner count following (measured in a 0.747 client: no array
    // "SetFaceVertices: second argument should be an array", another count "SetFaceVertices: only 3 or 4 vertices supported";
    // every id checked before any is set, a value that is no number reading as 0; see Mesh::set_face_vertices)
    add("SetFaceVertices", method(|lua, engine, id, args| {
        let f = id_arg(lua, engine, args, 1, FACE)?;
        let LuaValue::Table(table) = args.get(2) else { return Err(err("SetFaceVertices: second argument should be an array")) };
        let items = sequence(table);
        with_mesh(engine, id, true, |m| {
            let f = m.faces.at(FACE, f)?;
            if !(3..=4).contains(&items.len()) {
                return Err(err("SetFaceVertices: only 3 or 4 vertices supported"));
            }
            let mut slots = Vec::with_capacity(items.len());
            for item in &items {
                let v = loose_id(lua, item);
                check_kind(v, VERTEX)?;
                slots.push(m.positions.at(VERTEX, v)?);
            }
            m.set_face_vertices(f, &slots);
            Ok(none())
        })
    }));
    // SetFaceNormals / SetFaceColors / SetFaceUVs(face, ids): one per corner (measured in a 0.747 client: no table "EditableMesh
    // function: second argument should be a table", another count "Number of attributes provided does not match face"; then
    // corner by corner, so an error keeps the corners before it set; a corner given another one comes to the front of its
    // vertex's corners)
    for (name, kind) in [("SetFaceNormals", NORMAL), ("SetFaceColors", COLOR), ("SetFaceUVs", UV)] {
        add(name, method(move |lua, engine, id, args| {
            let f = id_arg(lua, engine, args, 1, FACE)?;
            let LuaValue::Table(table) = args.get(2) else { return Err(err("EditableMesh function: second argument should be a table")) };
            let items = sequence(table);
            with_mesh(engine, id, true, |m| {
                let f = m.faces.at(FACE, f)?;
                if items.len() != m.faces.get(f).expect("live").len() {
                    return Err(err("Number of attributes provided does not match face"));
                }
                for (corner, item) in items.iter().enumerate() {
                    let a = loose_id(lua, item);
                    check_kind(a, kind)?;
                    let slot = attribute_slot(m, kind, a)?;
                    m.set_corner(f, corner, (kind - 1) as u32, slot);
                }
                Ok(none())
            })
        }));
    }
    // (measured: a face that is gone is "Invalid faceId", 0 is named as no id; it removes the face only, what it used
    // staying until RemoveUnused)
    add("RemoveFace", method(|lua, engine, id, args| {
        let f = int64_arg(lua, engine, args, 1)?;
        refuse_fixed(engine, id)?;
        if parts(f).0 != FACE {
            return Err(wrong_kind(FACE, f));
        }
        with_mesh(engine, id, true, |m| {
            let f = m.faces.find(FACE, f).ok_or_else(|| err("Invalid faceId"))?;
            m.remove_faces(&[f]);
            Ok(none())
        })
    }));
    add("RemoveUnused", method(|lua, engine, id, _| {
        refuse_fixed(engine, id)?;
        let (removed, vertices) = with_mesh(engine, id, true, |m| {
            let vertices = m.positions.count();
            Ok((m.remove_unused(), vertices))
        })?;
        engine.state.borrow().sched.charge(vertices as f64 * crate::cost::REMOVE_UNUSED_VERTEX);
        Ok(values([ids_value(lua, removed.into_iter())?]))
    }));
    add("GetSize", method(|lua, engine, id, _| {
        let size = with_mesh(engine, id, false, |m| Ok(m.bounds().map_or(Vec3::ZERO, |(min, max)| max - min)))?;
        Ok(values([vector_value(lua, engine, size)?]))
    }));
    add("GetCenter", method(|lua, engine, id, _| {
        let center = with_mesh(engine, id, false, |m| Ok(m.bounds().map_or(Vec3::ZERO, |(min, max)| (min + max) * 0.5)))?;
        Ok(values([vector_value(lua, engine, center)?]))
    }));

    // Bones (measured in a 0.747 client; each setter is allowed on a fixed-size mesh)
    add("GetBoneName", method(|lua, engine, id, args| {
        let b = id_arg(lua, engine, args, 1, BONE)?;
        let name = with_mesh(engine, id, false, |m| Ok(m.bones.get(m.bones.at(BONE, b)?).map(|b| b.name.clone()).unwrap_or_default()))?;
        Ok(values([LuaValue::String(lua.create_string(&name)?)]))
    }));
    // SetBoneName(bone, name): the name read as a client reads a string parameter (services::strings: nil "Argument 2 missing
    // or nil", a number or another value as its text), then as bone_name takes it (a bone keeps its own name)
    add("SetBoneName", strings(SECOND, method(|lua, engine, id, args| {
        let name = args.string(2).map(|s| s.as_bytes().to_vec()).ok_or_else(|| err("Argument 2 missing or nil"))?;
        let b = id_arg(lua, engine, args, 1, BONE)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.bones.at(BONE, b)?;
            let name = bone_name(m, &name, Some(slot))?;
            m.bones.get_mut(slot).expect("live").name = name;
            Ok(none())
        })
    })));
    add("GetBoneParent", method(|lua, engine, id, args| {
        let b = id_arg(lua, engine, args, 1, BONE)?;
        let parent = with_mesh(engine, id, false, |m| {
            let parent = m.bones.get(m.bones.at(BONE, b)?).and_then(|b| b.parent);
            Ok(parent.map_or(0.0, |p| m.bones.id(BONE, p)))
        })?;
        Ok(values([LuaValue::Number(parent)]))
    }));
    // SetBoneParent(bone, parent): 0 for none; the bone itself or one under it "EditableMesh Error: Invalid bone parent"
    add("SetBoneParent", method(|lua, engine, id, args| {
        let parent = int64_arg(lua, engine, args, 2)?;
        let b = id_arg(lua, engine, args, 1, BONE)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.bones.at(BONE, b)?;
            let parent = match parent {
                0 => None,
                parent => {
                    check_kind(parent, BONE)?;
                    let mut up = Some(m.bones.at(BONE, parent)?);
                    let found = up;
                    while let Some(at) = up {
                        if at == slot {
                            return Err(err("EditableMesh Error: Invalid bone parent"));
                        }
                        up = m.bones.get(at).and_then(|bone| bone.parent);
                    }
                    found
                }
            };
            m.bones.get_mut(slot).expect("live").parent = parent;
            Ok(none())
        })
    }));
    add("GetBoneCFrame", method(|lua, engine, id, args| {
        let b = id_arg(lua, engine, args, 1, BONE)?;
        let c = with_mesh(engine, id, false, |m| Ok(m.bones.get(m.bones.at(BONE, b)?).map(|b| b.cframe).expect("live")))?;
        Ok(values([convert::to_lua(lua, engine, &cframe_value(&c))?]))
    }));
    add("SetBoneCFrame", method(|lua, engine, id, args| {
        let c = cframe_arg(engine, args, 2)?;
        let b = id_arg(lua, engine, args, 1, BONE)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.bones.at(BONE, b)?;
            m.bones.get_mut(slot).expect("live").cframe = c;
            Ok(none())
        })
    }));
    add("GetBoneIsVirtual", method(|lua, engine, id, args| {
        let b = id_arg(lua, engine, args, 1, BONE)?;
        let v = with_mesh(engine, id, false, |m| Ok(m.bones.get(m.bones.at(BONE, b)?).map(|b| b.virtual_bone).expect("live")))?;
        Ok(values([LuaValue::Boolean(v)]))
    }));
    // SetBoneIsVirtual(bone, virtual): a bone made not virtual leaves every FACS pose (measured)
    add("SetBoneIsVirtual", method(|lua, engine, id, args| {
        let on = bool_arg(engine, args, 2)?;
        let b = id_arg(lua, engine, args, 1, BONE)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.bones.at(BONE, b)?;
            m.bones.get_mut(slot).expect("live").virtual_bone = on;
            if !on {
                m.drop_bone_poses(slot);
            }
            Ok(none())
        })
    }));
    add("GetVertexBones", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        with_mesh(engine, id, false, |m| {
            let v = m.positions.at(VERTEX, v)?;
            let bones: Vec<f64> = m.skin.get(&v).map_or(Vec::new(), |(b, _)| b.iter().map(|&i| m.bones.id(BONE, i)).collect());
            Ok(values([ids_value(lua, bones.into_iter())?]))
        })
    }));
    add("GetVertexBoneWeights", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        with_mesh(engine, id, false, |m| {
            let v = m.positions.at(VERTEX, v)?;
            let weights = m.skin.get(&v).map_or(Vec::new(), |(_, w)| w.iter().map(|&w| w as f64).collect());
            Ok(values([ids_value(lua, weights.into_iter())?]))
        })
    }));
    add("SetVertexBones", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        let ids = ids_of(args.get(2));
        with_mesh(engine, id, true, |m| {
            let v = m.positions.at(VERTEX, v)?;
            let bones = slots_of(&m.bones, BONE, &ids);
            let entry = m.skin.entry(v).or_default();
            entry.1.resize(bones.len(), 0.0);
            entry.0 = bones;
            Ok(none())
        })
    }));
    add("SetVertexBoneWeights", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        // (measured: weights keep 1/255 steps)
        let weights: Vec<f32> = numbers_of(args.get(2)).into_iter().map(|w| ((w.clamp(0.0, 1.0) * 255.0).round() / 255.0) as f32).collect();
        with_mesh(engine, id, true, |m| {
            let v = m.positions.at(VERTEX, v)?;
            let entry = m.skin.entry(v).or_default();
            entry.1 = weights;
            Ok(none())
        })
    }));

    // BatchSetValues / BatchGetValues: positions, normals, uvs, colours by their ids
    add("BatchSetValues", method(|_, engine, id, args| {
        let LuaValue::Table(ids) = args.get(1) else { return Err(err("BatchSetValues: table of ids expected")) };
        let LuaValue::Table(items) = args.get(2) else { return Err(err("BatchSetValues: table of values expected")) };
        let ids: Vec<i64> = ids.sequence_values::<f64>().filter_map(Result::ok).map(id_of_number).collect();
        let items: Vec<LuaValue> = items.sequence_values::<LuaValue>().filter_map(Result::ok).collect();
        with_mesh(engine, id, true, |m| {
            for (&id, item) in ids.iter().zip(&items) {
                match parts(id).0 {
                    VERTEX => {
                        if let (Some(slot), Some(p)) = (m.positions.find(VERTEX, id), vec3_of(item)) {
                            *m.positions.get_mut(slot).expect("live") = p;
                        }
                    }
                    NORMAL => {
                        if let Some(slot) = m.normals.find(NORMAL, id) {
                            *m.normals.get_mut(slot).expect("live") = vec3_of(item);
                        }
                    }
                    UV => {
                        if let (Some(slot), Some(uv)) = (m.uvs.find(UV, id), vec2_of(item)) {
                            *m.uvs.get_mut(slot).expect("live") = uv;
                        }
                    }
                    // (measured: a number sets a colour's alpha, a Color3 its colour)
                    COLOR => {
                        if let Some(slot) = m.colors.find(COLOR, id).and_then(|slot| m.colors.get_mut(slot)) {
                            match (color_of(item), item) {
                                (Some(c), _) => slot.0 = c,
                                (None, LuaValue::Number(n)) => slot.1 = alpha(*n),
                                (None, LuaValue::Integer(n)) => slot.1 = alpha(*n as f64),
                                _ => {}
                            }
                        }
                    }
                    _ => return Err(err("BatchSetValues: invalid id")),
                }
            }
            Ok(none())
        })
    }));
    add("BatchGetValues", method(|lua, engine, id, args| {
        let ids = ids_of(args.get(1));
        let out = lua.create_table()?;
        with_mesh(engine, id, false, |m| {
            for (index, &id) in ids.iter().enumerate() {
                let value = match parts(id).0 {
                    VERTEX => m.positions.find(VERTEX, id).and_then(|i| m.positions.get(i)).map(|p| Value::Vector3(dom::Vector3::new(p.x, p.y, p.z))),
                    NORMAL => m.normals.find(NORMAL, id).and_then(|i| m.normal_value(i)).map(|p| Value::Vector3(dom::Vector3::new(p.x, p.y, p.z))),
                    UV => m.uvs.find(UV, id).and_then(|i| m.uvs.get(i)).map(|uv| Value::Vector2(dom::Vector2::new(uv[0], uv[1]))),
                    COLOR => m.colors.find(COLOR, id).and_then(|i| m.colors.get(i)).map(|(c, _)| Value::Color3(dom::Color3::new(c[0], c[1], c[2]))),
                    _ => None,
                };
                let value = value.ok_or_else(|| err("BatchGetValues: invalid id"))?;
                out.raw_set(index + 1, convert::to_lua(lua, engine, &value)?)?;
            }
            Ok(())
        })?;
        Ok(values([LuaValue::Table(out)]))
    }));

    // BatchAdd(attribute, values[, alphas]): vertices, normals, uvs, colours (with their alphas) or faces at once, their
    // new ids in the values' order (measured in a 0.747 client: tools/parity/probes/editable_mesh_batch.luau). Vertices,
    // normals, uvs and colours are all made first, then given their values in order, so a value of the wrong type raises
    // with the ones before it set and the rest left at their defaults; a nil normal is an automatic one. Faces (3 or 4
    // vertices: a quad is one face) are checked whole first, made in order (each corner as AddTriangle makes it) and numbered
    // backwards: the first face gets the batch's last id (with slots free, the batch's new slots backwards, then the free ones,
    // last freed first; measured in editable_mesh_vertex_batch.luau); the triangles' corners are made before the quads'.
    add("BatchAdd", method(|lua, engine, id, args| {
        refuse_fixed(engine, id)?;
        if args.len() < 3 {
            return Err(err("BatchAdd: expects at least 2 arguments (attr, data)"));
        }
        let attribute = mesh_attribute(engine, args.get(1))?;
        let (label, expects, wanted) = match attribute {
            0 => ("Vertex", "1 argument (positions)", 1),
            1 => ("Normal", "1 argument (normals)", 1),
            2 => ("Color", "2 arguments (colors, alphas)", 2),
            3 => ("UV", "1 argument (uvs)", 1),
            _ => ("Face", "1 argument (vertexIds)", 1),
        };
        let prefix = format!("BatchAdd(Enum.MeshAttribute.{label}, ...)");
        if args.len() - 2 != wanted {
            return Err(err(format!("{prefix}: expects {expects}")));
        }
        let array = |at: usize, name: &str| match args.get(at) {
            LuaValue::Table(table) => Ok(sequence(table)),
            _ => Err(err(format!("{prefix}: {name} must be an array"))),
        };
        // (the client's own argument numbers: -1 for a vector, 4 for a uv, 5 for a colour, wherever it is in the array)
        let wrong = |expected: &str, number: i32, value: &LuaValue| {
            Flow::Error(ScriptError::at_caller(format!("invalid argument #{number} ({expected} expected, got {})", crate::lua::typeof_value(engine, value))))
        };
        let fill = |slots: &[u32], items: &[LuaValue], set: &mut dyn FnMut(u32, &LuaValue) -> Result<(), Flow>| -> Result<(), Flow> {
            for (&slot, item) in slots.iter().zip(items) {
                set(slot, item)?;
            }
            Ok(())
        };
        let ids: Vec<f64> = match attribute {
            0 => {
                let items = array(2, "positions")?;
                with_mesh(engine, id, true, |m| {
                    let slots: Vec<u32> = items.iter().map(|_| m.positions.add(Vec3::ZERO)).collect();
                    fill(&slots, &items, &mut |slot, item| {
                        *m.positions.get_mut(slot).expect("made above") = vec3_of(item).ok_or_else(|| wrong("Vector3", -1, item))?;
                        Ok(())
                    })?;
                    Ok(slots.iter().map(|&slot| m.positions.id(VERTEX, slot)).collect())
                })?
            }
            1 => {
                let items = array(2, "normals")?;
                with_mesh(engine, id, true, |m| {
                    let slots: Vec<u32> = items.iter().map(|_| m.normals.add(None)).collect();
                    fill(&slots, &items, &mut |slot, item| {
                        if !item.is_nil() {
                            *m.normals.get_mut(slot).expect("made above") = Some(vec3_of(item).ok_or_else(|| wrong("Vector3", -1, item))?);
                        }
                        Ok(())
                    })?;
                    Ok(slots.iter().map(|&slot| m.normals.id(NORMAL, slot)).collect())
                })?
            }
            2 => {
                let colors = array(2, "colors")?;
                let alphas = array(3, "alphas")?;
                if colors.len() != alphas.len() {
                    return Err(err(format!("{prefix}: colors and alphas arrays must be the same length")));
                }
                with_mesh(engine, id, true, |m| {
                    let slots: Vec<u32> = colors.iter().map(|_| m.colors.add(([1.0; 3], 1.0))).collect();
                    for ((&slot, color), opacity) in slots.iter().zip(&colors).zip(&alphas) {
                        let rgb = color_of(color).ok_or_else(|| wrong("Color3", 5, color))?;
                        // (as a number reads it: anything else is 0, unmeasured beyond a string being taken)
                        let opacity = match opacity {
                            LuaValue::Number(n) => *n,
                            LuaValue::Integer(n) => *n as f64,
                            LuaValue::String(text) => text.to_str().ok().and_then(|text| text.trim().parse::<f64>().ok()).unwrap_or(0.0),
                            _ => 0.0,
                        };
                        *m.colors.get_mut(slot).expect("made above") = (rgb, alpha(opacity));
                    }
                    Ok(slots.iter().map(|&slot| m.colors.id(COLOR, slot)).collect())
                })?
            }
            3 => {
                let items = array(2, "uvs")?;
                with_mesh(engine, id, true, |m| {
                    let slots: Vec<u32> = items.iter().map(|_| m.uvs.add([0.0; 2])).collect();
                    fill(&slots, &items, &mut |slot, item| {
                        *m.uvs.get_mut(slot).expect("made above") = vec2_of(item).ok_or_else(|| wrong("Vector2", 4, item))?;
                        Ok(())
                    })?;
                    Ok(slots.iter().map(|&slot| m.uvs.id(UV, slot)).collect())
                })?
            }
            _ => {
                let LuaValue::Table(table) = args.get(2) else {
                    return Err(err(format!("{prefix}: vertexIds must be a 2D array")));
                };
                let items = sequence(table);
                with_mesh(engine, id, true, |m| {
                    let mut faces: Vec<Vec<u32>> = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        let LuaValue::Table(ids) = item else {
                            return Err(err(format!("{prefix}: element {} must be an array of 3 or 4 vertex ids", index + 1)));
                        };
                        let ids = sequence(ids);
                        if !(3..=4).contains(&ids.len()) {
                            return Err(err(format!("{prefix}: element {} must have 3 or 4 vertex ids, got {}", index + 1, ids.len())));
                        }
                        faces.push(ids.iter().map(|value| face_vertex(m, value)).collect::<Result<_, _>>()?);
                    }
                    // (the slots as the client hands a batch of faces them: made free after those already free, then
                    // taken last first)
                    m.faces.reserve(faces.len());
                    let slots: Vec<u32> = faces.iter().map(|_| m.faces.take()).collect();
                    // (the triangles' corners are made before the quads': measured, a quad then a triangle over a new vertex
                    // each read the triangle's new normal first)
                    let mut made: Vec<Option<Face>> = vec![None; faces.len()];
                    for quads in [false, true] {
                        for (index, vertices) in faces.iter().enumerate() {
                            if (vertices.len() == 4) == quads {
                                made[index] = Some(m.new_face(vertices));
                            }
                        }
                    }
                    for (face, &slot) in made.into_iter().zip(&slots) {
                        m.faces.put(slot, face.expect("made above"));
                    }
                    Ok(slots.iter().map(|&slot| m.faces.id(FACE, slot)).collect())
                })?
            }
        };
        Ok(values([ids_value(lua, ids.into_iter())?]))
    }));

    // BatchGetFaceAttributes(attribute, faceIds): per face, the vertex, normal, colour or uv ids of its corners (measured in a
    // 0.747 client: "BatchGetFaceAttributes: expects 2 arguments (attr, faceIds)"; the attribute as mesh_attribute reads it,
    // Face "BatchGetFaceAttributes: Enum.MeshAttribute.Face is not supported"; "BatchGetFaceAttributes: ids must be an array";
    // each id in turn: "BatchGetFaceAttributes: id at index <i> is not a number", 0 and gone "Invalid id", another kind named)
    add("BatchGetFaceAttributes", method(|lua, engine, id, args| {
        const METHOD: &str = "BatchGetFaceAttributes";
        if args.len() != 3 {
            return Err(err(format!("{METHOD}: expects 2 arguments (attr, faceIds)")));
        }
        let which = mesh_attribute(engine, args.get(1))?;
        if which == 4 {
            return Err(err(format!("{METHOD}: Enum.MeshAttribute.Face is not supported")));
        }
        let LuaValue::Table(table) = args.get(2) else { return Err(err(format!("{METHOD}: ids must be an array"))) };
        let items = sequence(table);
        let out = lua.create_table_with_capacity(items.len(), 0)?;
        with_mesh(engine, id, false, |m| {
            for (index, item) in items.iter().enumerate() {
                let f = element_id(lua, item, METHOD, "id", index + 1)?;
                check_kind(f, FACE)?;
                let face = *m.faces.get(m.faces.at(FACE, f)?).expect("live");
                out.raw_set(index + 1, ids_value(lua, (0..face.len()).map(|corner| corner_id(m, &face, corner, which as u64 + 1)))?)?;
            }
            Ok(())
        })?;
        Ok(values([LuaValue::Table(out)]))
    }));
    // BatchSetFaceAttributes(faceIds, attributeIdArrays): each face's corners given vertices, normals, colours or uvs, all of
    // the kind of the first array's first id (measured in a 0.747 client: "BatchSetFaceAttributes: expects 2 arguments
    // (faceIds, attributeIdArrays)", "...: faceIds must be an array", "...: attributeIdArrays must be an array of arrays",
    // "...: faceIds and attributeIdArrays must be the same length", "...: element <i> of attributeIdArrays is not an array",
    // "...: attributeId at face <i> corner <c> is not a number", a first id of no kind of those "...: unsupported attribute
    // ID type"; then face by face and corner by corner, so an error keeps what was set before it: "...: faceId at index <i> is
    // not a number", the face checked (0 and gone "Invalid id", another kind named), "...: inner array at index <i> has <n>
    // elements but face has <c> corners", each id checked (another kind named); allowed on a fixed-size mesh)
    add("BatchSetFaceAttributes", method(|lua, engine, id, args| {
        const METHOD: &str = "BatchSetFaceAttributes";
        if args.len() != 3 {
            return Err(err(format!("{METHOD}: expects 2 arguments (faceIds, attributeIdArrays)")));
        }
        let LuaValue::Table(faces) = args.get(1) else { return Err(err(format!("{METHOD}: faceIds must be an array"))) };
        let LuaValue::Table(arrays) = args.get(2) else { return Err(err(format!("{METHOD}: attributeIdArrays must be an array of arrays"))) };
        let (faces, arrays) = (sequence(faces), sequence(arrays));
        if faces.len() != arrays.len() {
            return Err(err(format!("{METHOD}: faceIds and attributeIdArrays must be the same length")));
        }
        let array = |index: usize| match &arrays[index] {
            LuaValue::Table(table) => Ok(sequence(table)),
            _ => Err(err(format!("{METHOD}: element {} of attributeIdArrays is not an array", index + 1))),
        };
        let attribute = |face: usize, corner: usize, value: &LuaValue| element_id(lua, value, METHOD, "attributeId", 0).map_err(|_| err(format!("{METHOD}: attributeId at face {face} corner {corner} is not a number")));
        let Some(first) = arrays.first() else { return Ok(none()) };
        let _ = first;
        let kind = match array(0)?.first() {
            Some(value) => parts(attribute(1, 1, value)?).0,
            None => 0,
        };
        if !matches!(kind, VERTEX | NORMAL | COLOR | UV) {
            return Err(err(format!("{METHOD}: unsupported attribute ID type")));
        }
        with_mesh(engine, id, true, |m| {
            for (index, face) in faces.iter().enumerate() {
                let f = element_id(lua, face, METHOD, "faceId", index + 1)?;
                check_kind(f, FACE)?;
                let f = m.faces.at(FACE, f)?;
                let items = array(index)?;
                let corners = m.faces.get(f).expect("live").len();
                if items.len() != corners {
                    return Err(err(format!("{METHOD}: inner array at index {} has {} elements but face has {corners} corners", index + 1, items.len())));
                }
                if kind == VERTEX {
                    let mut slots = Vec::with_capacity(corners);
                    for (corner, item) in items.iter().enumerate() {
                        let v = attribute(index + 1, corner + 1, item)?;
                        check_kind(v, VERTEX)?;
                        slots.push(m.positions.at(VERTEX, v)?);
                    }
                    m.set_face_vertices(f, &slots);
                    continue;
                }
                for (corner, item) in items.iter().enumerate() {
                    let a = attribute(index + 1, corner + 1, item)?;
                    check_kind(a, kind)?;
                    let slot = attribute_slot(m, kind, a)?;
                    m.set_corner(f, corner, (kind - 1) as u32, slot);
                }
            }
            Ok(none())
        })
    }));

    // A vertex's faces and the normals / colours / uvs its corners use: one per corner, front first (Mesh::corners_of;
    // measured in a 0.747 client, tools/parity/probes/editable_mesh_vertex_batch.luau)
    for (name, kind) in [("GetVertexFaces", FACE), ("GetVertexNormals", NORMAL), ("GetVertexColors", COLOR), ("GetVertexUVs", UV)] {
        add(name, method(move |lua, engine, id, args| {
            let v = id_arg(lua, engine, args, 1, VERTEX)?;
            with_mesh(engine, id, false, |m| {
                let v = m.positions.at(VERTEX, v)?;
                let ids: Vec<f64> = m.corners_of(v).into_iter().map(|(f, corner)| vertex_corner_id(m, f, corner, kind)).collect();
                Ok(values([ids_value(lua, ids.into_iter())?]))
            })
        }));
    }
    // BatchGetVertexAttributes(attribute, vertexIds): per vertex, what GetVertexFaces / GetVertexNormals / GetVertexColors /
    // GetVertexUVs list (measured: Vertex is not supported; each id is checked in turn, a number or a numeric string, 0 and
    // an id that is gone "Invalid id", one of another kind named; the client's argument errors)
    add("BatchGetVertexAttributes", method(|lua, engine, id, args| {
        const METHOD: &str = "BatchGetVertexAttributes";
        if args.len() != 3 {
            return Err(err(format!("{METHOD}: expects 2 arguments (attr, vertexIds)")));
        }
        let which = mesh_attribute(engine, args.get(1))?;
        if which == 0 {
            return Err(err(format!("{METHOD}: Enum.MeshAttribute.Vertex is not supported")));
        }
        let LuaValue::Table(table) = args.get(2) else { return Err(err(format!("{METHOD}: ids must be an array"))) };
        let items = sequence(table);
        let out = with_mesh(engine, id, false, |m| {
            let mut vertices = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let v = element_id(lua, item, METHOD, "id", index + 1)?;
                check_kind(v, VERTEX)?;
                vertices.push(m.positions.at(VERTEX, v)?);
            }
            let corners = m.corners_of_all(&vertices);
            let out = lua.create_table_with_capacity(vertices.len(), 0)?;
            for (index, v) in vertices.iter().enumerate() {
                let ids = corners[v].iter().map(|&(f, corner)| vertex_corner_id(m, f, corner, which as u64 + 1));
                out.raw_set(index + 1, ids_value(lua, ids)?)?;
            }
            Ok(out)
        })?;
        Ok(values([LuaValue::Table(out)]))
    }));
    // GetAdjacentVertices: per corner over the vertex, front first, the corner before it, then the one after it unless some
    // corner over the vertex has that one before it (measured on five vertices of a three-face mesh in a 0.747 client, and on
    // quads: the corners around a quad's corner)
    add("GetAdjacentVertices", method(|lua, engine, id, args| {
        let v = id_arg(lua, engine, args, 1, VERTEX)?;
        with_mesh(engine, id, false, |m| {
            let v = m.positions.at(VERTEX, v)?;
            let corners = m.corners_of(v);
            let around = |f: u32, corner: usize, after: bool| {
                let face = m.faces.get(f).expect("live");
                let (before, next) = face.around(corner);
                face.vertices[if after { next } else { before }]
            };
            let before: Vec<u32> = corners.iter().map(|&(f, corner)| around(f, corner, false)).collect();
            let mut out: Vec<f64> = Vec::new();
            for (&(f, corner), &previous) in corners.iter().zip(&before) {
                out.push(m.positions.id(VERTEX, previous));
                let next = around(f, corner, true);
                if !before.contains(&next) {
                    out.push(m.positions.id(VERTEX, next));
                }
            }
            Ok(values([ids_value(lua, out.into_iter())?]))
        })
    }));
    // GetAdjacentFaces: the faces sharing an edge with it (a quad's diagonal is no edge: measured)
    add("GetAdjacentFaces", method(|lua, engine, id, args| {
        let f = id_arg(lua, engine, args, 1, FACE)?;
        with_mesh(engine, id, false, |m| {
            let f = m.faces.at(FACE, f)?;
            let face = *m.faces.get(f).expect("live");
            let edges: Vec<(u32, u32)> = face.edges().collect();
            let shares = |other: &Face| other.edges().any(|(a, b)| edges.iter().any(|&(c, d)| (a == c && b == d) || (a == d && b == c)));
            let out: Vec<f64> = m.faces.by_slot().filter(|(i, other)| *i != f && shares(other)).map(|(i, _)| m.faces.id(FACE, i)).collect();
            Ok(values([ids_value(lua, out.into_iter())?]))
        })
    }));

    // The attribute at one corner: GetVertexFaceNormal / UV / Color(vertexId, faceId) and the setters (measured in a 0.747
    // client: the face is checked, then the corner found, the first in the face over the vertex, the vertex id itself
    // never checked, so one that is no vertex of the face is "vertexId not found on requested face"; a setter's id is
    // checked after; a corner given another one comes to the front of its vertex's corners)
    for (name, kind) in [("GetVertexFaceNormal", NORMAL), ("GetVertexFaceColor", COLOR), ("GetVertexFaceUV", UV)] {
        add(name, method(move |lua, engine, id, args| {
            let (v, f) = (int64_arg(lua, engine, args, 1)?, int64_arg(lua, engine, args, 2)?);
            check_kind(f, FACE)?;
            let found = with_mesh(engine, id, false, |m| {
                let (f, corner) = face_corner(m, v, f)?;
                Ok(vertex_corner_id(m, f, corner, kind))
            })?;
            Ok(values([LuaValue::Number(found)]))
        }));
    }
    for (name, kind) in [("SetVertexFaceNormal", NORMAL), ("SetVertexFaceColor", COLOR), ("SetVertexFaceUV", UV)] {
        add(name, method(move |lua, engine, id, args| {
            let (v, f, a) = (int64_arg(lua, engine, args, 1)?, int64_arg(lua, engine, args, 2)?, int64_arg(lua, engine, args, 3)?);
            check_kind(f, FACE)?;
            with_mesh(engine, id, true, |m| {
                let (f, corner) = face_corner(m, v, f)?;
                check_kind(a, kind)?;
                let slot = attribute_slot(m, kind, a)?;
                m.set_corner(f, corner, (kind - 1) as u32, slot);
                Ok(none())
            })
        }));
    }
    // BatchGetVertexFaceAttributes(attribute, vertexIds, faceIds): the normal, colour or uv id at each vertex's corner of
    // its face (measured: Vertex and Face are not supported; each pair is read, then checked, in turn)
    add("BatchGetVertexFaceAttributes", method(|lua, engine, id, args| {
        const METHOD: &str = "BatchGetVertexFaceAttributes";
        if args.len() != 4 {
            return Err(err(format!("{METHOD}: expects 3 arguments (attr, vertexIds, faceIds)")));
        }
        let which = mesh_attribute(engine, args.get(1))?;
        if which == 0 || which == 4 {
            return Err(err(format!("{METHOD}: Enum.MeshAttribute.{} is not supported", if which == 0 { "Vertex" } else { "Face" })));
        }
        let LuaValue::Table(vertices) = args.get(2) else { return Err(err(format!("{METHOD}: vertexIds must be an array"))) };
        let LuaValue::Table(faces) = args.get(3) else { return Err(err(format!("{METHOD}: faceIds must be an array"))) };
        let (vertices, faces) = (sequence(vertices), sequence(faces));
        if vertices.len() != faces.len() {
            return Err(err(format!("{METHOD}: vertexIds and faceIds arrays must be the same length")));
        }
        let found = with_mesh(engine, id, false, |m| {
            let mut found = Vec::with_capacity(vertices.len());
            for (index, (vertex, face)) in vertices.iter().zip(&faces).enumerate() {
                let (v, f) = read_pair(lua, METHOD, index + 1, vertex, face)?;
                let (f, corner) = pair_corner(m, METHOD, index + 1, v, f)?;
                found.push(vertex_corner_id(m, f, corner, which as u64 + 1));
            }
            Ok(found)
        })?;
        Ok(values([ids_value(lua, found.into_iter())?]))
    }));
    // BatchSetVertexFaceAttributes(vertexIds, faceIds, attributeIds): sets each vertex's corner of its face to a normal, a
    // colour or a uv, all of the kind of the first id (measured: pairs set in turn, so an error leaves the ones before it
    // set; a corner given another one comes to the front of its vertex's corners; allowed on a fixed-size mesh)
    add("BatchSetVertexFaceAttributes", method(|lua, engine, id, args| {
        const METHOD: &str = "BatchSetVertexFaceAttributes";
        if args.len() != 4 {
            return Err(err(format!("{METHOD}: expects 3 arguments (vertexIds, faceIds, attributeIds)")));
        }
        let LuaValue::Table(vertices) = args.get(1) else { return Err(err(format!("{METHOD}: vertexIds must be an array"))) };
        let vertices = sequence(vertices);
        let faces = match args.get(2) {
            LuaValue::Table(faces) if faces.raw_len() == vertices.len() => sequence(faces),
            _ => return Err(err(format!("{METHOD}: faceIds must be an array the same size as vertexIds"))),
        };
        let attributes = match args.get(3) {
            LuaValue::Table(ids) if ids.raw_len() == vertices.len() => sequence(ids),
            _ => return Err(err(format!("{METHOD}: attributeIds must be an array the same size as vertexIds"))),
        };
        let Some(first) = attributes.first() else { return Ok(none()) };
        let kind = parts(element_id(lua, first, METHOD, "attributeId", 1)?).0;
        if !matches!(kind, NORMAL | COLOR | UV) {
            return Err(err(format!("{METHOD}: unsupported attribute ID type {kind}")));
        }
        with_mesh(engine, id, true, |m| {
            for (index, ((vertex, face), attribute)) in vertices.iter().zip(&faces).zip(&attributes).enumerate() {
                let (v, f) = read_pair(lua, METHOD, index + 1, vertex, face)?;
                let a = element_id(lua, attribute, METHOD, "attributeId", index + 1)?;
                let (f, corner) = pair_corner(m, METHOD, index + 1, v, f)?;
                check_kind(a, kind)?;
                let slot = attribute_slot(m, kind, a)?;
                m.set_corner(f, corner, (kind - 1) as u32, slot);
            }
            Ok(none())
        })
    }));
    // BatchRemove(faceIds): removes faces, nothing else (measured in a 0.747 client: every id is checked first, so a bad one
    // removes none; another kind, 0 and a value that is no number are named as ids of no kind, a face that is gone is
    // "Invalid faceId"; a face named twice goes once; what the faces used stays until RemoveUnused; refused on a
    // fixed-size mesh; the slots are freed from the last listed face back, as Mesh::remove_faces says)
    add("BatchRemove", method(|lua, engine, id, args| {
        refuse_fixed(engine, id)?;
        if args.len() != 2 {
            return Err(err("BatchRemove: expects 1 argument (faceIds)"));
        }
        let LuaValue::Table(table) = args.get(1) else { return Err(err("BatchRemove: faceIds must be an array")) };
        let items = sequence(table);
        with_mesh(engine, id, true, |m| {
            let mut faces = Vec::with_capacity(items.len());
            for item in &items {
                let number = match item {
                    LuaValue::Integer(n) => *n as f64,
                    LuaValue::Number(n) => *n,
                    LuaValue::String(_) => lua.coerce_number(item.clone()).ok().flatten().unwrap_or(0.0),
                    _ => 0.0,
                };
                let f = id_of_number(number);
                if parts(f).0 != FACE {
                    return Err(wrong_kind(FACE, f));
                }
                faces.push(m.faces.find(FACE, f).ok_or_else(|| err("Invalid faceId"))?);
            }
            m.remove_faces(&faces);
            Ok(none())
        })
    }));

    // GetFacesWithAttribute / GetVerticesWithAttribute and the per-kind lists (measured in a 0.747 client, 7 meshes with every
    // attribute listed: tools/parity/probes/editable_mesh_queries.luau). The id is cast the Luau way. Per kind
    // (GetFacesWithNormal / UV / Color, GetVerticesWith...): one entry per corner that uses the attribute, front first,
    // duplicates kept; an id of another kind is named, 0 and gone "Invalid id". WithAttribute: for a vertex, its faces (each
    // once, in its corners' order) or itself; for a face, its vertices (GetVerticesWithAttribute only); for a normal, uv or
    // colour, the faces around the vertex of the attribute's front corner, that corner's face first, and the vertices of its
    // corners, each once, in id order; another id "Invalid attribute type index used".
    for (name, kind, faces) in [
        ("GetFacesWithNormal", NORMAL, true),
        ("GetFacesWithUV", UV, true),
        ("GetFacesWithColor", COLOR, true),
        ("GetVerticesWithNormal", NORMAL, false),
        ("GetVerticesWithUV", UV, false),
        ("GetVerticesWithColor", COLOR, false),
    ] {
        add(name, method(move |lua, engine, id, args| {
            let a = luau_int64(lua, engine, args, 1, name)?;
            check_kind(a, kind)?;
            let ids = with_mesh(engine, id, false, |m| {
                let slot = attribute_slot(m, kind, a)?;
                let corners = m.attribute_corners(kind, slot);
                Ok(corners.iter().map(|&(f, corner)| if faces { m.faces.id(FACE, f) } else { vertex_corner_id(m, f, corner, VERTEX) }).collect::<Vec<f64>>())
            })?;
            Ok(values([ids_value(lua, ids.into_iter())?]))
        }));
    }
    for (name, faces) in [("GetFacesWithAttribute", true), ("GetVerticesWithAttribute", false)] {
        add(name, method(move |lua, engine, id, args| {
            let a = luau_int64(lua, engine, args, 1, name)?;
            let kind = parts(a).0;
            let ids = with_mesh(engine, id, false, |m| {
                let mut out: Vec<f64> = Vec::new();
                let mut push = |id: f64| {
                    if !out.contains(&id) {
                        out.push(id);
                    }
                };
                match kind {
                    VERTEX => {
                        let v = m.positions.at(VERTEX, a)?;
                        if faces {
                            for (f, _) in m.corners_of(v) {
                                push(m.faces.id(FACE, f));
                            }
                        } else if !m.corners_of(v).is_empty() {
                            // (a vertex no face uses lists nothing: measured)
                            push(m.positions.id(VERTEX, v));
                        }
                    }
                    FACE if !faces => {
                        let f = m.faces.at(FACE, a)?;
                        let face = *m.faces.get(f).expect("live");
                        for corner in 0..face.len() {
                            push(m.positions.id(VERTEX, face.vertices[corner]));
                        }
                    }
                    NORMAL | COLOR | UV => {
                        let slot = attribute_slot(m, kind, a)?;
                        let corners = m.attribute_corners(kind, slot);
                        if faces {
                            if let Some(&(f, corner)) = corners.first() {
                                push(m.faces.id(FACE, f));
                                let v = m.faces.get(f).expect("live").vertices[corner];
                                for (f, _) in m.corners_of(v) {
                                    push(m.faces.id(FACE, f));
                                }
                            }
                        } else {
                            let mut ids: Vec<i64> = corners.iter().map(|&(f, corner)| vertex_corner_id(m, f, corner, VERTEX) as i64).collect();
                            ids.sort_unstable();
                            ids.dedup();
                            for id in ids {
                                push(id as f64);
                            }
                        }
                    }
                    _ => return Err(err("Invalid attribute type index used")),
                }
                Ok(out)
            })?;
            Ok(values([ids_value(lua, ids.into_iter())?]))
        }));
    }

    // FindClosestVertex(point): the vertex nearest it, used by a face or not, the first in slot order of those as near (measured
    // in a 0.747 client); 0 for a mesh with none
    add("FindClosestVertex", method(|_, engine, id, args| {
        let p = vector3_arg(engine, args, 1)?;
        let found = with_mesh(engine, id, false, |m| {
            let mut best: Option<(f32, u32)> = None;
            for (slot, q) in m.positions.by_slot() {
                let distance = (p - *q).length_squared();
                if best.is_none_or(|(nearest, _)| distance < nearest) {
                    best = Some((distance, slot));
                }
            }
            engine.state.borrow().sched.charge(m.positions.count() as f64 * crate::cost::LISTED_VERTEX);
            Ok(best.map_or(0.0, |(_, slot)| m.positions.id(VERTEX, slot)))
        })?;
        Ok(values([LuaValue::Number(found)]))
    }));
    // FindVerticesWithinSphere(center, radius): the vertices whose squared distance is at most the radius squared (a negative
    // radius as its size), in listing order (measured in a 0.747 client)
    add("FindVerticesWithinSphere", method(|lua, engine, id, args| {
        let radius = float_arg(lua, engine, args, 2)?;
        let center = vector3_arg(engine, args, 1)?;
        let ids = with_mesh(engine, id, false, |m| {
            let reach = radius * radius;
            engine.state.borrow().sched.charge(m.positions.count() as f64 * crate::cost::LISTED_VERTEX);
            Ok(m.positions.live().filter(|(_, q)| (center - **q).length_squared() <= reach).map(|(slot, _)| m.positions.id(VERTEX, slot)).collect::<Vec<f64>>())
        })?;
        Ok(values([ids_value(lua, ids.into_iter())?]))
    }));
    // FindClosestPointOnSurface(point): the face nearest it, the nearest point on it, that point's barycentric weights in the
    // triangle of the face it lies in (Mesh::query_triangles), and that triangle's three vertex ids (measured in a 0.747 client;
    // a mesh without faces "EditableMesh Error: Empty Result"). Not followed: which face an exact tie names (a client's own
    // search order) and the weights' last float bits.
    add("FindClosestPointOnSurface", method(|lua, engine, id, args| {
        let p = vector3_arg(engine, args, 1)?;
        with_mesh(engine, id, false, |m| {
            let mut best: Option<(f32, u32, [u32; 3], Vec3)> = None;
            for (f, face) in m.faces.live() {
                let (first, second) = m.query_triangles(face);
                for corners in std::iter::once(first).chain(second) {
                    let ids = corners.map(|corner| face.vertices[corner]);
                    let [a, b, c] = ids.map(|v| m.positions.get(v).copied().unwrap_or(Vec3::ZERO));
                    let q = closest_on_triangle(p, a, b, c);
                    let distance = (p - q).length_squared();
                    if best.is_none_or(|(nearest, ..)| distance < nearest) {
                        best = Some((distance, f, ids, q));
                    }
                }
            }
            let Some((_, f, ids, q)) = best else { return Err(err("EditableMesh Error: Empty Result")) };
            m.query_answer(lua, engine, f, ids, q)
        })
    }));
    // RaycastLocal(origin, direction): the face the segment from origin to origin + direction meets first (both sides, its end
    // excluded), as FindClosestPointOnSurface answers; nothing at all when it meets none (measured in a 0.747 client). Not
    // followed: which face a ray through a shared edge names and the point's and weights' last float bits.
    add("RaycastLocal", method(|lua, engine, id, args| {
        let direction = vector3_arg(engine, args, 2)?;
        let origin = vector3_arg(engine, args, 1)?;
        with_mesh(engine, id, false, |m| {
            let mut best: Option<(f32, u32, [u32; 3])> = None;
            for (f, face) in m.faces.live() {
                let (first, second) = m.query_triangles(face);
                for corners in std::iter::once(first).chain(second) {
                    let ids = corners.map(|corner| face.vertices[corner]);
                    let [a, b, c] = ids.map(|v| m.positions.get(v).copied().unwrap_or(Vec3::ZERO));
                    if let Some(t) = ray_triangle(origin, direction, a, b, c) {
                        if best.is_none_or(|(nearest, ..)| t < nearest) {
                            best = Some((t, f, ids));
                        }
                    }
                }
            }
            match best {
                Some((t, f, ids)) => m.query_answer(lua, engine, f, ids, origin + direction * t),
                None => Ok(LuaMultiValue::new()),
            }
        })
    }));
    // MergeVertices(tolerance): Mesh::merge_vertices (allowed on a fixed-size mesh, measured)
    add("MergeVertices", method(|lua, engine, id, args| {
        let tolerance = float_arg(lua, engine, args, 1)?;
        let map = with_mesh(engine, id, true, |m| Ok(m.merge_vertices(tolerance)))?;
        engine.state.borrow().sched.charge(map.1 as f64 * crate::cost::REMOVE_UNUSED_VERTEX);
        let out = lua.create_table()?;
        for (from, to) in map.0 {
            out.raw_set(format!("{}", from as i64), to)?;
        }
        Ok(values([LuaValue::Table(out)]))
    }));
    // IdDebugString(id): id_debug_string (the id cast as an int64 argument, a numeric string read exactly)
    add("IdDebugString", method(|lua, engine, _, args| {
        let id = convert::int64_argument(engine, args.get(1), 1).map_err(err)?;
        Ok(values([LuaValue::String(lua.create_string(id_debug_string(id))?)]))
    }));

    // GetBoneByName(name): the name read as a client reads a string parameter (services::strings); one no bone has raises 'No
    // bone named "<name>"' (measured in a 0.747 client)
    add("GetBoneByName", strings(FIRST, method(|lua, engine, id, args| {
        let name = args.string(1).map(|s| s.to_string_lossy()).unwrap_or_default();
        let found = with_mesh(engine, id, false, |m| Ok(m.bones.live().find(|(_, b)| b.name == name).map(|(i, _)| m.bones.id(BONE, i))))?;
        let _ = lua;
        Ok(values([LuaValue::Number(found.ok_or_else(|| err(format!("No bone named \"{name}\"")))?)]))
    })));
    // RemoveBone: its children lose their parent, its vertices its weights, the FACS poses the bone
    add("RemoveBone", method(|lua, engine, id, args| {
        let b = int64_arg(lua, engine, args, 1)?;
        refuse_fixed(engine, id)?;
        check_kind(b, BONE)?;
        with_mesh(engine, id, true, |m| {
            let b = m.bones.at(BONE, b)?;
            m.bones.remove(b);
            for bone in m.bones.items.iter_mut().flatten() {
                if bone.parent == Some(b) {
                    bone.parent = None;
                }
            }
            for (bones, weights) in m.skin.values_mut() {
                while let Some(at) = bones.iter().position(|&x| x == b) {
                    bones.remove(at);
                    if at < weights.len() {
                        weights.remove(at);
                    }
                }
            }
            m.drop_bone_poses(b);
            Ok(none())
        })
    }));
    add("ResetNormal", method(|lua, engine, id, args| {
        let n = id_arg(lua, engine, args, 1, NORMAL)?;
        with_mesh(engine, id, true, |m| {
            let slot = m.normals.at(NORMAL, n)?;
            *m.normals.get_mut(slot).expect("live") = None;
            Ok(none())
        })
    }));
    add("Clear", method(|_, engine, id, _| {
        with_mesh(engine, id, true, |m| {
            *m = Mesh::default();
            Ok(none())
        })
    }));

    // FACS poses of virtual bones, per action and per corrective (2 or 3 actions); measured in a 0.747 client: casts last to
    // first; a set pose replaces the action's (facs_pose), its identity CFrames dropped; GetFacsPose answers its bones' ids
    // and CFrames (two arrays, bones in id order), GetFacsPoses the actions with a pose in value order, GetFacsCorrectivePoses
    // the correctives' actions; refused on a fixed-size mesh
    add("SetFacsPose", method(|lua, engine, id, args| {
        let frames = array_arg(engine, args, 3)?;
        let ids = array_arg(engine, args, 2)?;
        let action = facs_unit(lua, engine, args.get(1), 1)?;
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let pose = facs_pose(m, &ids, &frames)?;
            let pose = settle_pose(m, pose);
            if pose.is_empty() { m.facs.remove(&action); } else { m.facs.insert(action, pose); }
            Ok(none())
        })
    }));
    add("SetFacsBonePose", method(|lua, engine, id, args| {
        let frame = cframe_arg(engine, args, 3)?;
        let bone = int64_arg(lua, engine, args, 2)?;
        let action = facs_unit(lua, engine, args.get(1), 1)?;
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let slot = facs_bone(m, bone)?;
            let mut pose = m.facs.remove(&action).unwrap_or_default();
            match pose.iter_mut().find(|(at, _)| *at == slot) {
                Some((_, held)) => *held = frame,
                None => pose.push((slot, frame)),
            }
            let pose = settle_pose(m, pose);
            if !pose.is_empty() {
                m.facs.insert(action, pose);
            }
            Ok(none())
        })
    }));
    add("GetFacsPose", method(|lua, engine, id, args| {
        let action = facs_unit(lua, engine, args.get(1), 1)?;
        with_mesh(engine, id, false, |m| Ok(pose_values(lua, engine, m, m.facs.get(&action))?))
    }));
    add("GetFacsPoses", method(|lua, engine, id, _| {
        let actions = with_mesh(engine, id, false, |m| Ok(m.facs.keys().copied().collect::<Vec<u32>>()))?;
        let out = lua.create_table()?;
        for (index, action) in actions.into_iter().enumerate() {
            if let Some(item) = facs_item(engine, action) {
                out.raw_set(index + 1, convert::to_lua(lua, engine, &item)?)?;
            }
        }
        Ok(values([LuaValue::Table(out)]))
    }));
    add("SetFacsCorrectivePose", method(|lua, engine, id, args| {
        let frames = array_arg(engine, args, 3)?;
        let ids = array_arg(engine, args, 2)?;
        let actions = corrective_actions(lua, engine, args, 1)?;
        refuse_fixed(engine, id)?;
        with_mesh(engine, id, true, |m| {
            let pose = facs_pose(m, &ids, &frames)?;
            let pose = settle_pose(m, pose);
            if pose.is_empty() { m.correctives.remove(&actions); } else { m.correctives.insert(actions, pose); }
            Ok(none())
        })
    }));
    add("GetFacsCorrectivePose", method(|lua, engine, id, args| {
        let actions = corrective_actions(lua, engine, args, 1)?;
        with_mesh(engine, id, false, |m| Ok(pose_values(lua, engine, m, m.correctives.get(&actions))?))
    }));
    add("GetFacsCorrectivePoses", method(|lua, engine, id, _| {
        let correctives = with_mesh(engine, id, false, |m| Ok(m.correctives.keys().cloned().collect::<Vec<Vec<u32>>>()))?;
        let out = lua.create_table()?;
        for (index, actions) in correctives.into_iter().enumerate() {
            let list = lua.create_table()?;
            for (at, action) in actions.into_iter().enumerate() {
                if let Some(item) = facs_item(engine, action) {
                    list.raw_set(at + 1, convert::to_lua(lua, engine, &item)?)?;
                }
            }
            out.raw_set(index + 1, list)?;
        }
        Ok(values([LuaValue::Table(out)]))
    }));

    add("Destroy", method(|_, engine, id, _| {
        let ext = extension(engine);
        ext.meshes.borrow_mut().remove(&id);
        ext.dirty.borrow_mut().remove(&id);
        Ok(none())
    }));
}

#[cfg(test)]
pub(crate) fn test_mesh_bytes(extent: f32, faces: usize) -> Vec<u8> {
    let mut mesh = Mesh::default();
    let a = mesh.positions.add(Vec3::ZERO);
    let b = mesh.positions.add(Vec3::new(extent, 0.0, 0.0));
    let c = mesh.positions.add(Vec3::new(0.0, extent, extent));
    let n = mesh.normals.add(Some(Vec3::Z));
    let color = mesh.colors.add(([1.0, 1.0, 1.0], 1.0));
    let uv = mesh.uvs.add([0.0, 0.0]);
    for _ in 0..faces.max(1) {
        mesh.faces.add(Face { count: 3, vertices: [a, b, c, 0], normals: [n, n, n, 0], colors: [color, color, color, 0], uvs: [uv, uv, uv, 0], stamps: [1, 2, 3, 0] });
    }
    encode(&mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mlua hands a whole Luau number over as its Integer when it fits one, else as a Number. Its Integer is 64 bits on
    /// most targets but 32 on wasm32 (a browser build), where every id past 2^31 (each EditableMesh id) arrives as a
    /// Number: the id readers give the same id from either.
    #[test]
    fn mesh_ids_read_the_same_from_an_integer_or_a_number() {
        let lua = Lua::new();
        let engine = Engine::new(crate::Options::default()).expect("engine");
        let mut mesh = Mesh::default();
        let vertex = mesh.positions.add(Vec3::ZERO);
        let vertex_id = mesh.positions.id(VERTEX, vertex) as i64;
        assert!(vertex_id > i64::from(i32::MAX), "a vertex id is past 32 bits: {vertex_id}");
        let ids = [0, 1, -1, i64::from(i32::MAX), i64::from(i32::MAX) + 1, i64::from(i32::MIN), vertex_id, (7 << 32) | 0xffff_ffff, 1 << 53, -(1 << 53), 1 << 62, i64::MIN];
        let mut integers = 0;
        for id in ids {
            let number = LuaValue::Number(id as f64);
            assert_eq!(int64_arg(&lua, &engine, &Args::new(LuaMultiValue::from_iter([number.clone()])), 0).ok(), Some(id), "int64 {id}");
            assert_eq!(element_id(&lua, &number, "M", "ids", 1).ok(), Some(id), "element {id}");
            let vertex_of_number = face_vertex(&mesh, &number).ok();
            assert_eq!(vertex_of_number, (id == vertex_id).then_some(vertex), "face vertex {id}");
            // (the Integer arm only where this target's Integer holds the id)
            let Some(integer) = id.try_into().ok().map(LuaValue::Integer) else { continue };
            integers += 1;
            assert_eq!(int64_arg(&lua, &engine, &Args::new(LuaMultiValue::from_iter([integer.clone()])), 0).ok(), Some(id), "int64 {id}");
            assert_eq!(element_id(&lua, &integer, "M", "ids", 1).ok(), Some(id), "element {id}");
            assert_eq!(face_vertex(&mesh, &integer).ok(), vertex_of_number, "face vertex {id}");
        }
        assert!(integers >= 5, "{integers} ids as Integers");
    }
}
