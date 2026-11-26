//! The 3D world recorder: what a run puts in the Workspace (and does to Lighting), frame by frame,
//! as a scene file, a ror playback and an event log, so ror draws the game's own code running
//! (port and extension of an earlier Lune-based recorder). Engine core: it reads
//! the dom, reflection and the scheduler's clock, observes property and hierarchy hooks and
//! engine::world's events, and depends on nothing of the UI side (layout, text, export, input,
//! sessions). run.rs is its command line (`rur-native record`).
//!
//! - Everything that is ever in the Workspace gets one copy in the scene, as it was when it
//!   appeared (the CurrentCamera's children too, as roots: Roblox draws what a client parents to
//!   its camera, a game's client-only world folders among it); before that frame and after it leaves it is
//!   hidden (a part's or decal's Transparency 1, a Glass part out of the world, an emitter's /
//!   trail's / beam's / light's Enabled off and its particles cleared, a GUI's Enabled or a GUI
//!   object's Visible off), and shown again if it comes back.
//! - Property writes become `set` actions or, where a property changes frame after frame, tweens
//!   keyed every frame (keys a straight line reproduces dropped); parts that move get motion tracks
//!   (world CFrames), as do attachments whose CFrame is written (or whose parent is not a part).
//!   `ParticleEmitter:Emit(n)` and `Clear()` become actions at their frame; an instance reparented
//!   inside the world a `parent` action.
//! - Lighting's own properties play on ror's `Lighting`: every write of the run, its setup's at frame 0
//!   (the engine's values before the run stand in for the place's, ror's stage); the skies, atmospheres and post effects a
//!   run parents to Lighting are kept in the scene's `~Lighting` folder and parented into
//!   `Lighting` while they are there (so are the post effects it parents to the CurrentCamera: Studio
//!   draws them as it draws Lighting's). The place's own (ror's stage: `--lighting-from`'s) leave with
//!   the run's taking its own of their class out of Lighting and come back with them.
//! - The Terrain's Clouds (a Clouds in the Workspace's Terrain, the only place Roblox draws one) play on
//!   ror's `Workspace.Terrain.Clouds` as Lighting's own properties do: every write of the run, its setup's
//!   at frame 0 (the engine's clouds stand in for the place's, ror's stage). No Clouds is recorded as an instance.
//! - A Studio-only placeholder (the place's hierarchy snapshot gives it a class and a name and no values;
//!   project::materialize builds it when a script first lists its parent) is the place's own: one that
//!   draws (an effect, a part, a highlight, a sky) is never drawn, in the world or in Lighting, since the
//!   renderer's stage draws the place's look, and a folder or model is drawn only once something the run
//!   made is under it, as that appears. So a run that looks at more of the place records what one that
//!   does not records.
//! - The ScreenGuis the local player's PlayerGui shows are kept in the scene's `PlayerGui` (ror draws
//!   them over the finished image), hidden by Enabled off while they are not there.
//! - Prints, warnings, errors and sound plays go to the event log with their times; the camera's
//!   CFrame and field of view when they change to the camera track.
//! - A part's or decal's Transparency is recorded as drawn, with its LocalTransparencyModifier, in the
//!   scene file as in the playback.
//!
//! Time `t` of frame `n` is `n / fps`; frame 0 is the world right after the harness started.

pub mod encode;
pub mod prepare;
pub mod run;

#[cfg(test)]
mod tests;

use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

use glam::Affine3A;
use rbx_dom_weak::{InstanceBuilder, Ustr, WeakDom, types as dom, types::Variant, ustr};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use serde_json::{Map, Value as Json, json};

use super::{
    ClassId, Engine, InstanceId, K, NameId, State,
    value::Value,
    world::{self, Emission, WorldEvent, frame},
};
use crate::dom::flags;
use encode::{cframe_json, numbers, round, to_json};

/// How a tracked property plays back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Track {
    /// A part's CFrame: a motion track
    Motion,
    /// An attachment's CFrame (a bone's Transform): where it is drawn, sampled every frame
    Attachment,
    /// Transparency as drawn (Transparency with LocalTransparencyModifier)
    Effective,
    /// Enabled of what ror switches: `enabled` actions
    Enabled,
    /// An emitter's TimeScale: `timeScale` actions
    TimeScale,
    /// Anything else: `set` actions, or keyed tweens where it changes every frame
    Plain,
}

/// The visual properties per class (and what inherits it).
const VISUAL: &[(&str, &[(&str, Track)])] = {
    use Track::*;
    &[
        ("BasePart", &[("CFrame", Motion), ("Size", Plain), ("Transparency", Effective), ("LocalTransparencyModifier", Effective), ("Color", Plain), ("Material", Plain), ("MaterialVariant", Plain), ("Reflectance", Plain)]),
        ("MeshPart", &[("TextureID", Plain), ("TextureContent", Plain), ("MeshContent", Plain), ("InitialSize", Plain)]),
        ("SurfaceAppearance", &[
            ("Color", Plain), ("EmissiveStrength", Plain), ("EmissiveTint", Plain), ("ColorMapContent", Plain), ("NormalMapContent", Plain),
            ("RoughnessMapContent", Plain), ("MetalnessMapContent", Plain), ("EmissiveMaskContent", Plain),
        ]),
        ("SpecialMesh", &[("Scale", Plain), ("Offset", Plain), ("VertexColor", Plain), ("TextureId", Plain), ("TextureContent", Plain)]),
        ("Decal", &[("Transparency", Effective), ("LocalTransparencyModifier", Effective), ("Color3", Plain), ("Texture", Plain), ("TextureContent", Plain), ("ZIndex", Plain), ("UVOffset", Plain), ("UVScale", Plain)]),
        ("Texture", &[("OffsetStudsU", Plain), ("OffsetStudsV", Plain), ("StudsPerTileU", Plain), ("StudsPerTileV", Plain)]),
        ("ParticleEmitter", &[
            ("Enabled", Enabled), ("TimeScale", TimeScale), ("Rate", Plain), ("Color", Plain), ("Size", Plain), ("Transparency", Plain),
            ("LightEmission", Plain), ("LightInfluence", Plain), ("Brightness", Plain), ("Speed", Plain), ("Lifetime", Plain),
            ("Acceleration", Plain), ("Drag", Plain), ("ZOffset", Plain), ("SpreadAngle", Plain), ("Texture", Plain), ("Rotation", Plain),
            ("RotSpeed", Plain), ("Squash", Plain), ("EmissionDirection", Plain), ("Orientation", Plain), ("LockedToPart", Plain),
            ("VelocityInheritance", Plain), ("FlipbookFramerate", Plain), ("FlipbookLayout", Plain), ("FlipbookMode", Plain),
            ("Shape", Plain), ("ShapeInOut", Plain), ("ShapeStyle", Plain), ("ShapePartial", Plain),
        ]),
        ("Beam", &[
            ("Enabled", Enabled), ("Color", Plain), ("Transparency", Plain), ("Width0", Plain), ("Width1", Plain), ("CurveSize0", Plain),
            ("CurveSize1", Plain), ("Texture", Plain), ("TextureLength", Plain), ("TextureMode", Plain), ("TextureSpeed", Plain),
            ("LightEmission", Plain), ("LightInfluence", Plain), ("Brightness", Plain), ("ZOffset", Plain), ("Segments", Plain), ("FaceCamera", Plain),
        ]),
        ("Trail", &[
            ("Enabled", Enabled), ("Color", Plain), ("Transparency", Plain), ("WidthScale", Plain), ("Lifetime", Plain), ("MinLength", Plain),
            ("MaxLength", Plain), ("Texture", Plain), ("TextureLength", Plain), ("TextureMode", Plain), ("LightEmission", Plain),
            ("LightInfluence", Plain), ("Brightness", Plain), ("FaceCamera", Plain),
        ]),
        ("Light", &[("Enabled", Enabled), ("Brightness", Plain), ("Range", Plain), ("Color", Plain), ("Shadows", Plain)]),
        ("SpotLight", &[("Angle", Plain), ("Face", Plain)]),
        ("SurfaceLight", &[("Angle", Plain), ("Face", Plain)]),
        ("Highlight", &[("Enabled", Enabled), ("FillColor", Plain), ("FillTransparency", Plain), ("OutlineColor", Plain), ("OutlineTransparency", Plain), ("DepthMode", Plain)]),
        ("Humanoid", &[("RigType", Plain), ("DisplayDistanceType", Plain), ("NameDisplayDistance", Plain)]),
        ("Attachment", &[("CFrame", Attachment)]),
        ("Bone", &[("Transform", Attachment)]),
        ("Fire", &[("Enabled", Enabled), ("Size", Plain), ("Heat", Plain), ("Color", Plain), ("SecondaryColor", Plain), ("TimeScale", TimeScale)]),
        ("Smoke", &[("Enabled", Enabled), ("Size", Plain), ("Opacity", Plain), ("RiseVelocity", Plain), ("Color", Plain), ("TimeScale", TimeScale)]),
        ("Sparkles", &[("Enabled", Enabled), ("SparkleColor", Plain), ("TimeScale", TimeScale)]),
        // GUIs drawing on the world (ground decals and telegraphs on a part's face, billboards)
        ("LayerCollector", &[("Enabled", Plain), ("ZIndexBehavior", Plain)]),
        // GUIs drawing on the screen, over the finished image
        ("ScreenGui", &[("DisplayOrder", Plain), ("IgnoreGuiInset", Plain), ("ScreenInsets", Plain)]),
        ("SurfaceGuiBase", &[("Face", Plain)]),
        ("SurfaceGui", &[("Brightness", Plain), ("ZOffset", Plain), ("LightInfluence", Plain), ("AlwaysOnTop", Plain), ("PixelsPerStud", Plain), ("CanvasSize", Plain), ("SizingMode", Plain)]),
        ("BillboardGui", &[("Brightness", Plain), ("LightInfluence", Plain), ("AlwaysOnTop", Plain), ("Size", Plain), ("StudsOffset", Plain), ("StudsOffsetWorldSpace", Plain), ("ExtentsOffset", Plain)]),
        ("GuiObject", &[
            ("Visible", Plain), ("Size", Plain), ("Position", Plain), ("AnchorPoint", Plain), ("Rotation", Plain),
            ("BackgroundTransparency", Plain), ("BackgroundColor3", Plain), ("ZIndex", Plain), ("ClipsDescendants", Plain),
        ]),
        ("ImageLabel", &[("Image", Plain), ("ImageColor3", Plain), ("ImageTransparency", Plain), ("ImageRectOffset", Plain), ("ImageRectSize", Plain), ("ScaleType", Plain), ("SliceCenter", Plain), ("SliceScale", Plain), ("TileSize", Plain)]),
        ("ImageButton", &[("Image", Plain), ("ImageColor3", Plain), ("ImageTransparency", Plain), ("ImageRectOffset", Plain), ("ImageRectSize", Plain), ("ScaleType", Plain), ("SliceCenter", Plain), ("SliceScale", Plain), ("TileSize", Plain)]),
        ("TextLabel", &[
            ("Text", Plain), ("TextColor3", Plain), ("TextTransparency", Plain), ("TextSize", Plain), ("TextStrokeColor3", Plain), ("TextStrokeTransparency", Plain),
            ("MaxVisibleGraphemes", Plain),
        ]),
        ("UIStroke", &[("Enabled", Plain), ("Color", Plain), ("Transparency", Plain), ("Thickness", Plain)]),
        ("UIShadow", &[("Color", Plain), ("Transparency", Plain)]),
        ("UIGradient", &[("Enabled", Plain), ("Color", Plain), ("Transparency", Plain), ("Rotation", Plain), ("Offset", Plain)]),
        ("UIScale", &[("Scale", Plain)]),
        ("ViewportFrame", &[("ImageTransparency", Plain), ("ImageColor3", Plain), ("Ambient", Plain), ("LightColor", Plain), ("LightDirection", Plain)]),
        ("Lighting", &[
            ("ExposureCompensation", Plain), ("Brightness", Plain), ("Ambient", Plain), ("OutdoorAmbient", Plain), ("ClockTime", Plain),
            ("FogColor", Plain), ("FogStart", Plain), ("FogEnd", Plain), ("ColorShift_Top", Plain), ("ColorShift_Bottom", Plain),
            ("EnvironmentDiffuseScale", Plain), ("EnvironmentSpecularScale", Plain), ("GeographicLatitude", Plain), ("GlobalShadows", Plain),
            ("ShadowSoftness", Plain),
        ]),
        ("PostEffect", &[("Enabled", Plain)]),
        ("ColorCorrectionEffect", &[("Brightness", Plain), ("Contrast", Plain), ("Saturation", Plain), ("TintColor", Plain)]),
        ("BloomEffect", &[("Intensity", Plain), ("Size", Plain), ("Threshold", Plain)]),
        ("BlurEffect", &[("Size", Plain)]),
        ("SunRaysEffect", &[("Intensity", Plain), ("Spread", Plain)]),
        ("DepthOfFieldEffect", &[("FarIntensity", Plain), ("FocusDistance", Plain), ("InFocusRadius", Plain), ("NearIntensity", Plain)]),
        ("Atmosphere", &[("Density", Plain), ("Offset", Plain), ("Color", Plain), ("Decay", Plain), ("Glare", Plain), ("Haze", Plain)]),
        ("Clouds", &[("Color", Plain), ("Cover", Plain), ("Density", Plain), ("Enabled", Plain)]),
    ]
};

/// What ror switches on and off (its `switchable` classes).
const SWITCHABLE: &[&str] = &["ParticleEmitter", "Trail", "Beam", "PointLight", "SpotLight", "SurfaceLight", "Highlight", "Fire", "Smoke", "Sparkles"];
/// What `Clear()` and a hide empty.
const CLEARABLE: &[&str] = &["ParticleEmitter", "Trail"];
/// What a child of Lighting adds to the look.
const LIGHTING_CHILDREN: &[&str] = &["Sky", "Atmosphere", "ColorCorrectionEffect", "BloomEffect", "BlurEffect", "SunRaysEffect", "DepthOfFieldEffect", "ColorGradingEffect"];
/// What draws nothing itself but changes how its parent draws (a part's shape, a GUI's corners or layout, a
/// character's clothing, a model's Humanoid), with no property that hides it: out of the world while it is gone.
const MODIFIERS: &[&str] = &["DataModelMesh", "UIComponent", "CharacterAppearance", "Humanoid"];
/// Never recorded, with everything under them: code, sounds, values and the like draw nothing (and
/// the Terrain's Clouds play on [`CLOUDS`]).
const SKIPPED: &[&str] = &[
    "LuaSourceContainer", "Sound", "ValueBase", "BindableEvent", "BindableFunction", "RemoteEvent", "UnreliableRemoteEvent",
    "RemoteFunction", "Animation", "AnimationController", "Animator", "Configuration", "SoundEffect", "SoundGroup",
    "Terrain", "Camera", "Clouds",
];
/// Recorded without anything under them. A Humanoid makes its model a character: Studio dresses the
/// limbs of its RigType in the model's BodyColors, Shirt, Pants and ShirtGraphic, and shows its name
/// over the Head unless its DisplayDistanceType is None (Roblox's documentation), so the renderer reads
/// both. What it holds (its Animator, HumanoidDescription and scale values) draws nothing.
const CLOSED: &[&str] = &["Humanoid"];

/// How the world's walk takes an instance of a class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// Not recorded, nor anything under it ([`SKIPPED`])
    Skip,
    /// Recorded, but nothing under it ([`CLOSED`])
    Closed,
    /// Recorded, and what is under it walked
    Open,
}
/// The scene folder holding what the run parented to Lighting.
const LIGHTING_FOLDER: &str = "~Lighting";
/// The playback's target for the Terrain's Clouds: their path in a place, where ror keeps the clouds it
/// draws (its stage's).
const CLOUDS: &str = "Workspace.Terrain.Clouds";
/// The scene's PlayerGui, holding the ScreenGuis the local player's PlayerGui shows (ror draws them on
/// the screen; their paths read as Roblox names them).
const PLAYER_GUI: &str = "PlayerGui";

/// How an instance is hidden and shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visibility {
    /// Transparency 1 (parts, decals and textures)
    Transparency,
    /// `enabled` off (and particles / trail segments cleared)
    Switch { clear: bool },
    /// Enabled false (LayerCollectors, UIStroke, UIGradient)
    Enabled,
    /// Visible false (GuiObjects)
    Visible,
    /// Parented out of Lighting (skies, atmospheres, post effects)
    Lighting,
    /// Parented out of the world (SurfaceAppearances: a part shows the one it holds; meshes, UI
    /// components, character appearances and Humanoids: [`MODIFIERS`]; Glass parts, which still draw at
    /// Transparency 1: [`is_glass`])
    Detach,
    /// Never hidden: draws nothing and changes nothing its descendants do not hide themselves
    /// (folders, models, attachments, joints...)
    None,
}

struct Record {
    id: InstanceId,
    class: ClassId,
    class_name: String,
    /// The copy's name in the scene (unique among its siblings)
    name: String,
    path: String,
    parent: Option<usize>,
    appear: u32,
    visibility: Visibility,
    /// The copy as it appeared: (property, value) as rbx-dom stores them, references separately
    properties: Vec<(Ustr, Variant)>,
    references: Vec<(Ustr, InstanceId)>,
    /// Encoded values: as in the file, and as last recorded
    file: FxHashMap<NameId, Json>,
    last: FxHashMap<NameId, Json>,
    /// Changes per property: how it plays back, (frame, encoded value)
    changes: Vec<(NameId, Track, Vec<(u32, Json)>)>,
    /// (frame, count, where it emitted from at the call)
    emits: Vec<(u32, f64, Option<Emission>)>,
    clears: Vec<u32>,
    /// Its instance references the file holds (a beam's attachments, a model's PrimaryPart, a GUI's
    /// Adornee), as last recorded, and their changes after it appeared: (frame, property, the instance
    /// it then names), set actions naming that instance's path or none
    refs: Vec<(NameId, Option<InstanceId>)>,
    ref_changes: Vec<(u32, NameId, Option<InstanceId>)>,
    /// World CFrame at appearance (parts, attachments) and over time
    rest: Option<Affine3A>,
    motion: Vec<(u32, Affine3A)>,
    /// The frame physics first moved it from (world::gravity): its LockedToPart particles stay behind;
    /// not written when something else moved it too after that (a script's moves carry them)
    physics_from: Option<u32>,
    physics_steered: bool,
    /// Where it was moved inside the world: (frame, new parent record; None for Lighting's folder)
    parents: Vec<(u32, Option<usize>, String)>,
    /// The parent instance it last had, and that parent's path (where a detached copy comes back to)
    parent_id: Option<InstanceId>,
    home: String,
    /// A child of Lighting (kept in ~Lighting, parented into Lighting while there)
    lighting_child: bool,
    /// A ScreenGui of the PlayerGui (kept in the scene's PlayerGui)
    screen: bool,
    /// An Attachment or Bone: placed by its parent's motion, never given a still track
    attachment: bool,
}

/// A sound that started or stopped.
#[derive(Clone, Debug)]
struct SoundEvent {
    frame: u32,
    time: f64,
    played: bool,
    local: bool,
    path: String,
    sound_id: String,
    volume: f64,
    speed: f64,
}

#[derive(Default)]
struct SiblingNames {
    used: FxHashSet<String>,
    /// Next suffix to try for a base name that has already collided (starts at 2)
    next: FxHashMap<String, usize>,
}

struct SerializedProp {
    dom_name: Ustr,
    stored: NameId,
    sibling: Option<NameId>,
    data_type: &'static rbx_reflection::DataType<'static>,
    is_ref: bool,
}

struct ExtraProp {
    dom_name: Ustr,
    name: NameId,
    sibling: Option<NameId>,
    engine_type: crate::reflection::EngineType,
}

struct TrackedProp {
    name: NameId,
    track: Track,
    sibling: Option<NameId>,
    is_content_id: bool,
    file_key: NameId,
}

struct SnapshotPlan {
    serialized: Vec<SerializedProp>,
    extra: Vec<ExtraProp>,
    known_attrs: bool,
    visibility: Visibility,
    is_part: bool,
    is_attachment: bool,
    is_bone: bool,
    is_viewport: bool,
    is_explosion: bool,
    tracked: Vec<TrackedProp>,
}

#[derive(Default)]
struct Inner {
    frame: u32,
    fps: f64,
    /// The moments of frames 1.. when they are not every 1 / fps (`--frame-times`)
    times: Option<Vec<f64>>,
    records: Vec<Record>,
    by_id: FxHashMap<InstanceId, usize>,
    /// Per record: in the world at the last walk (what was drawn then)
    present: Vec<bool>,
    /// The records in the world at the last walk, in walk order
    present_list: Vec<usize>,
    /// Per record: the last walk that found it in the world ([`Inner::walks`])
    seen: Vec<u32>,
    walks: u32,
    /// The last walk's instances, kept for its buffer
    walk_buffer: Vec<InstanceId>,
    /// Property writes since the last sample, in order (deduplicated)
    dirty: Vec<(InstanceId, NameId)>,
    dirty_set: FxHashSet<(InstanceId, NameId)>,
    emits: Vec<(InstanceId, f64, Option<Emission>)>,
    clears: Vec<InstanceId>,
    sounds: Vec<(WorldEvent, f64)>,
    sound_log: Vec<SoundEvent>,
    /// Attachment records sampled every frame
    sampled: FxHashSet<usize>,
    camera: Vec<(u32, Affine3A, f64)>,
    /// Lighting's own properties (target "Lighting")
    lighting: Option<Record>,
    /// The Terrain's Clouds' properties (target [`CLOUDS`]) and what finds those clouds
    clouds: Option<Record>,
    terrain_clouds: Option<TerrainClouds>,
    warnings: Vec<String>,
    unsupported: FxHashSet<String>,
    /// Sibling names already used in the scene, by parent record (None: the file's roots)
    names: FxHashMap<Option<usize>, SiblingNames>,
    /// Precomputed per-class snapshot & tracking plans
    plans: FxHashMap<ClassId, Arc<SnapshotPlan>>,
    tracked: FxHashMap<ClassId, Arc<[(NameId, Track)]>>,
    /// The instance references the scene file holds, per class
    reference_names: FxHashMap<ClassId, Arc<[NameId]>>,
    /// Property names by id, for the playback
    property_names: FxHashMap<NameId, String>,
    /// Lighting's children before the run (the place's own sky and effects: the renderer's stage
    /// brings its own; and a place's Studio-only ones hold no real values)
    ignored: FxHashSet<InstanceId>,
    /// Lighting, as the run started
    lighting_id: Option<InstanceId>,
    /// Moves out of and into Lighting since the last sample: (instance, class, into Lighting)
    lighting_moves: Vec<(InstanceId, ClassId, bool)>,
    /// The place's own skies, atmospheres and post effects of a class taken out of Lighting and put
    /// back, which ror's stage stands for: (frame, class name, in Lighting)
    stage: Vec<(u32, String, bool)>,
    /// The dom's structure version and the CurrentCamera when the world was last walked: while both
    /// are the same nothing appeared, left or moved, and the walk is skipped
    walked: Option<(u64, InstanceId)>,
    /// Walk every frame (a test comparing the two)
    walk_always: bool,
    /// The work clock's total (seconds, desktop prices) at each sampled frame, for a timing check's comparison of the work a
    /// frame did ([`Recorder::track_work`]); None when not asked for
    frame_work: Option<Vec<f64>>,
    /// An Adornee was written since the last walk: a GUI of the PlayerGui may draw on something else
    /// now, or on nothing, with nothing parented (the walk finds what [`Inner::adorned`] holds)
    adornee_written: bool,
    /// GUIs of the local player's PlayerGui that draw on something in the world, by what they are
    /// adorned to: recorded under it, where they draw
    adorned: FxHashMap<InstanceId, InstanceId>,
    /// ScreenGuis of the local player's PlayerGui (in it or a Folder of it), by that PlayerGui: drawn on
    /// the screen, recorded as the scene PlayerGui's children
    screens: FxHashMap<InstanceId, InstanceId>,
    /// Per class: how the walk takes it (SKIPPED, CLOSED)
    walks_by_class: Vec<Option<Walk>>,
    /// The ViewportFrame records each camera is the CurrentCamera of (their CameraCFrame and
    /// CameraFieldOfView follow it; a window's is skewed every frame to look through its face)
    viewports: FxHashMap<InstanceId, Vec<usize>>,
}

/// Records the Workspace of one engine (create it before the run builds anything, then call
/// [`Recorder::sample`] after the harness starts and after every frame).
pub struct Recorder {
    inner: Rc<RefCell<Inner>>,
}

impl Recorder {
    /// Installs the recorder's hooks on `engine` (property writes, world events).
    pub fn install(engine: &Rc<Engine>, fps: f64) -> Recorder {
        let mut state = Inner { fps, ..Default::default() };
        // (a world root named PlayerGui is renamed rather than mixed with the scene's PlayerGui)
        state.names.entry(None).or_default().used.insert(PLAYER_GUI.to_string());
        for property in ["Transparency", "Enabled", "Visible", "CameraCFrame", "CameraFieldOfView"] {
            if let Some(name) = engine.reflection.name_id(property) {
                state.property_names.insert(name, property.to_string());
            }
        }
        // (Lighting's record is there before the harness runs: what it writes then is a change at frame 0)
        if let Some(lighting) = super::services::find_service(engine, engine.world().game, "Lighting") {
            let engine_state = engine.state.borrow();
            state.ignored.extend(engine_state.dom.children_of(lighting).iter().copied());
            state.lighting_id = Some(lighting);
            state.lighting = Some(stage_record(lighting, engine_state.dom.at(lighting).class, "Lighting", "Lighting"));
        }
        // (so is the Terrain's Clouds', whenever the place's clouds are built or the run makes its own)
        let terrain_clouds = TerrainClouds::of(engine);
        if let Some(found) = terrain_clouds {
            state.clouds = Some(stage_record(found.workspace, found.clouds, "Clouds", CLOUDS));
            state.terrain_clouds = terrain_clouds;
        }
        let lighting_id = state.lighting_id;
        let inner = Rc::new(RefCell::new(state));
        let mut hooks = engine.hooks.borrow_mut();
        let moves = inner.clone();
        hooks.hierarchy_changed.push(Box::new(move |state: &mut State, id, old, new| {
            let mut inner = moves.borrow_mut();
            let Some(lighting) = inner.lighting_id else { return };
            let into = new == Some(lighting);
            if (old == Some(lighting)) != into {
                let class = state.dom.at(id).class;
                inner.lighting_moves.push((id, class, into));
            }
        }));
        let writes = inner.clone();
        let adornee = engine.reflection.name_id("Adornee");
        hooks.property_changed.push(Box::new(move |_state: &mut State, id, name| {
            let mut inner = writes.borrow_mut();
            if Some(name) == adornee {
                inner.adornee_written = true;
            }
            if inner.dirty_set.insert((id, name)) {
                inner.dirty.push((id, name));
            }
        }));
        // a write to Lighting or the Terrain's Clouds is the run's even when it stores the value the engine
        // held: that value only stands in for the place's, which the renderer's stage draws
        if lighting_id.is_some() || terrain_clouds.is_some() {
            let kept = inner.clone();
            hooks.property_unchanged.push(Box::new(move |state: &mut State, id, name| {
                if Some(id) == lighting_id || terrain_clouds.is_some_and(|clouds| clouds.holds(state, id)) {
                    let mut inner = kept.borrow_mut();
                    if inner.dirty_set.insert((id, name)) {
                        inner.dirty.push((id, name));
                    }
                }
            }));
        }
        let events = inner.clone();
        hooks.world_events.push(Box::new(move |state: &mut State, event: &WorldEvent| {
            let mut inner = events.borrow_mut();
            match event {
                WorldEvent::Emit { emitter, count, from } => inner.emits.push((*emitter, *count, *from)),
                WorldEvent::Clear { instance } => inner.clears.push(*instance),
                WorldEvent::SoundPlayed { .. } | WorldEvent::SoundStopped { .. } => {
                    let now = state.sched.now;
                    inner.sounds.push((event.clone(), now));
                }
            }
        }));
        Recorder { inner }
    }

    /// The frame rate of the recording, before its first frame is sampled (a prepared run's job's own:
    /// record/prepare.rs).
    pub fn set_fps(&self, fps: f64) {
        self.inner.borrow_mut().fps = fps;
    }

    /// The moments of the frames after frame 0 when the run steps `--frame-times` (else one every
    /// 1 / fps), before its first frame is sampled.
    pub fn set_frame_times(&self, args: &run::RecordArgs) {
        self.inner.borrow_mut().times = args.frame_times.is_some().then(|| args.frame_schedule());
    }

    /// Walks the world every frame, as the recorder did before it skipped frames where nothing was
    /// parented (a test's reference).
    #[cfg(test)]
    pub(crate) fn walk_every_frame(&self) {
        self.inner.borrow_mut().walk_always = true;
    }

    /// Records the work each frame does (the summary's `frame_work_ms`): a timing check compares it between its probes, as a
    /// script that budgets its work on os.clock does a different amount of it a frame when the clock moves.
    pub fn track_work(&self) {
        self.inner.borrow_mut().frame_work = Some(Vec::new());
    }

    /// Records the world as it is now as frame `frame`.
    pub fn sample(&self, engine: &Rc<Engine>, frame: u32) {
        // (EditableMesh snapshots of the meshes changed since the last sample)
        crate::meshes::flush(engine);
        let mut inner = self.inner.borrow_mut();
        inner.frame = frame;
        if inner.frame_work.is_some() {
            let total = engine.state.borrow().sched.work_total();
            if let Some(work) = inner.frame_work.as_mut() {
                work.push(total);
            }
        }
        sample(&mut inner, engine);
    }

    /// Writes `scene.rbxm`, `playback.json`, `events.json`, `camera.json` and returns the summary
    /// (also written as `summary.json`). Output lines the engine spooled as events.json holds them
    /// (`ree record`: [`event_line`]) are copied into its `output` array.
    pub fn write(&self, engine: &Rc<Engine>, out: &Path, extra: Map<String, Json>) -> Result<Json, String> {
        let inner = self.inner.borrow();
        std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let inner_ref: &Inner = &inner;
        let profiling = crate::timing::enabled();
        let (scene_res, (playback, pb_secs)) = rayon::join(
            || -> Result<(Vec<u8>, f64), String> {
                let t0 = profiling.then(std::time::Instant::now);
                let bytes = scene(inner_ref)?;
                Ok((bytes, t0.map_or(0.0, |t| t.elapsed().as_secs_f64())))
            },
            || {
                let t0 = profiling.then(std::time::Instant::now);
                let pb = playback(inner_ref);
                (pb, t0.map_or(0.0, |t| t.elapsed().as_secs_f64()))
            },
        );
        let (scene_bytes, scene_secs) = scene_res?;
        if profiling {
            crate::timing::add("write: scene", scene_secs);
        }
        std::fs::write(out.join("scene.rbxm"), scene_bytes).map_err(|e| format!("scene.rbxm: {e}"))?;
        if profiling {
            crate::timing::add("write: playback", pb_secs);
        }
        crate::timing::time("write: playback.json", || write_playback(&out.join("playback.json"), &playback))?;
        let spooled = engine.state.borrow_mut().output.take_spool();
        let events = events(&inner, engine);
        let counts = {
            let state = engine.state.borrow();
            let count = |kind: &str| state.output.entries.iter().filter(|entry| entry.kind == kind).count();
            (count("error"), count("warn"))
        };
        let counts = match spooled {
            Some(spool) => {
                // (none are kept beside a spool: `events` holds only the sounds)
                let counts = (counts.0 + spool.errors(), counts.1 + spool.warnings());
                let lines = spool.lines();
                write_events(&out.join("events.json"), &events, (lines > 0).then_some(spool.finish()?))?;
                counts
            }
            None => {
                write_json(&out.join("events.json"), &events)?;
                counts
            }
        };
        let camera: Vec<Json> = inner.camera.iter().map(|(frame, cframe, fov)| json!({ "at": time_of(&inner, *frame), "cframe": cframe_json(cframe), "fov": round(*fov) })).collect();
        write_json(&out.join("camera.json"), &Json::Array(camera))?;
        let summary = summary(&inner, counts, &playback, extra);
        write_json(&out.join("summary.json"), &summary)?;
        Ok(summary)
    }
}

/// `value` as pretty JSON in `path`, written as it is serialized (not built as one string first).
fn write_json(path: &Path, value: &Json) -> Result<(), String> {
    use std::io::Write;
    let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut file = std::io::BufWriter::with_capacity(1 << 16, std::fs::File::create(path).map_err(|e| fail(&e))?);
    serde_json::to_writer_pretty(&mut file, value).map_err(|e| fail(&e))?;
    file.flush().map_err(|e| fail(&e))
}

/// What goes into one of a top-level object's arrays left empty in its JSON: the elements' bytes, with
/// `,\n` between them ([`element`]).
enum Elements<'a> {
    File(std::fs::File),
    Bytes(Box<dyn Iterator<Item = &'a [u8]> + 'a>),
}

/// Writes `top` as JSON to `path` (`compact` for `playback.json`, pretty for `events.json`) with the
/// arrays of `arrays` (top-level keys, each holding `[]` in `top`) filled with their elements: the
/// exact bytes of `serde_json::to_vec` (`compact`) or `serde_json::to_vec_pretty` (`!compact`).
fn write_filled(path: &Path, top: &Json, arrays: Vec<(&str, Elements)>, compact: bool) -> Result<(), String> {
    use std::io::Write;
    let text = if compact { serde_json::to_string(top) } else { serde_json::to_string_pretty(top) }.map_err(|e| e.to_string())?;
    // (keys and strings are escaped: `"key":[]` / `"key": []` with an unescaped quote is a key; each top-level key is
    // the first of its kind, the object's own coming before any value's)
    let mut places = Vec::new();
    for (key, elements) in arrays {
        let needle = if compact { format!("\"{key}\":[]") } else { format!("\"{key}\": []") };
        let at = text.find(&needle).ok_or_else(|| format!("{}: no {key} array", path.display()))? + needle.len() - 2;
        places.push((at, elements));
    }
    crate::sort::by_key(&mut places, |(at, _)| *at);
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = std::io::BufWriter::with_capacity(1 << 16, std::fs::File::create(path).map_err(fail)?);
    let mut written = 0;
    for (at, elements) in places {
        file.write_all(text[written..at].as_bytes()).map_err(fail)?;
        written = at + 2;
        match elements {
            Elements::File(mut lines) => {
                file.write_all(b"[\n").map_err(fail)?;
                std::io::copy(&mut lines, &mut file).map_err(fail)?;
                file.write_all(b"\n  ]").map_err(fail)?;
            }
            Elements::Bytes(mut items) => match items.next() {
                None => file.write_all(b"[]").map_err(fail)?,
                Some(first) => {
                    file.write_all(if compact { b"[" } else { b"[\n" }).map_err(fail)?;
                    file.write_all(first).map_err(fail)?;
                    let sep: &[u8] = if compact { b"," } else { b",\n" };
                    for item in items {
                        file.write_all(sep).map_err(fail)?;
                        file.write_all(item).map_err(fail)?;
                    }
                    file.write_all(if compact { b"]" } else { b"\n  ]" }).map_err(fail)?;
                }
            },
        }
    }
    file.write_all(text[written..].as_bytes()).map_err(fail)?;
    file.flush().map_err(fail)
}

/// events.json with the spooled output lines (`lines`, [`event_line`]s with `,\n` between) in the
/// `output` array `events` holds empty: the bytes [`write_json`] writes of it holding them.
fn write_events(path: &Path, events: &Json, lines: Option<std::fs::File>) -> Result<(), String> {
    match lines {
        Some(lines) => write_filled(path, events, vec![("output", Elements::File(lines))], false),
        None => write_json(path, events),
    }
}

/// Writes pretty-printed JSON with a 4-space prefix on every line directly into `out`,
/// matching `serde_json::to_writer_pretty` followed by prefixing every line with four spaces.
struct Indent4Writer<'a>(&'a mut Vec<u8>);

impl std::io::Write for Indent4Writer<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf == b"\n" {
            self.0.extend_from_slice(b"\n    ");
            return Ok(1);
        }
        if !buf.contains(&b'\n') {
            self.0.extend_from_slice(buf);
            return Ok(buf.len());
        }
        let mut start = 0;
        for (i, &b) in buf.iter().enumerate() {
            if b == b'\n' {
                self.0.extend_from_slice(&buf[start..i]);
                self.0.extend_from_slice(b"\n    ");
                start = i + 1;
            }
        }
        self.0.extend_from_slice(&buf[start..]);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn indented_serialize<T: Serialize>(value: &T, out: &mut Vec<u8>) {
    let start = out.len();
    out.extend_from_slice(b"    ");
    let mut serializer = serde_json::Serializer::pretty(Indent4Writer(out));
    if value.serialize(&mut serializer).is_err() {
        out.truncate(start);
        out.extend_from_slice(b"    ");
    }
}

/// `value` as pretty JSON two levels in (an element of a top-level object's array): each line 4
/// spaces in, as the object's pretty JSON holds it.
fn indented(value: &Json, out: &mut Vec<u8>) {
    indented_serialize(value, out);
}

/// Compact JSON of one playback array element (no pretty-print indentation or newlines).
fn serialize_element<T: Serialize>(value: &T) -> Box<[u8]> {
    serde_json::to_vec(value).unwrap_or_default().into_boxed_slice()
}

/// An output line as events.json holds it in its `output` array (pretty, 4 spaces in).
pub(crate) fn event_line(entry: &crate::output::Entry, out: &mut Vec<u8>) {
    indented(&event_json(entry), out);
}

/// playback.json: its arrays as their compact elements' bytes ([`serialize_element`]; a recording of a busy scene made
/// a JSON value of hundreds of thousands of actions and a string of all of it, ~3 GB for one effect scene's, and
/// pretty-printing alone more than doubled playback.json's size with per-number indentation inside 12-float CFrames).
struct Playback {
    /// (time, bytes) of each action, in time order
    actions: Vec<(f64, Box<[u8]>)>,
    tweens: Vec<Box<[u8]>>,
    motion: Vec<Box<[u8]>>,
    frames: Option<Json>,
}

fn write_playback(path: &Path, playback: &Playback) -> Result<(), String> {
    // (`play`: a recording of a running game, which ror draws as one: a SurfaceAppearance map that
    // cannot load is left out rather than hiding the part)
    let mut top = json!({ "actions": [], "tweens": [], "motion": [], "play": true });
    if let Some(frames) = &playback.frames {
        top["frames"] = frames.clone();
    }
    write_filled(
        path,
        &top,
        vec![
            ("actions", Elements::Bytes(Box::new(playback.actions.iter().map(|(_, bytes)| &bytes[..])))),
            ("tweens", Elements::Bytes(Box::new(playback.tweens.iter().map(|bytes| &bytes[..])))),
            ("motion", Elements::Bytes(Box::new(playback.motion.iter().map(|bytes| &bytes[..])))),
        ],
        true,
    )
}

fn event_json(entry: &crate::output::Entry) -> Json {
    let mut item = json!({ "time": round(entry.time), "kind": entry.kind, "message": entry.message });
    if let Some(trace) = &entry.trace {
        item["trace"] = json!(trace);
    }
    // (a loopback run's line names its side; no other run's has one)
    if let Some(side) = entry.side {
        item["side"] = json!(side);
    }
    item
}

fn time_of(inner: &Inner, frame: u32) -> f64 {
    if let Some(&at) = inner.times.as_ref().and_then(|times| times.get((frame as usize).wrapping_sub(1))) {
        return round(at);
    }
    round(frame as f64 / inner.fps.max(1.0))
}

fn class_name(engine: &Engine, class: ClassId) -> &str {
    &engine.reflection.class(class).name
}

fn is_a(engine: &Engine, class: ClassId, name: &str) -> bool {
    engine.reflection.is_a(class, name)
}

/// The tracked properties of a class.
fn tracked(inner: &mut Inner, engine: &Engine, class: ClassId) -> Arc<[(NameId, Track)]> {
    if let Some(found) = inner.tracked.get(&class) {
        return found.clone();
    }
    let mut list: Vec<(NameId, Track)> = Vec::new();
    for (owner, properties) in VISUAL {
        if !is_a(engine, class, owner) {
            continue;
        }
        for (property, track) in properties.iter() {
            if let Some(name) = engine.reflection.name_id(property) {
                inner.property_names.insert(name, property.to_string());
                let engine_only = crate::reflection::ENGINE_ONLY.contains(property);
                if engine.reflection.class_property(class, name).is_some() || *track == Track::Attachment || engine_only {
                    list.retain(|(n, _)| *n != name);
                    list.push((name, *track));
                }
            }
        }
    }
    let list: Arc<[(NameId, Track)]> = list.into();
    inner.tracked.insert(class, list.clone());
    list
}

/// The instance references the scene file holds for a class: its serialized Ref properties (a beam's
/// and a trail's attachments, a model's PrimaryPart, a GUI's Adornee, a constraint's parts).
fn reference_names(inner: &mut Inner, engine: &Engine, class: ClassId) -> Arc<[NameId]> {
    if let Some(found) = inner.reference_names.get(&class) {
        return found.clone();
    }
    let mut names: Vec<NameId> = Vec::new();
    for (property, data_type) in encode::serialized_properties(class_name(engine, class)) {
        if !matches!(data_type, rbx_reflection::DataType::Value(dom::VariantType::Ref)) || matches!(&*property, "Parent") {
            continue;
        }
        if let Some(name) = engine.reflection.name_id(&property) {
            inner.property_names.insert(name, property.to_string());
            names.push(name);
        }
    }
    let names: Arc<[NameId]> = names.into();
    inner.reference_names.insert(class, names.clone());
    names
}

/// Records an instance reference's value at `frame` when it changed (the latest in a frame wins).
fn change_reference(record: &mut Record, slot: usize, now: Option<InstanceId>, frame: u32) {
    let (name, last) = record.refs[slot];
    if last == now {
        return;
    }
    record.refs[slot].1 = now;
    match record.ref_changes.iter_mut().rev().take_while(|change| change.0 == frame).find(|change| change.1 == name) {
        Some(change) => change.2 = now,
        None => record.ref_changes.push((frame, name, now)),
    }
}

/// Whether a part is Glass: past Transparency 1 Glass still draws, bending and hiding what is behind
/// it, when a Highlight is under it (measured with ror: Balls at Transparency 1, 1.5 and 3 with
/// disabled Highlights), and effects can keep a disabled Highlight under their distortion bubbles. Transparency 1 does not hide it: it leaves the world instead.
fn is_glass(state: &State, engine: &Engine, id: InstanceId) -> bool {
    let (Some(name), Some(glass)) = (engine.reflection.name_id("Material"), engine.reflection.enum_item("Material", "Glass")) else {
        return false;
    };
    state.dom.raw_get(id, name).and_then(Value::as_enum) == Some(glass)
}

fn visibility_of(engine: &Engine, class: ClassId) -> Visibility {
    let name = class_name(engine, class);
    if is_a(engine, class, "BasePart") || is_a(engine, class, "Decal") {
        Visibility::Transparency
    } else if SWITCHABLE.iter().any(|c| is_a(engine, class, c)) || is_a(engine, class, "Light") {
        Visibility::Switch { clear: CLEARABLE.contains(&name) }
    } else if name == "SurfaceAppearance" {
        Visibility::Detach
    } else if is_a(engine, class, "LayerCollector") || name == "UIStroke" || name == "UIGradient" {
        Visibility::Enabled
    } else if is_a(engine, class, "GuiObject") {
        Visibility::Visible
    } else if MODIFIERS.iter().any(|c| is_a(engine, class, c)) {
        Visibility::Detach
    } else {
        Visibility::None
    }
}

/// Whether `id` was in the world at the last walk (and is recorded).
#[inline]
fn is_present(inner: &Inner, id: &InstanceId) -> bool {
    inner.by_id.get(id).is_some_and(|&index| inner.present.get(index).copied().unwrap_or(false))
}

fn walk_of(inner: &mut Inner, engine: &Engine, class: ClassId) -> Walk {
    // (known per class: a walk asks for each of a map's 20,000 instances)
    let slot = class as usize;
    if inner.walks_by_class.len() <= slot {
        inner.walks_by_class.resize(slot + 1, None);
    }
    *inner.walks_by_class[slot].get_or_insert_with(|| {
        if SKIPPED.iter().any(|c| is_a(engine, class, c)) {
            Walk::Skip
        } else if CLOSED.iter().any(|c| is_a(engine, class, c)) {
            Walk::Closed
        } else {
            Walk::Open
        }
    })
}

/// A ViewportFrame's CurrentCamera, when it has one.
fn viewport_camera(state: &State, engine: &Engine, frame: InstanceId) -> Option<InstanceId> {
    let name = engine.reflection.name_id("CurrentCamera")?;
    state.dom.raw_get(frame, name).and_then(Value::as_instance).filter(|&camera| state.dom.get(camera).is_some())
}

/// A camera as a ViewportFrame saves it: (property, file value, playback value) of CameraCFrame and
/// CameraFieldOfView (a window's skewed CFrame kept as it is).
fn camera_values(state: &State, engine: &Engine, camera: InstanceId) -> Vec<(NameId, Variant, Json)> {
    let mut values = Vec::new();
    let read = |name: &str| engine.reflection.name_id(name).and_then(|n| state.dom.raw_get(camera, n));
    if let Some(name) = engine.reflection.name_id("CameraCFrame") {
        let transform = frame::affine_of(read("CFrame"));
        values.push((name, Variant::CFrame(frame::from_affine(&transform)), cframe_json(&transform)));
    }
    if let (Some(name), Some(fov)) = (engine.reflection.name_id("CameraFieldOfView"), read("FieldOfView").and_then(Value::as_number)) {
        values.push((name, Variant::Float32(fov as f32), json!(round(fov))));
    }
    values
}

/// The Workspace's CurrentCamera: not recorded itself, but what is parented to it is drawn in the
/// world as Roblox draws it (local-only content, such as a game's client-only world folders), recorded as roots.
fn is_drawn_camera(state: &State, engine: &Engine, id: InstanceId) -> bool {
    let world = engine.world();
    id == world.camera && state.dom.parent_of(id) == Some(world.workspace)
}

/// The Workspace's Terrain: its voxels are not recorded, but what is parented to it is drawn in the
/// world as Roblox draws it (measured in a capture Studio 2026-10-17: a Neon part and its particles
/// under workspace.Terrain draw), recorded as roots. Effect modules can clone their meshes into it.
fn is_drawn_terrain(state: &State, engine: &Engine, id: InstanceId) -> bool {
    let world = engine.world();
    state.dom.parent_of(id) == Some(world.workspace) && is_a(engine, state.dom.at(id).class, "Terrain")
}

/// What finds the Terrain's Clouds: the Clouds and Terrain classes and the Workspace.
#[derive(Clone, Copy)]
struct TerrainClouds {
    clouds: ClassId,
    terrain: ClassId,
    workspace: InstanceId,
}

impl TerrainClouds {
    fn of(engine: &Engine) -> Option<Self> {
        Some(Self { clouds: engine.reflection.class_id("Clouds")?, terrain: engine.reflection.class_id("Terrain")?, workspace: engine.world().workspace })
    }

    /// Whether `id` is a Clouds in the Workspace's Terrain: the Terrain's clouds, whatever its name
    /// (Roblox's documentation: a Clouds draws only as the Terrain's child).
    fn holds(self, state: &State, id: InstanceId) -> bool {
        let Some(instance) = state.dom.get(id).filter(|instance| instance.class == self.clouds) else { return false };
        instance.parent.and_then(|parent| state.dom.get(parent)).is_some_and(|terrain| terrain.class == self.terrain && terrain.parent == Some(self.workspace))
    }
}

/// Transparency as drawn: LocalTransparencyModifier over Transparency.
fn effective_transparency(state: &State, engine: &Engine, id: InstanceId) -> f64 {
    let read = |name: &str| engine.reflection.name_id(name).and_then(|n| state.dom.number(id, n)).unwrap_or(0.0);
    1.0 - (1.0 - read("Transparency")) * (1.0 - local_modifier(state, engine, id))
}

/// A part's or decal's LocalTransparencyModifier (0 for anything else).
fn local_modifier(state: &State, engine: &Engine, id: InstanceId) -> f64 {
    engine.reflection.name_id("LocalTransparencyModifier").and_then(|n| state.dom.number(id, n)).unwrap_or(0.0)
}

/// The value a tracked property is recorded as (None: it cannot be played back).
fn current(state: &State, engine: &Engine, id: InstanceId, name: NameId, track: Track) -> Option<Json> {
    if track == Track::Effective {
        return Some(json!(round(effective_transparency(state, engine, id))));
    }
    // a Content property is a view of its string asset id (MeshPart.TextureContent: TextureID)
    let class = state.dom.at(id).class;
    if let Some(sibling) = engine.reflection.content_sibling(class, name) {
        let uri = state.dom.raw_get(id, sibling).and_then(|value| match value {
            Value::String(uri) => Some(uri.to_string()),
            _ => None,
        }).unwrap_or_default();
        // (a relative local file as the file the engine read: encode::local_uri)
        return Some(json!(encode::local_uri(engine, &uri).unwrap_or(uri)));
    }
    let value = state.dom.raw_get(id, name)?;
    let content = matches!(value, Value::Content(_))
        || engine.reflection.class_property(class, name).is_some_and(|property| property.engine_type == crate::reflection::EngineType::ContentId);
    match to_json(engine, value)? {
        Json::String(uri) if content => Some(json!(encode::local_uri(engine, &uri).unwrap_or(uri))),
        other => Some(other),
    }
}

/// The tracked Content views of a string asset id a script wrote (TextureID: TextureContent), which
/// the write changed too.
fn content_views(inner: &mut Inner, engine: &Engine, class: ClassId, written: NameId) -> Vec<(NameId, Track)> {
    tracked(inner, engine, class).iter().copied().filter(|&(name, _)| engine.reflection.content_sibling(class, name) == Some(written)).collect()
}

/// Two positions or rotations apart (a moved CFrame).
fn differs(a: &Affine3A, b: &Affine3A) -> bool {
    (a.translation - b.translation).abs().max_element() > 1e-4 || (a.matrix3 - b.matrix3).to_cols_array().iter().any(|v| v.abs() > 1e-5)
}

/// Where an attachment is drawn: a bone where its Transform puts it (TransformedWorldCFrame), as an
/// Animator's pose moves the skinned meshes it drives.
fn drawn_cframe(state: &State, engine: &Engine, id: InstanceId) -> Affine3A {
    if is_a(engine, state.dom.at(id).class, "Bone") { world::transformed_world_cframe(state, engine, id) } else { world::world_cframe(state, engine, id) }
}

/// A bone its own Transform moves off its CFrame.
fn posed_bone(state: &State, engine: &Engine, id: InstanceId) -> bool {
    is_a(engine, state.dom.at(id).class, "Bone")
        && engine.reflection.name_id("Transform").is_some_and(|name| differs(&frame::affine_of(state.dom.raw_get(id, name)), &Affine3A::IDENTITY))
}

fn finite_affine(transform: &Affine3A) -> bool {
    transform.matrix3.is_finite() && transform.translation.is_finite()
}

/// A unique name for a copy under `parent` (the file's roots for None): siblings sharing a name
/// get `~2`, `~3`, ... so every playback path names one instance.
fn unique_name(inner: &mut Inner, parent: Option<usize>, name: &str) -> String {
    use std::fmt::Write;
    let entry = inner.names.entry(parent).or_default();
    if !entry.used.contains(name) {
        let owned = name.to_string();
        entry.used.insert(owned.clone());
        return owned;
    }
    let mut count = entry.next.get(name).copied().unwrap_or(2);
    let mut candidate = String::with_capacity(name.len() + 8);
    loop {
        candidate.clear();
        candidate.push_str(name);
        candidate.push('~');
        let _ = write!(&mut candidate, "{count}");
        if !entry.used.contains(&candidate) {
            break;
        }
        count += 1;
    }
    entry.used.insert(candidate.clone());
    if let Some(slot) = entry.next.get_mut(name) {
        *slot = count + 1;
    } else {
        entry.next.insert(name.to_string(), count + 1);
    }
    candidate
}

/// A name as a step of a playback path (ror's path syntax: quoted when dots, brackets or edge
/// spaces would split or trim it).
fn segment(name: &str) -> String {
    if name.is_empty() || name.contains(['.', '[', ']']) || name.starts_with(' ') || name.ends_with(' ') {
        let quote = if name.contains('"') { '\'' } else { '"' };
        format!("[{quote}{name}{quote}]")
    } else {
        name.to_string()
    }
}

fn join(path: Option<&str>, name: &str) -> String {
    let piece = segment(name);
    match path {
        None => piece,
        Some(path) if piece.starts_with('[') => format!("{path}{piece}"),
        Some(path) => format!("{path}.{piece}"),
    }
}

/// Precomputes the per-class reflection and tracking metadata once per `ClassId` so `snapshot`
/// and `appear` avoid repeated `rbx_reflection_database` walks, sorting, and string interning.
fn snapshot_plan(inner: &mut Inner, engine: &Engine, class: ClassId) -> Arc<SnapshotPlan> {
    if let Some(found) = inner.plans.get(&class) {
        return found.clone();
    }
    let class_name = class_name(engine, class);
    // the builder sets the name, the parent and the class, and the attributes and tags are written below
    let reserved = |property: &str| matches!(property, "Parent" | "Name" | "ClassName" | "Tags" | "Attributes" | "Source");
    let mut serialized = Vec::new();
    let mut serialized_names: FxHashSet<&str> = FxHashSet::default();
    for (property, data_type) in encode::serialized_properties(class_name) {
        if reserved(&property) {
            continue;
        }
        let Some(name) = engine.reflection.name_id(&property) else { continue };
        // a view whose value lives in another property: UICorner's CornerRadius reads TopLeftRadius
        // (services/mod.rs, measured)
        let stored = if class_name == "UICorner" && &*property == "CornerRadius" { K::TopLeftRadius as NameId } else { name };
        let sibling = engine.reflection.content_sibling(class, name);
        let is_ref = matches!(data_type, rbx_reflection::DataType::Value(dom::VariantType::Ref));
        let dom_name = ustr(&property);
        serialized_names.insert(dom_name.as_str());
        serialized.push(SerializedProp { dom_name, stored, sibling, data_type, is_ref });
    }
    // serialized in Studio's dump but unknown to rbx-dom's database (every property of a class it does
    // not know, such as UIShadow, Name among them)
    let info = engine.reflection.class(class);
    let mut extra_raw: Vec<(&str, NameId, crate::reflection::EngineType)> = info
        .properties
        .iter()
        .filter_map(|(&name, &index)| {
            let property = engine.reflection.property(index);
            let text = engine.reflection.name(name);
            (property.serialized
                && !reserved(text)
                && encode::dom_property(class_name, text).is_none()
                && !serialized_names.contains(text))
            .then_some((text, name, property.engine_type))
        })
        .collect();
    crate::sort::by(&mut extra_raw, |a, b| a.0.cmp(b.0));
    let extra: Vec<ExtraProp> = extra_raw
        .into_iter()
        .map(|(text, name, engine_type)| ExtraProp {
            dom_name: ustr(text),
            name,
            sibling: engine.reflection.content_sibling(class, name),
            engine_type,
        })
        .collect();
    let known_attrs = encode::dom_property(class_name, "Attributes").is_some();
    let tracked_list = tracked(inner, engine, class);
    let transp = transparency_name(engine);
    let tracked_props: Vec<TrackedProp> = tracked_list
        .iter()
        .filter(|&&(_, track)| !matches!(track, Track::Motion | Track::Attachment))
        .map(|&(name, track)| TrackedProp {
            name,
            track,
            sibling: if track == Track::Effective { None } else { engine.reflection.content_sibling(class, name) },
            is_content_id: engine
                .reflection
                .class_property(class, name)
                .is_some_and(|property| property.engine_type == crate::reflection::EngineType::ContentId),
            file_key: if track == Track::Effective { transp } else { name },
        })
        .collect();
    let plan = Arc::new(SnapshotPlan {
        serialized,
        extra,
        known_attrs,
        visibility: visibility_of(engine, class),
        is_part: is_a(engine, class, "BasePart"),
        is_attachment: is_a(engine, class, "Attachment"),
        is_bone: is_a(engine, class, "Bone"),
        is_viewport: class_name == "ViewportFrame",
        is_explosion: class_name == "Explosion",
        tracked: tracked_props,
    });
    inner.plans.insert(class, plan.clone());
    plan
}

/// The copy of `id` as it is now: its serialized properties (as rbx-dom names and stores them, then
/// those only Studio's newer dump serializes, such as a UICorner's per-corner radii), references,
/// attributes and tags.
#[inline(never)]
fn snapshot(state: &State, engine: &Engine, id: InstanceId, plan: &SnapshotPlan) -> (Vec<(Ustr, Variant)>, Vec<(Ustr, InstanceId)>) {
    let mut properties = Vec::with_capacity(plan.serialized.len() + plan.extra.len());
    let mut references = Vec::new();
    for prop in &plan.serialized {
        if prop.is_ref {
            if let Some(Value::Instance(target)) = state.dom.raw_get(id, prop.stored) {
                references.push((prop.dom_name, *target));
            }
            continue;
        }
        let variant = if let Some(sibling) = prop.sibling
            && let Some(Value::String(uri)) = state.dom.raw_get(id, sibling)
        {
            encode::content_uri_to_variant(uri, prop.data_type)
        } else {
            state.dom.raw_get(id, prop.stored).and_then(|value| encode::to_variant(engine, value, prop.data_type, &|_| None))
        };
        if let Some(mut variant) = variant {
            encode::localize(engine, &mut variant);
            properties.push((prop.dom_name, variant));
        }
    }
    for prop in &plan.extra {
        let variant = if let Some(sibling) = prop.sibling
            && let Some(Value::String(uri)) = state.dom.raw_get(id, sibling)
        {
            Some(encode::content_uri_untyped(uri))
        } else {
            state.dom.raw_get(id, prop.name).and_then(|value| encode::untyped_variant(engine, value, prop.engine_type))
        };
        if let Some(mut variant) = variant {
            encode::localize(engine, &mut variant);
            properties.push((prop.dom_name, variant));
        }
    }
    let instance = state.dom.at(id);
    // what the model it came from held that the engine does not model (a mesh's InitialSize, ...)
    if let Some(unmodeled) = &instance.unmodeled {
        for (name, mut value) in unmodeled.iter() {
            if !properties.iter().any(|(property, _)| property.as_str() == name) {
                encode::localize(engine, &mut value);
                properties.push((ustr(name), value));
            }
        }
    }
    // rbx-binary writes attributes and tags only through its database's Instance entry
    if plan.known_attrs && !instance.attributes.is_empty() {
        let mut attributes = dom::Attributes::new();
        for (key, value) in &instance.attributes {
            if let Some(variant) = attribute_variant(engine, value) {
                attributes.insert(key.to_str_lossy().into_owned(), variant);
            }
        }
        properties.push((ustr("Attributes"), Variant::Attributes(attributes)));
    }
    if plan.known_attrs && !instance.tags.is_empty() {
        let tags: Vec<String> = instance.tags.iter().map(|t| t.to_str_lossy().into_owned()).collect();
        properties.push((ustr("Tags"), Variant::Tags(tags.into())));
    }
    (properties, references)
}

fn attribute_variant(engine: &Engine, value: &Value) -> Option<Variant> {
    Some(match value {
        Value::Bool(b) => Variant::Bool(*b),
        Value::Number(n) => Variant::Float64(*n),
        Value::String(s) => Variant::String(s.to_str_lossy().into_owned()),
        Value::Vector3(v) => Variant::Vector3(*v),
        Value::Vector2(v) => Variant::Vector2(*v),
        Value::Color3(c) => Variant::Color3(*c),
        Value::UDim(u) => Variant::UDim(*u),
        Value::UDim2(u) => Variant::UDim2(*u),
        Value::Rect(r) => Variant::Rect(*r),
        Value::NumberRange(r) => Variant::NumberRange(*r),
        Value::NumberSequence(s) => Variant::NumberSequence((**s).clone()),
        Value::ColorSequence(s) => Variant::ColorSequence((**s).clone()),
        Value::EnumItem(item) => Variant::Enum(dom::Enum::from_u32(engine.reflection.enum_item_info(*item).value as u32)),
        Value::Dom(v) => (**v).clone(),
        _ => return None,
    })
}

/// Records a change of `name` at the current frame (the latest value in a frame wins).
fn change(record: &mut Record, name: NameId, track: Track, value: Json, frame: u32) {
    let previous = record.last.get(&name).or_else(|| record.file.get(&name));
    if previous == Some(&value) {
        return;
    }
    record.last.insert(name, value.clone());
    let list = match record.changes.iter_mut().find(|(n, _, _)| *n == name) {
        Some((_, _, list)) => list,
        None => {
            record.changes.push((name, track, Vec::new()));
            &mut record.changes.last_mut().unwrap().2
        }
    };
    match list.last_mut() {
        Some(last) if last.0 == frame => last.1 = value,
        _ => list.push((frame, value)),
    }
}

fn add_motion(record: &mut Record, frame: u32, cframe: Affine3A, warnings: &mut Vec<String>) {
    if !finite_affine(&cframe) {
        let warning = format!("a CFrame with a NaN or an infinity was written to {}: it is drawn where it last was", record.path);
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
        return;
    }
    if let Some(last) = record.motion.last_mut() {
        if last.0 == frame {
            last.1 = cframe;
            return;
        }
    }
    let previous = record.motion.last().map(|(_, c)| *c).or(record.rest).unwrap_or(Affine3A::IDENTITY);
    if !differs(&previous, &cframe) {
        return;
    }
    let last_frame = record.motion.last().map(|(f, _)| *f);
    if frame > record.appear && last_frame.is_none_or(|f| f + 1 < frame) {
        record.motion.push((frame - 1, previous));
    }
    record.motion.push((frame, cframe));
}

/// An attachment or bone whose own track starts after it appeared (a Transform first set later, an
/// AnimationTrack starting mid-recording): until then it rode what carries it (the nearest ancestor
/// part, or an ancestor attachment with a track of its own) at the offset it appeared at. Its track
/// gets those frames, so the playback does not hold it at its first sample while its part moves.
fn backfill_carried(inner: &mut Inner, state: &State, engine: &Engine, index: usize, frame: u32) {
    let record = &inner.records[index];
    if !record.attachment || !record.motion.is_empty() || frame <= record.appear + 1 {
        return;
    }
    let Some(rest) = record.rest else { return };
    let appear = record.appear;
    let mut carrier = None;
    let mut current = state.dom.parent_of(record.id);
    while let Some(id) = current {
        let class = state.dom.at(id).class;
        let part = is_a(engine, class, "BasePart");
        if !part && !is_a(engine, class, "Attachment") {
            break;
        }
        if let Some(&found) = inner.by_id.get(&id) {
            if part || !inner.records[found].motion.is_empty() {
                carrier = Some(found);
                break;
            }
        }
        current = state.dom.parent_of(id);
    }
    let Some(carrier) = carrier else { return };
    let carried = &inner.records[carrier];
    if carried.motion.is_empty() {
        return;
    }
    let at = |f: u32| carried.motion.iter().rev().find(|(key, _)| *key <= f).map(|(_, cframe)| *cframe).or(carried.rest);
    let Some(base) = at(appear) else { return };
    let offset = base.inverse() * rest;
    let keys: Vec<(u32, Affine3A)> = (appear..frame).filter_map(|f| at(f).map(|cframe| (f, cframe * offset))).collect();
    let mut warnings = std::mem::take(&mut inner.warnings);
    for (f, cframe) in keys {
        add_motion(&mut inner.records[index], f, cframe, &mut warnings);
    }
    inner.warnings = warnings;
}

/// The parent record an instance's copy goes under: its parent's record, the file's roots for
/// the Workspace's children, Lighting's folder for Lighting's children; None when it is not
/// recorded at all (its parent was not).
fn parent_slot(inner: &Inner, state: &State, engine: &Engine, id: InstanceId) -> Option<(Option<usize>, bool)> {
    if let Some(adornee) = inner.adorned.get(&id) {
        return inner.by_id.get(adornee).map(|&index| (Some(index), inner.records[index].lighting_child));
    }
    if inner.screens.contains_key(&id) {
        return Some((None, false));
    }
    let parent = placed_parent(state, engine, id)?;
    // (most instances' parent is recorded: the Workspace, the drawn camera and Lighting never are)
    if let Some(&index) = inner.by_id.get(&parent) {
        return Some((Some(index), inner.records[index].lighting_child));
    }
    let world = engine.world();
    if is_drawn_camera(state, engine, parent) && is_a(engine, state.dom.at(id).class, "PostEffect") {
        // a post effect in the camera draws exactly as one in Lighting (measured in Studio
        // 2026-10-17: a Camera's BloomEffect and ColorCorrectionEffect matched the Lighting's)
        return Some((None, true));
    }
    if parent == world.workspace || is_drawn_camera(state, engine, parent) || is_drawn_terrain(state, engine, parent) {
        return Some((None, false));
    }
    let lighting = super::services::find_service(engine, world.game, "Lighting");
    if Some(parent) == lighting {
        return Some((None, true));
    }
    inner.by_id.get(&parent).map(|&index| (Some(index), inner.records[index].lighting_child))
}

/// The instance an instance draws under: an adorned GUI's adornee, a ScreenGui's PlayerGui (whatever
/// Folder of it holds it), else its parent.
fn drawn_parent(inner: &Inner, state: &State, engine: &Engine, id: InstanceId) -> Option<InstanceId> {
    inner.adorned.get(&id).or_else(|| inner.screens.get(&id)).copied().or_else(|| placed_parent(state, engine, id))
}

/// The nearest instance above `id` that is not a Studio-only placeholder the recording passes over (one that draws; a folder or
/// model is drawn once something the run made is under it, [`push_holders`]).
fn placed_parent(state: &State, engine: &Engine, id: InstanceId) -> Option<InstanceId> {
    let mut parent = state.dom.parent_of(id)?;
    while state.dom.at(parent).has(flags::STUDIO_ONLY) && !holds(state, engine, parent) {
        parent = state.dom.parent_of(parent)?;
    }
    Some(parent)
}

/// A placeholder of a class that only holds what is under it (a folder, a model): drawn for what the run put in it.
fn holds(state: &State, engine: &Engine, id: InstanceId) -> bool {
    let class = state.dom.at(id).class;
    is_a(engine, class, "Folder") || is_a(engine, class, "Model")
}

/// Before `id` (not a placeholder), the placeholder folders and models above it that hold it, outermost first, once a walk
/// (parents first, as the walk lists everything).
fn push_holders(state: &State, engine: &Engine, id: InstanceId, now: &mut Vec<InstanceId>, holders: &mut FxHashSet<InstanceId>) {
    let mut chain: Vec<InstanceId> = Vec::new();
    let mut at = state.dom.parent_of(id);
    while let Some(parent) = at.filter(|&parent| state.dom.at(parent).has(flags::STUDIO_ONLY)) {
        if holds(state, engine, parent) && !holders.contains(&parent) {
            chain.push(parent);
        }
        at = state.dom.parent_of(parent);
    }
    for &holder in chain.iter().rev() {
        holders.insert(holder);
        now.push(holder);
    }
}

/// A new record for `id`, appearing at the current frame.
fn appear(inner: &mut Inner, state: &State, engine: &Engine, id: InstanceId, parent: Option<usize>, lighting: bool) -> usize {
    let frame = inner.frame;
    let class = state.dom.at(id).class;
    let name = state.dom.at(id).name.to_str_lossy().into_owned();
    let lighting_root = lighting && parent.is_none();
    let screen = parent.is_none() && inner.screens.contains_key(&id);
    let unique = if lighting_root {
        unique_name(inner, Some(usize::MAX), &name)
    } else if screen {
        unique_name(inner, Some(usize::MAX - 1), &name)
    } else {
        unique_name(inner, parent, &name)
    };
    let path = if lighting_root {
        join(Some(LIGHTING_FOLDER), &unique)
    } else if screen {
        join(Some(PLAYER_GUI), &unique)
    } else {
        join(parent.map(|p| inner.records[p].path.as_str()), &unique)
    };
    let plan = snapshot_plan(inner, engine, class);
    let (properties, references) = snapshot(state, engine, id, &plan);
    let refs = reference_names(inner, engine, class).iter().map(|&name| (name, state.dom.raw_get(id, name).and_then(Value::as_instance))).collect();
    let visibility = if lighting_root { Visibility::Lighting } else { plan.visibility };
    let visibility = if visibility == Visibility::Transparency && is_glass(state, engine, id) { Visibility::Detach } else { visibility };
    let is_part = plan.is_part;
    let is_attachment = plan.is_attachment;
    let rest = (is_part || is_attachment).then(|| world::world_cframe(state, engine, id));
    let mut record = Record {
        id,
        class,
        class_name: class_name(engine, class).to_string(),
        name: unique,
        path,
        parent,
        appear: frame,
        visibility,
        properties,
        references,
        file: FxHashMap::default(),
        last: FxHashMap::default(),
        changes: Vec::new(),
        emits: Vec::new(),
        clears: Vec::new(),
        refs,
        ref_changes: Vec::new(),
        rest: rest.filter(finite_affine),
        motion: Vec::new(),
        physics_from: None,
        physics_steered: false,
        parents: Vec::new(),
        parent_id: drawn_parent(inner, state, engine, id),
        home: if screen { PLAYER_GUI.to_string() } else { parent.map(|p| inner.records[p].path.clone()).unwrap_or_default() },
        lighting_child: lighting,
        screen,
        attachment: is_attachment,
    };
    if rest.is_some_and(|r| !finite_affine(&r)) {
        inner.warnings.push(format!("{} appeared at a NaN or infinite CFrame: it is drawn where it next moves", record.path));
    }
    // a ViewportFrame's camera as Roblox files keep it: its CameraCFrame and CameraFieldOfView (the
    // CurrentCamera reference is not saved), followed as the camera moves (sample_changes)
    if plan.is_viewport {
        let index = inner.records.len();
        if let Some(camera) = viewport_camera(state, engine, id) {
            inner.viewports.entry(camera).or_default().push(index);
            for (name, variant, json) in camera_values(state, engine, camera) {
                let key = ustr(engine.reflection.name(name));
                record.properties.retain(|(property, _)| *property != key);
                record.properties.push((key, variant));
                record.file.insert(name, json);
            }
        }
    }
    // the file's values of what is tracked
    for prop in &plan.tracked {
        let value = if prop.track == Track::Effective {
            // the file holds the Transparency the client draws: files don't keep LocalTransparencyModifier, and a client draws
            // and shadows a part (and a decal) at 1 - (1 - T)(1 - M) (measured in a 0.747 client: ror P16 SH-2)
            let drawn = effective_transparency(state, engine, id);
            if local_modifier(state, engine, id) != 0.0 {
                let key = ustr("Transparency");
                match record.properties.iter_mut().find(|(property, _)| *property == key) {
                    Some(slot) => slot.1 = Variant::Float32(drawn as f32),
                    None => record.properties.push((key, Variant::Float32(drawn as f32))),
                }
            }
            Some(json!(round(drawn)))
        } else if let Some(sibling) = prop.sibling {
            let uri = state.dom.raw_get(id, sibling).and_then(|value| match value {
                Value::String(uri) => Some(uri.to_string()),
                _ => None,
            }).unwrap_or_default();
            Some(json!(encode::local_uri(engine, &uri).unwrap_or(uri)))
        } else if let Some(raw) = state.dom.raw_get(id, prop.name) {
            let content = prop.is_content_id || matches!(raw, Value::Content(_));
            to_json(engine, raw).map(|encoded| match encoded {
                Json::String(uri) if content => json!(encode::local_uri(engine, &uri).unwrap_or(uri)),
                other => other,
            })
        } else {
            None
        };
        if let Some(value) = value {
            record.file.insert(prop.file_key, value);
        }
    }
    // hidden in the file until it appears, then shown by the playback (a Lighting child is kept in the
    // file's ~Lighting folder, so the playback moves it into Lighting even from the first frame)
    if frame > 0 || matches!(record.visibility, Visibility::Lighting) {
        hide_in_file(&mut record, engine);
        show(&mut record, state, engine, frame);
    }
    if plan.is_explosion && frame > 0 {
        record.emits.push((frame, 1.0, None));
    }
    // an attachment hanging off something that is not a part is not carried by a part's motion
    if is_attachment && !state.dom.parent_of(id).is_some_and(|p| is_a(engine, state.dom.at(p).class, "BasePart") || is_a(engine, state.dom.at(p).class, "Attachment")) {
        inner.sampled.insert(inner.records.len());
    }
    // a bone posed before it appears: the file holds its CFrame without the Transform
    if plan.is_bone && posed_bone(state, engine, id) {
        inner.sampled.insert(inner.records.len());
    }
    let index = inner.records.len();
    inner.records.push(record);
    inner.by_id.insert(id, index);
    index
}

fn transparency_name(engine: &Engine) -> NameId {
    engine.reflection.name_id("Transparency").expect("Transparency")
}

fn enabled_name(engine: &Engine) -> NameId {
    engine.reflection.name_id("Enabled").expect("Enabled")
}

fn visible_name() -> NameId {
    K::Visible as NameId
}

/// The file holds the copy hidden (it appears later).
fn hide_in_file(record: &mut Record, engine: &Engine) {
    let set = |record: &mut Record, property: &str, variant: Variant, name: NameId, encoded: Json| {
        let key = ustr(property);
        record.properties.retain(|(p, _)| *p != key);
        record.properties.push((key, variant));
        record.file.insert(name, encoded);
    };
    match record.visibility {
        Visibility::Transparency => set(record, "Transparency", Variant::Float32(1.0), transparency_name(engine), json!(1.0)),
        Visibility::Switch { .. } | Visibility::Enabled => set(record, "Enabled", Variant::Bool(false), enabled_name(engine), json!(false)),
        Visibility::Visible => set(record, "Visible", Variant::Bool(false), visible_name(), json!(false)),
        Visibility::Detach => record.parents.push((0, None, String::new())),
        Visibility::Lighting | Visibility::None => {}
    }
}

/// The playback shows the copy as it is now (at its appearance, or coming back).
fn show(record: &mut Record, state: &State, engine: &Engine, frame: u32) {
    match record.visibility {
        Visibility::Transparency => change(record, transparency_name(engine), Track::Plain, json!(round(effective_transparency(state, engine, record.id))), frame),
        Visibility::Switch { .. } => {
            let on = state.dom.boolean(record.id, enabled_name(engine)).unwrap_or(true);
            change(record, enabled_name(engine), Track::Enabled, json!(on), frame);
        }
        Visibility::Enabled => {
            let on = state.dom.boolean(record.id, enabled_name(engine)).unwrap_or(true);
            change(record, enabled_name(engine), Track::Plain, json!(on), frame);
        }
        Visibility::Visible => {
            let on = state.dom.boolean(record.id, visible_name()).unwrap_or(true);
            change(record, visible_name(), Track::Plain, json!(on), frame);
        }
        Visibility::Lighting => record.parents.push((frame, None, "Lighting".into())),
        Visibility::Detach => {
            let home = record.home.clone();
            record.parents.push((frame, record.parent, home));
        }
        Visibility::None => {}
    }
}

/// The playback hides the copy (it left the world).
fn hide(record: &mut Record, engine: &Engine, frame: u32) {
    match record.visibility {
        Visibility::Transparency => change(record, transparency_name(engine), Track::Plain, json!(1.0), frame),
        Visibility::Switch { clear } => {
            change(record, enabled_name(engine), Track::Enabled, json!(false), frame);
            if clear {
                record.clears.push(frame);
            }
        }
        Visibility::Enabled => change(record, enabled_name(engine), Track::Plain, json!(false), frame),
        Visibility::Visible => change(record, visible_name(), Track::Plain, json!(false), frame),
        Visibility::Lighting => record.parents.push((frame, None, LIGHTING_FOLDER.into())),
        Visibility::Detach => record.parents.push((frame, None, String::new())),
        Visibility::None => {}
    }
}

/// Every tracked property of a record as it is now (a copy coming back, whose writes while it
/// was away were not seen).
fn resample(inner: &mut Inner, state: &State, engine: &Engine, index: usize) {
    let frame = inner.frame;
    let class = inner.records[index].class;
    let id = inner.records[index].id;
    for slot in 0..inner.records[index].refs.len() {
        let now = state.dom.raw_get(id, inner.records[index].refs[slot].0).and_then(Value::as_instance);
        change_reference(&mut inner.records[index], slot, now, frame);
    }
    for &(name, track) in tracked(inner, engine, class).iter() {
        match track {
            Track::Motion => {
                let cframe = world::world_cframe(state, engine, id);
                let mut warnings = std::mem::take(&mut inner.warnings);
                add_motion(&mut inner.records[index], frame, cframe, &mut warnings);
                inner.warnings = warnings;
            }
            Track::Attachment => {
                inner.sampled.insert(index);
            }
            Track::Effective => {}
            _ => {
                if let Some(value) = current(state, engine, id, name, track) {
                    change(&mut inner.records[index], name, track, value, frame);
                }
            }
        }
    }
}

fn sample(inner: &mut Inner, engine: &Rc<Engine>) {
    let state = engine.state.borrow();
    let world = engine.world();
    let lighting = super::services::find_service(engine, world.game, "Lighting");
    // (no parent changed anywhere since the last walk, no Adornee was written and the same camera is
    // drawn: the world holds what it held, where it was, so nothing appears, comes back, moves or leaves)
    stage_moves(inner, &state, engine);
    let walk = inner.walk_always || inner.adornee_written || inner.walked != Some((state.dom.structure_version, world.camera));
    if walk {
        crate::timing::time("frames: sample: walk", || walk_world(inner, &state, engine, lighting));
        inner.walked = Some((state.dom.structure_version, world.camera));
        inner.adornee_written = false;
    }
    crate::timing::time("frames: sample: changes", || sample_changes(inner, &state, engine, lighting));
}

/// The place's own look leaving Lighting and coming back: a sky, atmosphere or post effect the
/// recording never showed (there before the run, or gone before a frame showed it) taken out takes
/// the stage's of its class with it, as a cutscene that parks the map's look does;
/// one from before the run put back brings them back (one the run made is recorded when it comes).
fn stage_moves(inner: &mut Inner, state: &State, engine: &Engine) {
    for (id, class, into) in std::mem::take(&mut inner.lighting_moves) {
        if !LIGHTING_CHILDREN.iter().any(|name| is_a(engine, class, name)) {
            continue;
        }
        let placeholder = state.dom.get(id).is_some_and(|instance| instance.has(flags::STUDIO_ONLY));
        let own = if into { inner.ignored.contains(&id) || placeholder } else { !inner.by_id.contains_key(&id) };
        let name = class_name(engine, class);
        let shown = inner.stage.iter().rev().find(|(_, stage, _)| stage == name).is_none_or(|(_, _, shown)| *shown);
        if own && shown != into {
            inner.stage.push((inner.frame, name.to_string(), into));
        }
    }
}

/// Appearances, comings back, moves inside the world and departures since the last walk.
#[inline(never)]
fn walk_world(inner: &mut Inner, state: &State, engine: &Rc<Engine>, lighting: Option<InstanceId>) {
    let world = engine.world();
    let frame = inner.frame;

    // What is in the world now, parents first
    let mut now: Vec<InstanceId> = std::mem::take(&mut inner.walk_buffer);
    now.clear();
    let mut stack: Vec<InstanceId> = state.dom.children_of(world.workspace).iter().rev().copied().collect();
    // (the placeholder folders and models of this walk drawn for what is under them)
    let mut holders: FxHashSet<InstanceId> = FxHashSet::default();
    while let Some(id) = stack.pop() {
        let class = state.dom.at(id).class;
        let walk = walk_of(inner, engine, class);
        if walk == Walk::Skip {
            if is_drawn_camera(&state, engine, id) || is_drawn_terrain(&state, engine, id) {
                stack.extend(state.dom.children_of(id).iter().rev().copied());
            }
            continue;
        }
        // (a Studio-only placeholder is not drawn for itself, whenever it was built; what the run put in it is, under the
        // placeholder folders and models above it)
        if state.dom.at(id).has(flags::STUDIO_ONLY) {
            if walk == Walk::Open {
                stack.extend(state.dom.children_of(id).iter().rev().copied());
            }
            continue;
        }
        push_holders(state, engine, id, &mut now, &mut holders);
        now.push(id);
        if walk == Walk::Open {
            stack.extend(state.dom.children_of(id).iter().rev().copied());
        }
    }
    // SurfaceGuis and BillboardGuis in the local player's PlayerGui adorned to something in the world
    // draw there as if parented to it (a portal window can host its SurfaceGui in PlayerGui); its
    // ScreenGuis, in it or a Folder of it, draw on the screen (a cutscene's fade)
    inner.adorned.clear();
    inner.screens.clear();
    if let Some(player_gui) = world.local_player.and_then(|player| state.dom.children_of(player).iter().copied().find(|&c| is_a(engine, state.dom.at(c).class, "PlayerGui"))) {
        // (what the walk found so far: built only once a GUI here names an Adornee)
        let mut drawn: Option<FxHashSet<InstanceId>> = None;
        let adornee_name = engine.reflection.name_id("Adornee");
        // (instance, inside a ScreenGui)
        let mut guis: Vec<(InstanceId, bool)> = state.dom.children_of(player_gui).iter().rev().map(|&c| (c, false)).collect();
        while let Some((id, in_screen)) = guis.pop() {
            let class = state.dom.at(id).class;
            let adorned = (is_a(engine, class, "SurfaceGuiBase") || is_a(engine, class, "BillboardGui"))
                .then(|| adornee_name.and_then(|name| state.dom.raw_get(id, name)).and_then(Value::as_instance))
                .flatten()
                .filter(|adornee| drawn.get_or_insert_with(|| now.iter().copied().collect()).contains(adornee));
            let screen = !in_screen && class_name(engine, class) == "ScreenGui";
            if let Some(adornee) = adorned {
                inner.adorned.insert(id, adornee);
            } else if screen {
                inner.screens.insert(id, player_gui);
            } else {
                guis.extend(state.dom.children_of(id).iter().rev().map(|&c| (c, in_screen)));
                continue;
            }
            // the GUI's subtree; a world GUI inside a ScreenGui is looked at on its own
            let mut subtree = vec![id];
            while let Some(item) = subtree.pop() {
                let item_class = state.dom.at(item).class;
                let walk = walk_of(inner, engine, item_class);
                if walk == Walk::Skip {
                    continue;
                }
                if screen && item != id && (is_a(engine, item_class, "SurfaceGuiBase") || is_a(engine, item_class, "BillboardGui")) {
                    guis.push((item, true));
                    continue;
                }
                now.push(item);
                if walk == Walk::Open {
                    subtree.extend(state.dom.children_of(item).iter().rev().copied());
                }
            }
        }
    }
    if let Some(lighting) = lighting {
        let mut stack: Vec<InstanceId> = state
            .dom
            .children_of(lighting)
            .iter()
            .rev()
            .copied()
            .filter(|&c| {
                !inner.ignored.contains(&c)
                    && !state.dom.at(c).has(flags::STUDIO_ONLY)
                    && LIGHTING_CHILDREN.iter().any(|name| is_a(engine, state.dom.at(c).class, name))
            })
            .collect();
        while let Some(id) = stack.pop() {
            now.push(id);
            stack.extend(state.dom.children_of(id).iter().rev().copied());
        }
    }
    inner.walks += 1;
    let walk = inner.walks;
    let mut found: Vec<usize> = Vec::with_capacity(inner.present_list.len() + 16);

    // Appearances, comings back and moves inside the world
    for &id in &now {
        let Some((parent, in_lighting)) = parent_slot(inner, state, engine, id) else {
            // (recorded and in the world, though drawn under nothing now: still there)
            if let Some(&index) = inner.by_id.get(&id) {
                mark_seen(inner, &mut found, index, walk);
            }
            continue;
        };
        match inner.by_id.get(&id).copied() {
            None => {
                let index = appear(inner, state, engine, id, parent, in_lighting);
                mark_seen(inner, &mut found, index, walk);
            }
            Some(index) => {
                let current_parent = drawn_parent(inner, state, engine, id);
                if inner.records[index].parent_id != current_parent && !in_lighting {
                    let screen = parent.is_none() && inner.screens.contains_key(&id);
                    let path = if screen { PLAYER_GUI.to_string() } else { parent.map(|p| inner.records[p].path.clone()).unwrap_or_default() };
                    inner.records[index].parents.push((frame, parent, path.clone()));
                    inner.records[index].parent_id = current_parent;
                    inner.records[index].home = path;
                }
                if !inner.present.get(index).copied().unwrap_or(false) {
                    show(&mut inner.records[index], state, engine, frame);
                    resample(inner, state, engine, index);
                }
                mark_seen(inner, &mut found, index, walk);
            }
        }
    }
    // Departures: what was there at the last walk and is not now
    let previous = std::mem::replace(&mut inner.present_list, found);
    for &index in &previous {
        if inner.seen[index] != walk {
            inner.present[index] = false;
            hide(&mut inner.records[index], engine, frame);
        }
    }
    let found = std::mem::take(&mut inner.present_list);
    for &index in &found {
        inner.present[index] = true;
    }
    inner.present_list = found;
    inner.walk_buffer = now;
}

/// Record `index` is in the world at walk `walk` (once per walk).
#[inline]
fn mark_seen(inner: &mut Inner, found: &mut Vec<usize>, index: usize, walk: u32) {
    if inner.seen.len() <= index {
        inner.seen.resize(index + 1, 0);
        inner.present.resize(index + 1, false);
    }
    if inner.seen[index] != walk {
        inner.seen[index] = walk;
        found.push(index);
    }
}

/// What changed in the world this frame: property writes, emits and clears, attachments that move by
/// their own CFrame, sounds, the camera.
#[inline(never)]
fn sample_changes(inner: &mut Inner, state: &State, engine: &Rc<Engine>, lighting: Option<InstanceId>) {
    let world = engine.world();
    let frame = inner.frame;

    // Property writes of what was already there
    let dirty = std::mem::take(&mut inner.dirty);
    inner.dirty_set.clear();
    let lighting_id = lighting;
    let camera_names = (engine.reflection.name_id("CFrame"), engine.reflection.name_id("FieldOfView"), engine.reflection.name_id("CurrentCamera"));
    for (id, name) in dirty {
        // a moved camera moves the ViewportFrames that look through it; a frame given another camera
        // looks through that one
        if (Some(name) == camera_names.0 || Some(name) == camera_names.1) && let Some(frames) = inner.viewports.get(&id).cloned() {
            for index in frames {
                if inner.present.get(index).copied().unwrap_or(false) {
                    for (property, _, json) in camera_values(state, engine, id) {
                        change(&mut inner.records[index], property, Track::Plain, json, frame);
                    }
                }
            }
            continue;
        }
        if Some(name) == camera_names.2 && let Some(&index) = inner.by_id.get(&id) {
            if let Some(camera) = viewport_camera(state, engine, id) {
                let entry = inner.viewports.entry(camera).or_default();
                if !entry.contains(&index) {
                    entry.push(index);
                }
                for (property, _, json) in camera_values(state, engine, camera) {
                    change(&mut inner.records[index], property, Track::Plain, json, frame);
                }
            }
            continue;
        }
        let clouds = inner.terrain_clouds.is_some_and(|found| found.holds(state, id));
        if Some(id) == lighting_id || clouds {
            if let Some(mut record) = if clouds { inner.clouds.take() } else { inner.lighting.take() } {
                if let Some(&(_, track)) = tracked(inner, engine, record.class).iter().find(|(n, _)| *n == name) {
                    match current(&state, engine, id, name, track) {
                        Some(value) => change(&mut record, name, track, value, frame),
                        None => {
                            inner.unsupported.insert(format!("{}.{}", record.class_name, engine.reflection.name(name)));
                        }
                    }
                }
                *if clouds { &mut inner.clouds } else { &mut inner.lighting } = Some(record);
            }
            continue;
        }
        let Some(&index) = inner.by_id.get(&id) else { continue };
        if !is_present(inner, &id) || inner.records[index].appear >= frame {
            continue;
        }
        let class = inner.records[index].class;
        // an instance reference written after it appeared (a beam given its attachments once parented)
        if let Some(slot) = inner.records[index].refs.iter().position(|&(reference, _)| reference == name) {
            let now = state.dom.raw_get(id, name).and_then(Value::as_instance);
            change_reference(&mut inner.records[index], slot, now, frame);
            continue;
        }
        for (view, track) in content_views(inner, engine, class, name) {
            if let Some(value) = current(&state, engine, id, view, track) {
                change(&mut inner.records[index], view, track, value, frame);
            }
        }
        let Some(&(_, track)) = tracked(inner, engine, class).iter().find(|(n, _)| *n == name) else { continue };
        match track {
            Track::Motion => {
                let cframe = world::world_cframe(&state, engine, id);
                let mut warnings = std::mem::take(&mut inner.warnings);
                add_motion(&mut inner.records[index], frame, cframe, &mut warnings);
                inner.warnings = warnings;
                if inner.records[index].physics_from.is_none() && state.gravity.moved(id) {
                    inner.records[index].physics_from = Some(frame);
                }
                if state.gravity.steered(id) {
                    inner.records[index].physics_steered = true;
                }
            }
            Track::Attachment => {
                inner.sampled.insert(index);
            }
            Track::Effective => {
                let value = json!(round(effective_transparency(&state, engine, id)));
                change(&mut inner.records[index], transparency_name(engine), Track::Plain, value, frame);
            }
            _ => match current(&state, engine, id, name, track) {
                Some(value) => change(&mut inner.records[index], name, track, value, frame),
                None => {
                    let label = format!("{}.{}", inner.records[index].class_name, engine.reflection.name(name));
                    inner.unsupported.insert(label);
                }
            },
        }
    }

    // Emits and clears of what is in the world
    for (id, count, from) in std::mem::take(&mut inner.emits) {
        if let Some(&index) = inner.by_id.get(&id) {
            if is_present(inner, &id) {
                inner.records[index].emits.push((frame, count, from));
            }
        }
    }
    for id in std::mem::take(&mut inner.clears) {
        if let Some(&index) = inner.by_id.get(&id) {
            if is_present(inner, &id) {
                inner.records[index].clears.push(frame);
            }
        }
    }

    // Attachments that move by their own CFrame (parents first: a track starting late backfills
    // from what carried it, which may itself have just started)
    let mut sampled: Vec<usize> = inner.sampled.iter().copied().collect();
    crate::sort::ord(&mut sampled);
    for index in sampled {
        let id = inner.records[index].id;
        if is_present(inner, &id) {
            backfill_carried(inner, &state, engine, index, frame);
            let cframe = drawn_cframe(&state, engine, id);
            let mut warnings = std::mem::take(&mut inner.warnings);
            add_motion(&mut inner.records[index], frame, cframe, &mut warnings);
            inner.warnings = warnings;
        }
    }

    // Sounds
    for (event, time) in std::mem::take(&mut inner.sounds) {
        let (sound, played, local) = match event {
            WorldEvent::SoundPlayed { sound, local } => (sound, true, local),
            WorldEvent::SoundStopped { sound } => (sound, false, false),
            _ => continue,
        };
        if state.dom.get(sound).is_none() {
            continue;
        }
        let read = |name: &str| engine.reflection.name_id(name).and_then(|n| state.dom.raw_get(sound, n)).cloned();
        let sound_id = match read("SoundId") {
            Some(Value::String(s)) => s.to_str_lossy().into_owned(),
            Some(Value::Content(c)) => c.uri().map(|u| u.to_str_lossy().into_owned()).unwrap_or_default(),
            _ => String::new(),
        };
        inner.sound_log.push(SoundEvent {
            frame,
            time,
            played,
            local,
            path: state.dom.full_name(&engine.reflection, sound),
            sound_id,
            volume: read("Volume").and_then(|v| v.as_number()).unwrap_or(0.5),
            speed: read("PlaybackSpeed").and_then(|v| v.as_number()).unwrap_or(1.0),
        });
    }

    // The camera, when it moves
    let camera = world.camera;
    if state.dom.get(camera).is_some() {
        let cframe = frame::affine_of(state.dom.raw_get(camera, world::cframe_name(engine)));
        let fov = engine.reflection.name_id("FieldOfView").and_then(|n| state.dom.number(camera, n)).unwrap_or(70.0);
        let moved = inner.camera.last().is_none_or(|(_, last, last_fov)| differs(last, &cframe) || (last_fov - fov).abs() > 1e-3);
        if moved && finite_affine(&cframe) {
            inner.camera.push((frame, cframe, fov));
        }
    }
}

/// The record of a look the renderer's stage draws: Lighting's own properties (`id` Lighting), the
/// Terrain's Clouds (`id` the Workspace holding them). It holds no values from before the run: the
/// engine's instance stands in for the place's, so every write of the run is a change, and a property
/// holds the stage's value until the run first writes it.
fn stage_record(id: InstanceId, class: ClassId, class_name: &str, path: &str) -> Record {
    Record {
        id,
        class,
        class_name: class_name.into(),
        name: class_name.into(),
        path: path.into(),
        parent: None,
        appear: 0,
        visibility: Visibility::None,
        properties: Vec::new(),
        references: Vec::new(),
        file: FxHashMap::default(),
        last: FxHashMap::default(),
        changes: Vec::new(),
        emits: Vec::new(),
        clears: Vec::new(),
        refs: Vec::new(),
        ref_changes: Vec::new(),
        rest: None,
        motion: Vec::new(),
        physics_from: None,
        physics_steered: false,
        parents: Vec::new(),
        parent_id: None,
        home: String::new(),
        lighting_child: false,
        screen: false,
        attachment: false,
    }
}

// Output

/// The scene: every record's copy as it appeared, parents first; references pointed at copies.
/// A UniqueId another copy has already (clones keep their template's) is replaced with one made
/// from it and a count, before rbx-dom would replace it with a random one: the same run writes the
/// same bytes.
#[inline(never)]
fn scene(inner: &Inner) -> Result<Vec<u8>, String> {
    let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    let mut refs: Vec<dom::Ref> = Vec::with_capacity(inner.records.len());
    let mut lighting_folder: Option<dom::Ref> = None;
    let mut player_gui: Option<dom::Ref> = None;
    let mut roots: Vec<dom::Ref> = Vec::new();
    let mut unique_ids: FxHashSet<dom::UniqueId> = FxHashSet::default();
    let mut next_index = 0u32;
    let unique_id_key = ustr("UniqueId");
    for record in &inner.records {
        let parent = match record.parent {
            Some(index) => refs[index],
            None if record.lighting_child => *lighting_folder.get_or_insert_with(|| {
                let folder = dom.insert(root, InstanceBuilder::new("Folder").with_name(LIGHTING_FOLDER));
                roots.push(folder);
                folder
            }),
            None if record.screen => *player_gui.get_or_insert_with(|| {
                let gui = dom.insert(root, InstanceBuilder::new("PlayerGui").with_name(PLAYER_GUI));
                roots.push(gui);
                gui
            }),
            None => root,
        };
        let builder = InstanceBuilder::new(record.class_name.as_str())
            .with_name(record.name.as_str())
            .with_properties(record.properties.iter().map(|&(property, ref value)| {
                let value = match value {
                    // (rbx-dom keeps only the UniqueId property unique)
                    Variant::UniqueId(id) if property == unique_id_key && !unique_ids.insert(*id) => {
                        let unique = loop {
                            let candidate = dom::UniqueId::new(next_index, id.time(), id.random());
                            next_index = next_index.wrapping_add(1);
                            if unique_ids.insert(candidate) {
                                break candidate;
                            }
                        };
                        Variant::UniqueId(unique)
                    }
                    other => other.clone(),
                };
                (property, value)
            }));
        let referent = dom.insert(parent, builder);
        if parent == root {
            roots.push(referent);
        }
        refs.push(referent);
    }
    for (index, record) in inner.records.iter().enumerate() {
        for &(property, target) in &record.references {
            let target = inner.by_id.get(&target).map(|&i| refs[i]).unwrap_or_else(dom::Ref::none);
            if let Some(instance) = dom.get_by_ref_mut(refs[index]) {
                instance.properties.insert(property, Variant::Ref(target));
            }
        }
    }
    let mut bytes = Vec::with_capacity(inner.records.len().saturating_mul(128));
    rbx_binary::to_writer(&mut bytes, &dom, &roots).map_err(|e| format!("scene.rbxm: {e}"))?;
    Ok(bytes)
}

/// How far a motion track's line may stray from a dropped key's position, in studs. A fixed distance: a bound that
/// grew with the distance from the origin let a part 400 studs out stray 0.08 studs, which reads as a shake when the
/// camera (keyed far more finely) follows that part.
const MOTION_POSITION_STUDS: f64 = 2e-3;

/// What a motion key's numbers may stray by: its position by a fixed distance, its rotation by a proportion.
#[inline]
fn motion_allowance(component: usize, value: f64) -> f64 {
    if component < 3 { MOTION_POSITION_STUDS } else { 2e-4 * (1.0 + value.abs()) }
}

/// Typed playback JSON records with fields in exact alphabetical key order (matching `serde_json::Map`).
#[derive(Serialize)]
struct EnabledAction<'a> {
    at: f64,
    class: &'a str,
    enabled: bool,
    target: &'a str,
}

#[derive(Serialize)]
struct TimeScaleAction<'a> {
    at: f64,
    class: &'a str,
    target: &'a str,
    #[serde(rename = "timeScale")]
    time_scale: &'a Json,
}

struct SingleField<'a, V: Serialize>(&'a str, &'a V);

impl<V: Serialize> Serialize for SingleField<'_, V> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.0, self.1)?;
        map.end()
    }
}

#[derive(Serialize)]
struct SetAction<'a, V: Serialize> {
    at: f64,
    set: SingleField<'a, V>,
    target: &'a str,
}

#[derive(Serialize)]
struct EmitAction<'a> {
    at: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cframe: Option<[f64; 12]>,
    class: &'a str,
    emit: u64,
    #[serde(rename = "partSize", skip_serializing_if = "Option::is_none")]
    part_size: Option<[f64; 3]>,
    target: &'a str,
}

#[derive(Serialize)]
struct ClearAction<'a> {
    at: f64,
    class: &'a str,
    clear: bool,
    target: &'a str,
}

#[derive(Serialize)]
struct ParentAction<'a> {
    at: f64,
    parent: &'a str,
    target: &'a str,
}

#[derive(Serialize)]
struct StageAction<'a> {
    at: f64,
    class: &'a str,
    parent: &'a str,
    target: &'a str,
}

#[derive(Serialize)]
struct TweenKey<'a> {
    at: f64,
    value: &'a Json,
}

#[derive(Serialize)]
struct TweenTrack<'a> {
    keys: &'a [TweenKey<'a>],
    property: &'a str,
    target: &'a str,
}

#[derive(Serialize)]
struct MotionKey {
    at: f64,
    cframe: [f64; 12],
}

#[derive(Serialize)]
struct MotionTrack<'a> {
    keys: &'a [MotionKey],
    #[serde(rename = "physicsFrom", skip_serializing_if = "Option::is_none")]
    physics_from: Option<f64>,
    target: &'a str,
}

#[derive(Serialize)]
struct StillMotionKey {
    at: u32,
    cframe: [f64; 12],
}

#[derive(Serialize)]
struct StillMotionTrack<'a> {
    keys: [StillMotionKey; 1],
    target: &'a str,
}

/// Drops motion keys a straight line between their neighbours reproduces, directly on `[f64; 12]`.
fn reduce_motion(keys: Vec<MotionKey>) -> Vec<MotionKey> {
    if keys.len() <= 2 {
        return keys;
    }
    let mut kept = Vec::with_capacity(keys.len());
    kept.push(MotionKey { at: keys[0].at, cframe: keys[0].cframe });
    let mut anchor = 0usize;
    for index in 1..keys.len() - 1 {
        let a = &keys[anchor].cframe;
        let b = &keys[index + 1].cframe;
        let ta = keys[anchor].at;
        let tb = keys[index + 1].at;
        let fits = (anchor + 1..=index).all(|between| {
            let v = &keys[between].cframe;
            let alpha = (keys[between].at - ta) / (tb - ta).max(1e-9);
            (0..12).all(|c| (a[c] + (b[c] - a[c]) * alpha - v[c]).abs() <= motion_allowance(c, v[c]))
        });
        if !fits {
            kept.push(MotionKey { at: keys[index].at, cframe: keys[index].cframe });
            anchor = index;
        }
    }
    let last = &keys[keys.len() - 1];
    kept.push(MotionKey { at: last.at, cframe: last.cframe });
    kept
}

/// Drops tween keys a straight line between their neighbours reproduces, without cloning `Json` values.
fn reduce_tween<'a>(keys: Vec<TweenKey<'a>>, values: &[Option<Vec<f64>>]) -> Vec<TweenKey<'a>> {
    if keys.len() <= 2 {
        return keys;
    }
    let mut kept = Vec::with_capacity(keys.len());
    kept.push(TweenKey { at: keys[0].at, value: keys[0].value });
    let mut anchor = 0usize;
    for index in 1..keys.len() - 1 {
        let (a, b) = (&values[anchor], &values[index + 1]);
        let (ta, tb) = (keys[anchor].at, keys[index + 1].at);
        let fits = match (a, b) {
            (Some(a), Some(b)) if a.len() == b.len() => (anchor + 1..=index).all(|between| {
                let Some(v) = &values[between] else { return false };
                if v.len() != a.len() {
                    return false;
                }
                let alpha = (keys[between].at - ta) / (tb - ta).max(1e-9);
                v.iter().zip(a.iter().zip(b.iter())).all(|(v, (a, b))| (a + (b - a) * alpha - v).abs() <= 1e-4 * (1.0 + v.abs()))
            }),
            _ => false,
        };
        if !fits {
            kept.push(TweenKey { at: keys[index].at, value: keys[index].value });
            anchor = index;
        }
    }
    let last = &keys[keys.len() - 1];
    kept.push(TweenKey { at: last.at, value: last.value });
    kept
}

#[cfg(test)]
fn reduce(keys: Vec<Json>, field: &str, allowance: fn(usize, f64) -> f64) -> Vec<Json> {
    if keys.len() <= 2 {
        return keys;
    }
    let numbers = |key: &Json| key.get(field).and_then(encode::numbers);
    let mut kept = vec![keys[0].clone()];
    let mut anchor = 0usize;
    for index in 1..keys.len() - 1 {
        let (a, b) = (numbers(&keys[anchor]), numbers(&keys[index + 1]));
        let (ta, tb) = (keys[anchor]["at"].as_f64().unwrap_or(0.0), keys[index + 1]["at"].as_f64().unwrap_or(0.0));
        let fits = match (&a, &b) {
            (Some(a), Some(b)) if a.len() == b.len() => (anchor + 1..=index).all(|between| {
                let Some(v) = numbers(&keys[between]) else { return false };
                if v.len() != a.len() {
                    return false;
                }
                let alpha = (keys[between]["at"].as_f64().unwrap_or(0.0) - ta) / (tb - ta).max(1e-9);
                v.iter().enumerate().all(|(c, v)| (a[c] + (b[c] - a[c]) * alpha - v).abs() <= allowance(c, *v))
            }),
            _ => false,
        };
        if !fits {
            kept.push(keys[index].clone());
            anchor = index;
        }
    }
    kept.push(keys[keys.len() - 1].clone());
    kept
}

/// The playback (ror's README "Playback").
#[inline(never)]
fn playback(inner: &Inner) -> Playback {
    let mut actions: Vec<(f64, Box<[u8]>)> = Vec::new();
    let mut tweens: Vec<Box<[u8]>> = Vec::new();
    let mut motion: Vec<Box<[u8]>> = Vec::new();
    // (Lighting's and the Terrain's Clouds' values before the run's first write are the stage's: ror's `{"initial": true}`)
    let stage = json!({ "initial": true });
    let records = inner.records.iter().map(|record| (record, false)).chain(inner.lighting.iter().chain(inner.clouds.iter()).map(|record| (record, true)));
    for (record, staged) in records {
        let target = record.path.as_str();
        for (name, track, list) in &record.changes {
            let property = inner.property_names.get(name).map_or("", String::as_str);
            match track {
                Track::Enabled => {
                    for (frame, value) in list {
                        let at = time_of(inner, *frame);
                        actions.push((at, serialize_element(&EnabledAction { at, class: &record.class_name, enabled: value.as_bool() == Some(true), target })));
                    }
                    continue;
                }
                Track::TimeScale => {
                    for (frame, value) in list {
                        let at = time_of(inner, *frame);
                        actions.push((at, serialize_element(&TimeScaleAction { at, class: &record.class_name, target, time_scale: value })));
                    }
                    continue;
                }
                _ => {}
            }
            let continuous = list.windows(2).any(|pair| pair[1].0 == pair[0].0 + 1);
            let list_nums: Option<Vec<Vec<f64>>> = if continuous {
                let first = numbers(&list[0].1);
                if let Some(first_nums) = first {
                    let shape = first_nums.len();
                    let mut all = Vec::with_capacity(list.len());
                    all.push(first_nums);
                    let mut ok = true;
                    for (_, v) in &list[1..] {
                        match numbers(v) {
                            Some(n) if n.len() == shape => all.push(n),
                            _ => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    ok.then_some(all)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(list_nums) = list_nums {
                let shape = list_nums[0].len();
                let (initial, initial_nums): (Option<&Json>, Option<Vec<f64>>) = if staged {
                    (Some(&stage), None)
                } else {
                    match record.file.get(name) {
                        Some(v) => match numbers(v) {
                            Some(n) if n.len() == shape => (Some(v), Some(n)),
                            _ => (None, None),
                        },
                        None => (None, None),
                    }
                };
                let mut keys: Vec<TweenKey<'_>> = Vec::with_capacity(list.len() * 2);
                let mut key_nums: Vec<Option<Vec<f64>>> = Vec::with_capacity(list.len() * 2);
                let mut prev_frame: Option<u32> = None;
                let mut prev_val: Option<&Json> = initial;
                let mut prev_nums: Option<Vec<f64>> = initial_nums;
                for ((frame, value), nums) in list.iter().zip(list_nums.into_iter()) {
                    let gap = match prev_frame {
                        None => *frame > 0,
                        Some(last) => last + 1 < *frame,
                    };
                    if gap {
                        if let Some(held) = prev_val {
                            keys.push(TweenKey { at: time_of(inner, frame - 1), value: held });
                            key_nums.push(prev_nums.clone());
                        }
                    }
                    keys.push(TweenKey { at: time_of(inner, *frame), value });
                    key_nums.push(Some(nums.clone()));
                    prev_frame = Some(*frame);
                    prev_val = Some(value);
                    prev_nums = Some(nums);
                }
                let keys = reduce_tween(keys, &key_nums);
                tweens.push(serialize_element(&TweenTrack { keys: &keys, property, target }));
            } else {
                for (frame, value) in list {
                    let at = time_of(inner, *frame);
                    actions.push((at, serialize_element(&SetAction { at, set: SingleField(property, value), target })));
                }
            }
        }
        // each burst where its emitter was at the call (ror places it there: a script moving an attachment
        // between Emit calls in one frame leaves a burst at each place, where the frame's motion has one)
        for (frame, count, from) in &record.emits {
            let count = count.round();
            if count > 0.0 {
                let at = time_of(inner, *frame);
                let (cframe, part_size) = match from.filter(|from| finite_affine(&from.cframe)) {
                    Some(from) => (
                        Some(encode::cframe_numbers(&from.cframe)),
                        from.part_size.filter(|size| size.is_finite()).map(|size| [round(size.x as f64), round(size.y as f64), round(size.z as f64)]),
                    ),
                    None => (None, None),
                };
                actions.push((at, serialize_element(&EmitAction { at, cframe, class: &record.class_name, emit: count as u64, part_size, target })));
            }
        }
        // instance references written after it appeared: the path of the instance named, or none
        for (frame, name, named) in &record.ref_changes {
            let property = inner.property_names.get(name).map_or("", String::as_str);
            let target_path: Option<&str> = named.and_then(|named| inner.by_id.get(&named)).map(|&found| inner.records[found].path.as_str());
            let at = time_of(inner, *frame);
            actions.push((at, serialize_element(&SetAction { at, set: SingleField(property, &target_path), target })));
        }
        for frame in &record.clears {
            let at = time_of(inner, *frame);
            actions.push((at, serialize_element(&ClearAction { at, class: &record.class_name, clear: true, target })));
        }
        for (frame, _, path) in &record.parents {
            let at = time_of(inner, *frame);
            actions.push((at, serialize_element(&ParentAction { at, parent: path, target })));
        }
        if !record.motion.is_empty() {
            let mut keys: Vec<MotionKey> = Vec::with_capacity(record.motion.len() * 2);
            let mut prev_frame: Option<u32> = None;
            let mut prev_val: Option<[f64; 12]> = None;
            for (frame, cframe) in &record.motion {
                let gap = match prev_frame {
                    None => *frame > 0,
                    Some(last) => last + 1 < *frame,
                };
                if gap {
                    if let Some(held) = prev_val {
                        keys.push(MotionKey { at: time_of(inner, frame - 1), cframe: held });
                    }
                }
                let val = encode::cframe_numbers(cframe);
                keys.push(MotionKey { at: time_of(inner, *frame), cframe: val });
                prev_frame = Some(*frame);
                prev_val = Some(val);
            }
            let keys = reduce_motion(keys);
            let physics_from = record.physics_from.filter(|_| !record.physics_steered).map(|frame| time_of(inner, frame));
            motion.push(serialize_element(&MotionTrack { keys: &keys, physics_from, target }));
        }
    }
    for (frame, class, shown) in &inner.stage {
        let at = time_of(inner, *frame);
        actions.push((at, serialize_element(&StageAction { at, class, parent: if *shown { "Lighting" } else { "" }, target: "Lighting" })));
    }
    // a part that stays put under a part that moves stays put (ror carries what is under a moving part);
    // an attachment or a bone (an Attachment) is never pinned: it goes where its part goes
    for record in &inner.records {
        if record.motion.is_empty() && !record.attachment && record.rest.is_some() {
            let mut ancestor = record.parent;
            while let Some(index) = ancestor {
                let above = &inner.records[index];
                if !above.motion.is_empty() && above.rest.is_some() {
                    motion.push(serialize_element(&StillMotionTrack {
                        keys: [StillMotionKey { at: 0, cframe: encode::cframe_numbers(&record.rest.unwrap()) }],
                        target: &record.path,
                    }));
                    break;
                }
                ancestor = above.parent;
            }
        }
    }
    // (stable: equal times keep the order they were made in)
    crate::sort::by(&mut actions, |a, b| a.0.total_cmp(&b.0));
    let frames = ((inner.fps - 60.0).abs() > 1e-9 || inner.times.is_some()).then(|| Json::Array((0..=inner.frame).map(|f| json!(time_of(inner, f))).collect()));
    Playback { actions, tweens, motion, frames }
}

#[inline(never)]
fn events(inner: &Inner, engine: &Engine) -> Json {
    let state = engine.state.borrow();
    let output: Vec<Json> = state.output.entries.iter().map(event_json).collect();
    let sounds: Vec<Json> = inner
        .sound_log
        .iter()
        .map(|s| {
            json!({
                "time": round(s.time), "frame": s.frame, "event": if s.played { "play" } else { "stop" }, "local": s.local,
                "sound": s.path, "soundId": s.sound_id, "volume": round(s.volume), "playbackSpeed": round(s.speed),
            })
        })
        .collect();
    json!({ "output": output, "sounds": sounds })
}

/// The summary; `(errors, warns)`: the output lines of those kinds.
#[inline(never)]
fn summary(inner: &Inner, (errors, warns): (usize, usize), playback: &Playback, mut extra: Map<String, Json>) -> Json {
    // (a timing check's: the work each sampled frame did, ms at desktop prices, the first sample's being everything before it)
    if let Some(totals) = &inner.frame_work {
        let mut last = 0.0;
        let work: Vec<Json> = totals
            .iter()
            .map(|total| {
                let frame = (total - last) * 1000.0;
                last = *total;
                json!((frame * 1e6).round() / 1e6)
            })
            .collect();
        extra.insert("frame_work_ms".into(), Json::Array(work));
    }
    let mut activity: Vec<(u32, usize)> = Vec::new();
    let mut bump = |frame: u32, count: usize| match activity.iter_mut().find(|(f, _)| *f == frame) {
        Some((_, c)) => *c += count,
        None => activity.push((frame, count)),
    };
    for record in &inner.records {
        if record.appear > 0 {
            bump(record.appear, 1);
        }
        for (frame, count, _) in &record.emits {
            bump(*frame, *count as usize);
        }
    }
    crate::sort::ord(&mut activity);
    let count = |key: &str| match key {
        "actions" => playback.actions.len(),
        "tweens" => playback.tweens.len(),
        _ => playback.motion.len(),
    };
    extra.insert("frames".into(), json!(inner.frame + 1));
    extra.insert("fps".into(), json!(inner.fps));
    extra.insert("duration".into(), json!(time_of(inner, inner.frame)));
    extra.insert("instances".into(), json!(inner.records.len()));
    extra.insert("actions".into(), json!(count("actions")));
    extra.insert("tweens".into(), json!(count("tweens")));
    extra.insert("motion".into(), json!(count("motion")));
    extra.insert("emits".into(), json!(inner.records.iter().map(|r| r.emits.len()).sum::<usize>()));
    extra.insert("sounds".into(), json!(inner.sound_log.iter().filter(|s| s.played).count()));
    extra.insert("errors".into(), json!(errors));
    extra.insert("warnings".into(), json!(warns));
    extra.insert("recorderWarnings".into(), json!(inner.warnings));
    let mut unsupported: Vec<&String> = inner.unsupported.iter().collect();
    crate::sort::ord(&mut unsupported);
    extra.insert("unsupported".into(), json!(unsupported));
    extra.insert("activity".into(), Json::Array(activity.into_iter().map(|(frame, count)| json!({ "at": time_of(inner, frame), "count": count })).collect()));
    Json::Object(extra)
}
