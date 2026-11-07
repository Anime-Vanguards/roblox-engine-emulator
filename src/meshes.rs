//! EditableMesh offline: meshes scripts build or load from mesh assets, and the MeshParts that show
//! them. A MeshPart bound to an EditableMesh (CreateMeshPartAsync, ApplyMesh) points at a FileMesh
//! 4.00 snapshot of it under `<state dir>/editable-meshes/`, written again (named by its content)
//! before each frame the recorder samples after the mesh changed, so ror draws what the script made
//! as Roblox shows it: the mesh's own coordinates from the part's origin, scaled by Size / MeshSize
//! (InitialSize keeps MeshSize: ror scales by it), and skinned by its bones.
//!
//! Measured in Studio (2026-10-16): ids carry their kind in the high 32 bits (vertex 1, normal 2,
//! colour 3, uv 4, face 5, bone 6) over a count from 1; a clone keeps them. AddTriangle gives each
//! corner a new automatic normal (the face's), a white opaque colour and a (0, 0) uv. An automatic
//! normal no face uses reads (0, 0, 0). Bone weights keep 1/255 steps. A mesh asset loads one vertex
//! per file vertex (none merged), each with its own normal, uv and colour, the file's bones and bind
//! CFrames, and four bone slots a skinned vertex (zero weights included). MeshSize is the mesh's box
//! when the part is made (at least 0.001) and stays when the mesh changes; ApplyMesh copies MeshSize
//! and keeps Size. RemoveUnused returns the ids it removed.

use std::{
    cell::RefCell,
    hash::{Hash, Hasher},
    path::PathBuf,
    rc::Rc,
};

use glam::Vec3;
use mlua::prelude::*;
use rbx_dom_weak::types::{self as dom, Variant};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};

use crate::{
    Engine, InstanceId, NameId, ScriptError, State, files,
    instance as core,
    lua::{Args, Flow, Ret, convert, instance_value, none, values},
    services::{Behaviors, method},
    value::{ContentValue, Str, Value},
};

const VERTEX: u64 = 1;
const NORMAL: u64 = 2;
const COLOR: u64 = 3;
const UV: u64 = 4;
const FACE: u64 = 5;
const BONE: u64 = 6;

fn make_id(kind: u64, index: u32) -> f64 {
    ((kind << 32) | (index as u64 + 1)) as f64
}

fn split_id(id: f64) -> Option<(u64, u32)> {
    if !(id.is_finite() && id >= 0.0 && id.fract() == 0.0) {
        return None;
    }
    let id = id as u64;
    let low = id & 0xFFFF_FFFF;
    (low > 0).then(|| (id >> 32, (low - 1) as u32))
}

/// Items addressed by stable index; a removed one leaves a hole (ids are never reused).
#[derive(Clone, Debug)]
struct Slots<T>(Vec<Option<T>>);

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Slots<T> {
    fn add(&mut self, item: T) -> u32 {
        self.0.push(Some(item));
        (self.0.len() - 1) as u32
    }
    fn get(&self, index: u32) -> Option<&T> {
        self.0.get(index as usize).and_then(Option::as_ref)
    }
    fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        self.0.get_mut(index as usize).and_then(Option::as_mut)
    }
    fn remove(&mut self, index: u32) -> Option<T> {
        self.0.get_mut(index as usize).and_then(Option::take)
    }
    fn live(&self) -> impl Iterator<Item = (u32, &T)> {
        self.0.iter().enumerate().filter_map(|(index, item)| item.as_ref().map(|item| (index as u32, item)))
    }
}

#[derive(Clone, Copy, Debug)]
struct Face {
    vertices: [u32; 3],
    normals: [u32; 3],
    colors: [u32; 3],
    uvs: [u32; 3],
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
}

impl Mesh {
    fn face_normal(&self, face: &Face) -> Vec3 {
        let p = |i: usize| self.positions.get(face.vertices[i]).copied().unwrap_or(Vec3::ZERO);
        (p(1) - p(0)).cross(p(2) - p(0))
    }

    /// Every automatic normal's value: the normalised sum of the (area-weighted) normals of the faces
    /// that use it; (0, 0, 0) when none does.
    fn automatic_normals(&self) -> FxHashMap<u32, Vec3> {
        let mut sums: FxHashMap<u32, Vec3> = FxHashMap::default();
        for (_, face) in self.faces.live() {
            let normal = self.face_normal(face);
            for &n in &face.normals {
                if matches!(self.normals.get(n), Some(None)) {
                    *sums.entry(n).or_insert(Vec3::ZERO) += normal;
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
                for (_, face) in self.faces.live() {
                    if face.normals.contains(&index) {
                        sum += self.face_normal(face);
                    }
                }
                Some(sum.normalize_or_zero())
            }
        }
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

    /// Removes what no face uses: vertices, normals, colours, uvs (Studio's RemoveUnused); the ids.
    fn remove_unused(&mut self) -> Vec<f64> {
        let mut used = [FxHashSet::default(), FxHashSet::default(), FxHashSet::default(), FxHashSet::default()];
        for (_, face) in self.faces.live() {
            used[0].extend(face.vertices);
            used[1].extend(face.normals);
            used[2].extend(face.colors);
            used[3].extend(face.uvs);
        }
        let mut removed = Vec::new();
        let unused = |slots: Vec<u32>, used: &FxHashSet<u32>| slots.into_iter().filter(|i| !used.contains(i)).collect::<Vec<_>>();
        for index in unused(self.positions.live().map(|(i, _)| i).collect(), &used[0]) {
            self.positions.remove(index);
            self.skin.remove(&index);
            removed.push(make_id(VERTEX, index));
        }
        for index in unused(self.normals.live().map(|(i, _)| i).collect(), &used[1]) {
            self.normals.remove(index);
            removed.push(make_id(NORMAL, index));
        }
        for index in unused(self.colors.live().map(|(i, _)| i).collect(), &used[2]) {
            self.colors.remove(index);
            removed.push(make_id(COLOR, index));
        }
        for index in unused(self.uvs.live().map(|(i, _)| i).collect(), &used[3]) {
            self.uvs.remove(index);
            removed.push(make_id(UV, index));
        }
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
                mesh.faces.add(Face { vertices: c, normals: c, colors: c, uvs: c });
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
    let bone_order: Vec<u32> = mesh.bones.live().map(|(i, _)| i).collect();
    let bone_slot: FxHashMap<u32, u16> = bone_order.iter().enumerate().map(|(slot, &i)| (i, slot as u16)).collect();
    let skinned = !bone_order.is_empty();
    let faces: Vec<Face> = mesh.faces.live().map(|(_, f)| *f).collect();

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
            for &v in &face.vertices {
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
    /// Decoded asset meshes by uri (a warm service decodes each once per engine)
    #[cfg_attr(not(feature = "meshes"), allow(dead_code))]
    assets: RefCell<FxHashMap<String, Rc<Mesh>>>,
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

fn numbers_of(value: &LuaValue) -> Vec<f64> {
    match value {
        LuaValue::Table(table) => table.sequence_values::<f64>().filter_map(Result::ok).collect(),
        _ => Vec::new(),
    }
}

/// The index of `id` when it is of `kind` (a script error naming the kind otherwise).
fn index_of(args: &Args, at: usize, kind: u64, what: &str) -> Result<u32, Flow> {
    match args.number(at).and_then(split_id) {
        Some((k, index)) if k == kind => Ok(index),
        _ => Err(err(format!("Invalid {what} id"))),
    }
}

/// Runs `f` on the mesh `id`; `changed` marks it for a new snapshot.
fn with_mesh<T>(engine: &Engine, id: InstanceId, changed: bool, f: impl FnOnce(&mut Mesh) -> Result<T, Flow>) -> Result<T, Flow> {
    let ext = extension(engine);
    let result = {
        let mut meshes = ext.meshes.borrow_mut();
        let mesh = meshes.get_mut(&id).ok_or_else(|| err("This EditableMesh has been destroyed"))?;
        f(mesh)?
    };
    if changed {
        ext.dirty.borrow_mut().insert(id);
    }
    Ok(result)
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

/// `options.FixedSize` of a CreateEditableMesh options table.
fn fixed_option(value: &LuaValue) -> bool {
    match value {
        LuaValue::Table(table) => table.get::<Option<bool>>("FixedSize").ok().flatten().unwrap_or(false),
        _ => false,
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
        Value::Content(content) => content.uri().map(|uri| uri.to_string()),
        Value::String(uri) => Some(uri.to_string()),
        Value::Number(id) if id > 0.0 => Some(format!("rbxassetid://{}", id as u64)),
        _ => None,
    }
}

/// The mesh asset `uri`, decoded (cached per engine).
#[cfg(feature = "meshes")]
fn load_asset(engine: &Engine, uri: &str) -> Result<Rc<Mesh>, String> {
    if let Some(mesh) = extension(engine).assets.borrow().get(uri) {
        return Ok(mesh.clone());
    }
    files::note_unchecked("a mesh was read");
    // a local file (a relative one the project's), else an asset downloaded once
    let path = match engine.options.local_file(uri) {
        Some(path) => path,
        None => {
            let id = crate::animation::source::asset_id(uri).ok_or_else(|| format!("{uri}: not an asset id"))?;
            let cache = crate::state_dir(&engine.options.repo).join("meshes");
            let cached = cache.join(id.to_string());
            if !files::is_file(&cached) {
                crate::animation::source::fetch(engine, uri, &cache, &cached)?;
            }
            cached
        }
    };
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let file = ror_mesh::decode_mesh(&bytes).map_err(|error| format!("{uri}: {error}"))?;
    let mesh = Rc::new(Mesh::from_file(&file));
    extension(engine).assets.borrow_mut().insert(uri.to_string(), mesh.clone());
    Ok(mesh)
}

#[cfg(not(feature = "meshes"))]
fn load_asset(_: &Engine, uri: &str) -> Result<Rc<Mesh>, String> {
    Err(format!("{uri}: this build decodes no mesh assets (feature `meshes`)"))
}

/// Writes `mesh`'s snapshot (named by its content) and returns its path.
fn snapshot(engine: &Engine, mesh: &Mesh) -> Result<PathBuf, String> {
    files::note_unchecked("a mesh was written");
    let bytes = encode(mesh);
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    let directory = crate::state_dir(&engine.options.repo).join("editable-meshes");
    let path = directory.join(format!("{:016x}-{}.mesh", hasher.finish(), bytes.len()));
    if !files::is_file(&path) {
        std::fs::create_dir_all(&directory).map_err(|error| format!("{}: {error}", directory.display()))?;
        std::fs::write(&path, &bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    }
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

/// A new MeshPart showing `mesh` (CreateMeshPartAsync of an EditableMesh's Content).
fn mesh_part(lua: &Lua, engine: &Rc<Engine>, mesh_id: InstanceId, options: &LuaValue) -> Ret {
    let names = names(engine).ok_or_else(|| err("MeshPart is not available"))?;
    let (path, size, skinned) = {
        let ext = extension(engine);
        let meshes = ext.meshes.borrow();
        let mesh = meshes.get(&mesh_id).ok_or_else(|| err("This EditableMesh has been destroyed"))?;
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
/// keep no MeshSize; Roblox reads it from the mesh), else Size
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
    vector(state.dom.raw_get(id, names.mesh_size))
        .or_else(|| vector(state.dom.raw_get(id, names.initial_size)))
        .or_else(from_file)
        .or_else(|| state.dom.raw_get(id, names.size).cloned())
        .unwrap_or(Value::Vector3(dom::Vector3::new(0.0, 0.0, 0.0)))
}

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
    add("AssetService", "CreateEditableMesh", method(|lua, engine, _, args| new_mesh(lua, engine, Mesh::default(), fixed_option(args.get(1)))));
    add("AssetService", "CreateEditableMeshAsync", method(|lua, engine, _, args| {
        let fixed_size = fixed_option(args.get(2));
        if let Some(source) = content_object(engine, args.get(1)) {
            let mesh = extension(engine).meshes.borrow().get(&source).cloned().unwrap_or_default();
            engine.state.borrow().sched.charge(mesh.positions.live().count() as f64 * crate::cost::MESH_COPY_VERTEX);
            return new_mesh(lua, engine, mesh, fixed_size);
        }
        let Some(uri) = content_uri(args.get(1)) else {
            return Err(err("CreateEditableMeshAsync: expected a mesh Content"));
        };
        // Studio's wait (crate::cost): the asset's first fetch in the session, else its decode
        let first = !extension(engine).assets.borrow().contains_key(&uri);
        let mesh = match load_asset(engine, &uri) {
            Ok(mesh) => (*mesh).clone(),
            Err(problem) => {
                engine.warn(format!("EditableMesh: {problem}; it starts empty"));
                Mesh::default()
            }
        };
        let wait = if first { crate::cost::MESH_FETCH } else { mesh.positions.live().count() as f64 * crate::cost::MESH_DECODE_VERTEX };
        let id = new_mesh_id(engine, mesh, fixed_size)?;
        engine.state.borrow_mut().sched.schedule_task(crate::scheduler::TimerKind::Delay, wait, lua.current_thread(), vec![Value::Instance(id)]);
        Err(Flow::Yield)
    }));
    add("AssetService", "CreateMeshPartAsync", method(|lua, engine, _, args| {
        if let Some(source) = content_object(engine, args.get(1)) {
            let vertices = extension(engine).meshes.borrow().get(&source).map_or(0, |mesh| mesh.positions.live().count());
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
        let source = args.instance(1).ok_or_else(|| err("ApplyMesh: expected a MeshPart"))?;
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

    // Adding
    add("AddVertex", method(|_, engine, id, args| {
        let p = vec3_of(args.get(1)).ok_or_else(|| err("AddVertex: Vector3 expected"))?;
        if fixed(engine, id) {
            return Err(err("Cannot add vertices to a fixed-size EditableMesh"));
        }
        with_mesh(engine, id, true, |m| Ok(values([LuaValue::Number(make_id(VERTEX, m.positions.add(p)))])))
    }));
    add("AddNormal", method(|_, engine, id, args| {
        let n = vec3_of(args.get(1));
        with_mesh(engine, id, true, |m| Ok(values([LuaValue::Number(make_id(NORMAL, m.normals.add(n)))])))
    }));
    add("AddUV", method(|_, engine, id, args| {
        let uv = vec2_of(args.get(1)).ok_or_else(|| err("AddUV: Vector2 expected"))?;
        with_mesh(engine, id, true, |m| Ok(values([LuaValue::Number(make_id(UV, m.uvs.add(uv)))])))
    }));
    add("AddColor", method(|_, engine, id, args| {
        let c = color_of(args.get(1)).ok_or_else(|| err("AddColor: Color3 expected"))?;
        let a = alpha(args.number(2).unwrap_or(1.0));
        with_mesh(engine, id, true, |m| Ok(values([LuaValue::Number(make_id(COLOR, m.colors.add((c, a))))])))
    }));
    add("AddTriangle", method(|_, engine, id, args| {
        let v = [index_of(args, 1, VERTEX, "vertex")?, index_of(args, 2, VERTEX, "vertex")?, index_of(args, 3, VERTEX, "vertex")?];
        if fixed(engine, id) {
            return Err(err("Cannot add faces to a fixed-size EditableMesh"));
        }
        with_mesh(engine, id, true, |m| {
            if v.iter().any(|&i| m.positions.get(i).is_none()) {
                return Err(err("Invalid vertex id"));
            }
            let normals = [m.normals.add(None), m.normals.add(None), m.normals.add(None)];
            let colors = [m.colors.add(([1.0; 3], 1.0)), m.colors.add(([1.0; 3], 1.0)), m.colors.add(([1.0; 3], 1.0))];
            let uvs = [m.uvs.add([0.0; 2]), m.uvs.add([0.0; 2]), m.uvs.add([0.0; 2])];
            Ok(values([LuaValue::Number(make_id(FACE, m.faces.add(Face { vertices: v, normals, colors, uvs })))]))
        })
    }));
    add("AddBone", method(|_, engine, id, args| {
        let LuaValue::Table(params) = args.get(1) else { return Err(err("AddBone: table expected")) };
        let name: String = params.get::<Option<String>>("Name").ok().flatten().unwrap_or_default();
        let cframe = params.get::<LuaValue>("CFrame").ok().and_then(|v| cframe_of(&v)).unwrap_or([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let virtual_bone = params.get::<Option<bool>>("Virtual").ok().flatten().unwrap_or(false);
        with_mesh(engine, id, true, |m| Ok(values([LuaValue::Number(make_id(BONE, m.bones.add(Bone { name, cframe, parent: None, virtual_bone })))])))
    }));

    // Listing
    for (name, kind) in [("GetVertices", VERTEX), ("GetNormals", NORMAL), ("GetColors", COLOR), ("GetUVs", UV), ("GetFaces", FACE), ("GetBones", BONE)] {
        add(name, method(move |lua, engine, id, _| {
            with_mesh(engine, id, false, |m| {
                let ids: Vec<f64> = match kind {
                    VERTEX => m.positions.live().map(|(i, _)| make_id(kind, i)).collect(),
                    NORMAL => m.normals.live().map(|(i, _)| make_id(kind, i)).collect(),
                    COLOR => m.colors.live().map(|(i, _)| make_id(kind, i)).collect(),
                    UV => m.uvs.live().map(|(i, _)| make_id(kind, i)).collect(),
                    FACE => m.faces.live().map(|(i, _)| make_id(kind, i)).collect(),
                    _ => m.bones.live().map(|(i, _)| make_id(kind, i)).collect(),
                };
                let each = if kind == VERTEX { crate::cost::LISTED_VERTEX } else { crate::cost::LISTED };
                engine.state.borrow().sched.charge(ids.len() as f64 * each);
                Ok(values([ids_value(lua, ids.into_iter())?]))
            })
        }));
    }

    // Values
    add("GetPosition", method(|lua, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        let p = with_mesh(engine, id, false, |m| m.positions.get(v).copied().ok_or_else(|| err("Invalid vertex id")))?;
        Ok(values([vector_value(lua, engine, p)?]))
    }));
    add("SetPosition", method(|_, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        let p = vec3_of(args.get(2)).ok_or_else(|| err("SetPosition: Vector3 expected"))?;
        with_mesh(engine, id, true, |m| {
            *m.positions.get_mut(v).ok_or_else(|| err("Invalid vertex id"))? = p;
            Ok(none())
        })
    }));
    add("GetNormal", method(|lua, engine, id, args| {
        let n = index_of(args, 1, NORMAL, "normal")?;
        let value = with_mesh(engine, id, false, |m| m.normal_value(n).ok_or_else(|| err("Invalid normal id")))?;
        Ok(values([vector_value(lua, engine, value)?]))
    }));
    add("SetNormal", method(|_, engine, id, args| {
        let n = index_of(args, 1, NORMAL, "normal")?;
        let value = vec3_of(args.get(2));
        with_mesh(engine, id, true, |m| {
            *m.normals.get_mut(n).ok_or_else(|| err("Invalid normal id"))? = value;
            Ok(none())
        })
    }));
    add("GetUV", method(|lua, engine, id, args| {
        let i = index_of(args, 1, UV, "uv")?;
        let uv = with_mesh(engine, id, false, |m| m.uvs.get(i).copied().ok_or_else(|| err("Invalid uv id")))?;
        Ok(values([convert::to_lua(lua, engine, &Value::Vector2(dom::Vector2::new(uv[0], uv[1])))?]))
    }));
    add("SetUV", method(|_, engine, id, args| {
        let i = index_of(args, 1, UV, "uv")?;
        let uv = vec2_of(args.get(2)).ok_or_else(|| err("SetUV: Vector2 expected"))?;
        with_mesh(engine, id, true, |m| {
            *m.uvs.get_mut(i).ok_or_else(|| err("Invalid uv id"))? = uv;
            Ok(none())
        })
    }));
    add("GetColor", method(|lua, engine, id, args| {
        let i = index_of(args, 1, COLOR, "color")?;
        let (c, _) = with_mesh(engine, id, false, |m| m.colors.get(i).copied().ok_or_else(|| err("Invalid color id")))?;
        Ok(values([convert::to_lua(lua, engine, &Value::Color3(dom::Color3::new(c[0], c[1], c[2])))?]))
    }));
    add("SetColor", method(|_, engine, id, args| {
        let i = index_of(args, 1, COLOR, "color")?;
        let c = color_of(args.get(2)).ok_or_else(|| err("SetColor: Color3 expected"))?;
        with_mesh(engine, id, true, |m| {
            m.colors.get_mut(i).ok_or_else(|| err("Invalid color id"))?.0 = c;
            Ok(none())
        })
    }));
    add("GetColorAlpha", method(|_, engine, id, args| {
        let i = index_of(args, 1, COLOR, "color")?;
        let (_, a) = with_mesh(engine, id, false, |m| m.colors.get(i).copied().ok_or_else(|| err("Invalid color id")))?;
        Ok(values([LuaValue::Number(a as f64)]))
    }));
    add("SetColorAlpha", method(|_, engine, id, args| {
        let i = index_of(args, 1, COLOR, "color")?;
        let a = alpha(args.number(2).unwrap_or(1.0));
        with_mesh(engine, id, true, |m| {
            m.colors.get_mut(i).ok_or_else(|| err("Invalid color id"))?.1 = a;
            Ok(none())
        })
    }));

    // Faces
    for (name, which) in [("GetFaceVertices", 0usize), ("GetFaceNormals", 1), ("GetFaceColors", 2), ("GetFaceUVs", 3)] {
        add(name, method(move |lua, engine, id, args| {
            let f = index_of(args, 1, FACE, "face")?;
            with_mesh(engine, id, false, |m| {
                let face = m.faces.get(f).ok_or_else(|| err("Invalid face id"))?;
                let (kind, items) = match which {
                    0 => (VERTEX, face.vertices),
                    1 => (NORMAL, face.normals),
                    2 => (COLOR, face.colors),
                    _ => (UV, face.uvs),
                };
                Ok(values([ids_value(lua, items.iter().map(|&i| make_id(kind, i)))?]))
            })
        }));
    }
    for (name, which) in [("SetFaceVertices", 0usize), ("SetFaceNormals", 1), ("SetFaceColors", 2), ("SetFaceUVs", 3)] {
        add(name, method(move |_, engine, id, args| {
            let f = index_of(args, 1, FACE, "face")?;
            let kind = [VERTEX, NORMAL, COLOR, UV][which];
            let ids: Vec<u32> = numbers_of(args.get(2)).into_iter().filter_map(split_id).filter(|(k, _)| *k == kind).map(|(_, i)| i).collect();
            if ids.len() != 3 {
                return Err(err(format!("{name}: three ids of the right kind expected")));
            }
            with_mesh(engine, id, true, |m| {
                let face = m.faces.get_mut(f).ok_or_else(|| err("Invalid face id"))?;
                let target = match which {
                    0 => &mut face.vertices,
                    1 => &mut face.normals,
                    2 => &mut face.colors,
                    _ => &mut face.uvs,
                };
                *target = [ids[0], ids[1], ids[2]];
                Ok(none())
            })
        }));
    }
    add("RemoveFace", method(|_, engine, id, args| {
        let f = index_of(args, 1, FACE, "face")?;
        with_mesh(engine, id, true, |m| {
            m.faces.remove(f);
            Ok(none())
        })
    }));
    add("RemoveVertex", method(|_, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        with_mesh(engine, id, true, |m| {
            m.positions.remove(v);
            m.skin.remove(&v);
            let doomed: Vec<u32> = m.faces.live().filter(|(_, f)| f.vertices.contains(&v)).map(|(i, _)| i).collect();
            for f in doomed {
                m.faces.remove(f);
            }
            Ok(none())
        })
    }));
    add("RemoveUnused", method(|lua, engine, id, _| {
        let (removed, vertices) = with_mesh(engine, id, true, |m| {
            let vertices = m.positions.live().count();
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

    // Bones
    add("GetBoneName", method(|lua, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let name = with_mesh(engine, id, false, |m| m.bones.get(b).map(|b| b.name.clone()).ok_or_else(|| err("Invalid bone id")))?;
        Ok(values([LuaValue::String(lua.create_string(&name)?)]))
    }));
    add("SetBoneName", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let name = args.string(2).map(|s| s.to_string_lossy()).unwrap_or_default();
        with_mesh(engine, id, true, |m| {
            m.bones.get_mut(b).ok_or_else(|| err("Invalid bone id"))?.name = name;
            Ok(none())
        })
    }));
    add("GetBoneParent", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let parent = with_mesh(engine, id, false, |m| m.bones.get(b).map(|b| b.parent).ok_or_else(|| err("Invalid bone id")))?;
        Ok(values([LuaValue::Number(parent.map_or(0.0, |p| make_id(BONE, p)))]))
    }));
    add("SetBoneParent", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let parent = args.number(2).and_then(split_id).filter(|(k, _)| *k == BONE).map(|(_, i)| i);
        with_mesh(engine, id, true, |m| {
            m.bones.get_mut(b).ok_or_else(|| err("Invalid bone id"))?.parent = parent;
            Ok(none())
        })
    }));
    add("GetBoneCFrame", method(|lua, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let c = with_mesh(engine, id, false, |m| m.bones.get(b).map(|b| b.cframe).ok_or_else(|| err("Invalid bone id")))?;
        let cframe = dom::CFrame::new(dom::Vector3::new(c[0], c[1], c[2]), dom::Matrix3::new(dom::Vector3::new(c[3], c[4], c[5]), dom::Vector3::new(c[6], c[7], c[8]), dom::Vector3::new(c[9], c[10], c[11])));
        Ok(values([convert::to_lua(lua, engine, &Value::Dom(Rc::new(Variant::CFrame(cframe))))?]))
    }));
    add("SetBoneCFrame", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let c = cframe_of(args.get(2)).ok_or_else(|| err("SetBoneCFrame: CFrame expected"))?;
        with_mesh(engine, id, true, |m| {
            m.bones.get_mut(b).ok_or_else(|| err("Invalid bone id"))?.cframe = c;
            Ok(none())
        })
    }));
    add("GetBoneIsVirtual", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        let v = with_mesh(engine, id, false, |m| m.bones.get(b).map(|b| b.virtual_bone).ok_or_else(|| err("Invalid bone id")))?;
        Ok(values([LuaValue::Boolean(v)]))
    }));
    add("GetVertexBones", method(|lua, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        with_mesh(engine, id, false, |m| {
            let bones = m.skin.get(&v).map_or(Vec::new(), |(b, _)| b.iter().map(|&i| make_id(BONE, i)).collect());
            Ok(values([ids_value(lua, bones.into_iter())?]))
        })
    }));
    add("GetVertexBoneWeights", method(|lua, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        with_mesh(engine, id, false, |m| {
            let weights = m.skin.get(&v).map_or(Vec::new(), |(_, w)| w.iter().map(|&w| w as f64).collect());
            Ok(values([ids_value(lua, weights.into_iter())?]))
        })
    }));
    add("SetVertexBones", method(|_, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        let bones: Vec<u32> = numbers_of(args.get(2)).into_iter().filter_map(split_id).filter(|(k, _)| *k == BONE).map(|(_, i)| i).collect();
        with_mesh(engine, id, true, |m| {
            let entry = m.skin.entry(v).or_default();
            entry.1.resize(bones.len(), 0.0);
            entry.0 = bones;
            Ok(none())
        })
    }));
    add("SetVertexBoneWeights", method(|_, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        // (measured: weights keep 1/255 steps)
        let weights: Vec<f32> = numbers_of(args.get(2)).into_iter().map(|w| ((w.clamp(0.0, 1.0) * 255.0).round() / 255.0) as f32).collect();
        with_mesh(engine, id, true, |m| {
            let entry = m.skin.entry(v).or_default();
            entry.1 = weights;
            Ok(none())
        })
    }));

    // BatchSetValues / BatchGetValues: positions, normals, uvs, colours by their ids
    add("BatchSetValues", method(|_, engine, id, args| {
        let LuaValue::Table(ids) = args.get(1) else { return Err(err("BatchSetValues: table of ids expected")) };
        let LuaValue::Table(items) = args.get(2) else { return Err(err("BatchSetValues: table of values expected")) };
        let ids: Vec<f64> = ids.sequence_values::<f64>().filter_map(Result::ok).collect();
        let items: Vec<LuaValue> = items.sequence_values::<LuaValue>().filter_map(Result::ok).collect();
        with_mesh(engine, id, true, |m| {
            for (id, item) in ids.iter().zip(&items) {
                match split_id(*id) {
                    Some((VERTEX, i)) => {
                        if let (Some(slot), Some(p)) = (m.positions.get_mut(i), vec3_of(item)) {
                            *slot = p;
                        }
                    }
                    Some((NORMAL, i)) => {
                        if let Some(slot) = m.normals.get_mut(i) {
                            *slot = vec3_of(item);
                        }
                    }
                    Some((UV, i)) => {
                        if let (Some(slot), Some(uv)) = (m.uvs.get_mut(i), vec2_of(item)) {
                            *slot = uv;
                        }
                    }
                    // (measured: a number sets a colour's alpha, a Color3 its colour)
                    Some((COLOR, i)) => {
                        if let Some(slot) = m.colors.get_mut(i) {
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
        let ids = numbers_of(args.get(1));
        let out = lua.create_table()?;
        with_mesh(engine, id, false, |m| {
            for (index, id) in ids.iter().enumerate() {
                let value = match split_id(*id) {
                    Some((VERTEX, i)) => m.positions.get(i).map(|p| Value::Vector3(dom::Vector3::new(p.x, p.y, p.z))),
                    Some((NORMAL, i)) => m.normal_value(i).map(|p| Value::Vector3(dom::Vector3::new(p.x, p.y, p.z))),
                    Some((UV, i)) => m.uvs.get(i).map(|uv| Value::Vector2(dom::Vector2::new(uv[0], uv[1]))),
                    Some((COLOR, i)) => m.colors.get(i).map(|(c, _)| Value::Color3(dom::Color3::new(c[0], c[1], c[2]))),
                    _ => None,
                };
                let value = value.ok_or_else(|| err("BatchGetValues: invalid id"))?;
                out.raw_set(index + 1, convert::to_lua(lua, engine, &value)?)?;
            }
            Ok(())
        })?;
        Ok(values([LuaValue::Table(out)]))
    }));

    // Per-face and per-vertex attribute batches (Enum.MeshAttribute: Vertex 0, Normal 1, Color 2,
    // UV 3, Face 4)
    fn attribute(engine: &Engine, value: &LuaValue) -> Option<u32> {
        match convert::from_lua(value) {
            Value::EnumItem(item) => Some(engine.reflection.enum_item_info(item).value as u32),
            _ => None,
        }
    }
    fn face_ids(face: &Face, attribute: u32) -> Option<(u64, [u32; 3])> {
        match attribute {
            0 => Some((VERTEX, face.vertices)),
            1 => Some((NORMAL, face.normals)),
            2 => Some((COLOR, face.colors)),
            3 => Some((UV, face.uvs)),
            _ => None,
        }
    }
    add("BatchGetFaceAttributes", method(|lua, engine, id, args| {
        let which = attribute(engine, args.get(1)).ok_or_else(|| err("BatchGetFaceAttributes: MeshAttribute expected"))?;
        let faces = numbers_of(args.get(2));
        let out = lua.create_table_with_capacity(faces.len(), 0)?;
        with_mesh(engine, id, false, |m| {
            for (index, face) in faces.iter().enumerate() {
                let face = match split_id(*face) {
                    Some((FACE, f)) => m.faces.get(f),
                    _ => None,
                };
                let face = face.ok_or_else(|| err("Invalid face id"))?;
                let (kind, items) = face_ids(face, which).ok_or_else(|| err("BatchGetFaceAttributes: not a face attribute"))?;
                out.raw_set(index + 1, ids_value(lua, items.iter().map(|&i| make_id(kind, i)))?)?;
            }
            Ok(())
        })?;
        Ok(values([LuaValue::Table(out)]))
    }));
    add("BatchSetFaceAttributes", method(|_, engine, id, args| {
        let faces = numbers_of(args.get(1));
        let LuaValue::Table(arrays) = args.get(2) else { return Err(err("BatchSetFaceAttributes: table expected")) };
        let arrays: Vec<Vec<f64>> = arrays.sequence_values::<LuaValue>().filter_map(Result::ok).map(|v| numbers_of(&v)).collect();
        with_mesh(engine, id, true, |m| {
            for (face, ids) in faces.iter().zip(&arrays) {
                let Some((FACE, f)) = split_id(*face) else { return Err(err("Invalid face id")) };
                let parsed: Vec<(u64, u32)> = ids.iter().filter_map(|&i| split_id(i)).collect();
                if parsed.len() != 3 || parsed.iter().any(|(k, _)| *k != parsed[0].0) {
                    return Err(err("BatchSetFaceAttributes: three ids of one kind expected"));
                }
                let face = m.faces.get_mut(f).ok_or_else(|| err("Invalid face id"))?;
                let target = match parsed[0].0 {
                    VERTEX => &mut face.vertices,
                    NORMAL => &mut face.normals,
                    COLOR => &mut face.colors,
                    UV => &mut face.uvs,
                    _ => return Err(err("BatchSetFaceAttributes: not a face attribute")),
                };
                *target = [parsed[0].1, parsed[1].1, parsed[2].1];
            }
            Ok(none())
        })
    }));
    // A vertex's faces, and the normals / colours / uvs its corners use (first use first)
    fn vertex_corners(m: &Mesh, v: u32) -> Vec<(u32, usize)> {
        m.faces.live().flat_map(|(f, face)| face.vertices.iter().enumerate().filter(move |(_, x)| **x == v).map(move |(c, _)| (f, c))).collect()
    }
    add("GetVertexFaces", method(|lua, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        with_mesh(engine, id, false, |m| {
            let mut faces: Vec<f64> = Vec::new();
            for (f, _) in vertex_corners(m, v) {
                let id = make_id(FACE, f);
                if !faces.contains(&id) {
                    faces.push(id);
                }
            }
            Ok(values([ids_value(lua, faces.into_iter())?]))
        })
    }));
    for (name, which) in [("GetVertexNormals", 1u32), ("GetVertexColors", 2), ("GetVertexUVs", 3)] {
        add(name, method(move |lua, engine, id, args| {
            let v = index_of(args, 1, VERTEX, "vertex")?;
            with_mesh(engine, id, false, |m| {
                let mut out: Vec<f64> = Vec::new();
                for (f, corner) in vertex_corners(m, v) {
                    let face = m.faces.get(f).unwrap();
                    let (kind, items) = face_ids(face, which).unwrap();
                    let id = make_id(kind, items[corner]);
                    if !out.contains(&id) {
                        out.push(id);
                    }
                }
                Ok(values([ids_value(lua, out.into_iter())?]))
            })
        }));
    }
    add("GetAdjacentVertices", method(|lua, engine, id, args| {
        let v = index_of(args, 1, VERTEX, "vertex")?;
        with_mesh(engine, id, false, |m| {
            let mut out: Vec<f64> = Vec::new();
            for (f, _) in vertex_corners(m, v) {
                for &other in &m.faces.get(f).unwrap().vertices {
                    let id = make_id(VERTEX, other);
                    if other != v && !out.contains(&id) {
                        out.push(id);
                    }
                }
            }
            Ok(values([ids_value(lua, out.into_iter())?]))
        })
    }));
    add("GetAdjacentFaces", method(|lua, engine, id, args| {
        let f = index_of(args, 1, FACE, "face")?;
        with_mesh(engine, id, false, |m| {
            let face = *m.faces.get(f).ok_or_else(|| err("Invalid face id"))?;
            let edges = [(face.vertices[0], face.vertices[1]), (face.vertices[1], face.vertices[2]), (face.vertices[2], face.vertices[0])];
            let shares = |other: &Face| {
                let mine = [(other.vertices[0], other.vertices[1]), (other.vertices[1], other.vertices[2]), (other.vertices[2], other.vertices[0])];
                mine.iter().any(|&(a, b)| edges.iter().any(|&(c, d)| (a == c && b == d) || (a == d && b == c)))
            };
            let out: Vec<f64> = m.faces.live().filter(|(i, other)| *i != f && shares(other)).map(|(i, _)| make_id(FACE, i)).collect();
            Ok(values([ids_value(lua, out.into_iter())?]))
        })
    }));
    add("GetBoneByName", method(|_, engine, id, args| {
        let name = args.string(1).map(|s| s.to_string_lossy()).unwrap_or_default();
        let found = with_mesh(engine, id, false, |m| Ok(m.bones.live().find(|(_, b)| b.name == name).map(|(i, _)| i)))?;
        Ok(values([LuaValue::Number(found.map_or(0.0, |i| make_id(BONE, i)))]))
    }));
    add("RemoveBone", method(|_, engine, id, args| {
        let b = index_of(args, 1, BONE, "bone")?;
        with_mesh(engine, id, true, |m| {
            m.bones.remove(b);
            for bone in m.bones.0.iter_mut().flatten() {
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
            Ok(none())
        })
    }));
    add("ResetNormal", method(|_, engine, id, args| {
        let n = index_of(args, 1, NORMAL, "normal")?;
        with_mesh(engine, id, true, |m| {
            *m.normals.get_mut(n).ok_or_else(|| err("Invalid normal id"))? = None;
            Ok(none())
        })
    }));
    add("Clear", method(|_, engine, id, _| {
        with_mesh(engine, id, true, |m| {
            *m = Mesh::default();
            Ok(none())
        })
    }));

    add("Destroy", method(|_, engine, id, _| {
        let ext = extension(engine);
        ext.meshes.borrow_mut().remove(&id);
        ext.dirty.borrow_mut().remove(&id);
        Ok(none())
    }));

}
