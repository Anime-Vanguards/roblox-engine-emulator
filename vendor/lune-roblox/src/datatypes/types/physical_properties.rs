use core::fmt;

use mlua::prelude::*;
use rbx_dom_weak::types::CustomPhysicalProperties as DomCustomPhysicalProperties;

use lune_utils::TableBuilder;

use crate::exports::LuaExportsTable;

use super::{super::*, EnumItem};

/**
    An implementation of the [PhysicalProperties](https://create.roblox.com/docs/reference/engine/datatypes/PhysicalProperties) Roblox datatype.

    This implements all documented properties, methods & constructors of the `PhysicalProperties` class as of October 2025.
*/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalProperties {
    pub(crate) density: f32,
    pub(crate) friction: f32,
    pub(crate) friction_weight: f32,
    pub(crate) elasticity: f32,
    pub(crate) elasticity_weight: f32,
    pub(crate) acoustic_absorption: f32,
}

impl PhysicalProperties {
    pub(crate) fn from_material(material_enum_item: &EnumItem) -> Option<PhysicalProperties> {
        MATERIAL_ENUM_MAP
            .iter()
            .find(|props| props.0 == material_enum_item.name)
            .map(|props| PhysicalProperties {
                density: props.1,
                friction: props.2,
                elasticity: props.3,
                friction_weight: props.4,
                elasticity_weight: props.5,
                acoustic_absorption: props.6,
            })
    }
}

impl LuaExportsTable for PhysicalProperties {
    const EXPORT_NAME: &'static str = "PhysicalProperties";

    fn create_exports_table(lua: Lua) -> LuaResult<LuaTable> {
        type ArgsMaterial = LuaUserDataRef<EnumItem>;
        type ArgsNumbers = (f32, f32, f32, Option<f32>, Option<f32>, Option<f32>);

        let physical_properties_new = |lua: &Lua, args: LuaMultiValue| {
            if let Ok(value) = ArgsMaterial::from_lua_multi(args.clone(), lua) {
                if value.parent.desc.name == "Material" {
                    match PhysicalProperties::from_material(&value) {
                        Some(props) => Ok(props),
                        None => Err(LuaError::RuntimeError(format!(
                            "Found unknown Material '{}'",
                            value.name
                        ))),
                    }
                } else {
                    Err(LuaError::RuntimeError(format!(
                        "Expected argument #1 to be a Material, got {}",
                        value.parent.desc.name
                    )))
                }
            } else if let Ok((
                density,
                friction,
                elasticity,
                friction_weight,
                elasticity_weight,
                acoustic_absorption,
            )) = ArgsNumbers::from_lua_multi(args, lua)
            {
                // Roblox's documentation: each value is clamped to its acceptable range
                Ok(PhysicalProperties {
                    density: density.clamp(0.0001, 100.0),
                    friction: friction.clamp(0.0, 2.0),
                    friction_weight: friction_weight.unwrap_or(1.0).clamp(0.0, 100.0),
                    elasticity: elasticity.clamp(0.0, 1.0),
                    elasticity_weight: elasticity_weight.unwrap_or(1.0).clamp(0.0, 100.0),
                    acoustic_absorption: acoustic_absorption.unwrap_or(1.0).clamp(0.0, 1.0),
                })
            } else {
                // FUTURE: Better error message here using given arg types
                Err(LuaError::RuntimeError(
                    "Invalid arguments to constructor".to_string(),
                ))
            }
        };

        TableBuilder::new(lua)?
            .with_function("new", physical_properties_new)?
            .build_readonly()
    }
}

impl LuaUserData for PhysicalProperties {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("Density", |_, this| Ok(this.density));
        fields.add_field_method_get("Friction", |_, this| Ok(this.friction));
        fields.add_field_method_get("FrictionWeight", |_, this| Ok(this.friction_weight));
        fields.add_field_method_get("Elasticity", |_, this| Ok(this.elasticity));
        fields.add_field_method_get("ElasticityWeight", |_, this| Ok(this.elasticity_weight));
        fields.add_field_method_get("AcousticAbsorption", |_, this| Ok(this.acoustic_absorption));
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(LuaMetaMethod::Eq, userdata_impl_eq);
        methods.add_meta_method(LuaMetaMethod::ToString, userdata_impl_to_string);
    }
}

impl fmt::Display for PhysicalProperties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // six fields with AcousticAbsorption last, 9 significant digits each ("1, 0.300000012, 0.5, 1, 1, 1"; Studio 0.744)
        let g = |value: f32| super::format_g(f64::from(value), 9);
        write!(
            f,
            "{}, {}, {}, {}, {}, {}",
            g(self.density),
            g(self.friction),
            g(self.elasticity),
            g(self.friction_weight),
            g(self.elasticity_weight),
            g(self.acoustic_absorption)
        )
    }
}

impl From<DomCustomPhysicalProperties> for PhysicalProperties {
    fn from(v: DomCustomPhysicalProperties) -> Self {
        Self {
            density: v.density(),
            friction: v.friction(),
            friction_weight: v.friction_weight(),
            elasticity: v.elasticity(),
            elasticity_weight: v.elasticity_weight(),
            acoustic_absorption: v.acoustic_absorption(),
        }
    }
}

impl From<PhysicalProperties> for DomCustomPhysicalProperties {
    fn from(v: PhysicalProperties) -> Self {
        DomCustomPhysicalProperties::new(
            v.density,
            v.friction,
            v.elasticity,
            v.friction_weight,
            v.elasticity_weight,
            v.acoustic_absorption,
        )
    }
}

/*

    NOTE: The material definitions below are each material's values as a playing Roblox client reads them
    (PhysicalProperties.new(material) for every Enum.Material item, Studio 0.744: tools/parity/probes/
    physical_materials.luau in roblox-engine-emulator), each written as the shortest decimal that rounds to the
    float the client printed. (The table was generated with the physical_properties_enum_map script and rounded
    to two decimals, which put Basalt's density at 2.69 where Roblox has 2.691 and Water's friction at 0.)

*/
#[rustfmt::skip]
const MATERIAL_ENUM_MAP: &[(&str, f32, f32, f32, f32, f32, f32)] = &[
    ("Plastic",          0.7,    0.3,    0.5,    1.0,    1.0,    0.3),
    ("SmoothPlastic",    0.7,    0.2,    0.5,    1.0,    1.0,    0.2),
    ("Neon",             0.7,    0.3,    0.2,    1.0,    1.0,    0.3),
    ("Wood",            0.35,   0.48,    0.2,    1.0,    1.0,   0.25),
    ("WoodPlanks",      0.35,   0.48,    0.2,    1.0,    1.0,   0.25),
    ("Marble",         2.563,    0.2,   0.17,    1.0,    1.0,   0.01),
    ("Basalt",         2.691,    0.7,   0.15,    0.3,    1.0,    0.2),
    ("Slate",          2.691,    0.4,    0.2,    1.0,    1.0,   0.03),
    ("CrackedLava",    2.691,   0.65,   0.15,    1.0,    1.0,   0.35),
    ("Concrete",       2.403,    0.7,    0.2,    0.3,    1.0,   0.05),
    ("Limestone",      2.691,    0.5,   0.15,    1.0,    1.0,   0.15),
    ("Granite",        2.691,    0.4,    0.2,    1.0,    1.0,    0.2),
    ("Pavement",       2.691,    0.5,   0.17,    0.3,    1.0,    0.6),
    ("Brick",          1.922,    0.8,   0.15,    0.3,    1.0,   0.35),
    ("Pebble",         2.403,    0.4,   0.17,    1.0,    1.5,    0.4),
    ("Cobblestone",    2.691,    0.5,   0.17,    1.0,    1.0,   0.45),
    ("Rock",           2.691,    0.5,   0.17,    1.0,    1.0,   0.15),
    ("Sandstone",      2.691,    0.5,   0.15,    5.0,    1.0,    0.2),
    ("CorrodedMetal",   7.85,    0.7,    0.2,    1.0,    1.0,   0.15),
    ("DiamondPlate",    7.85,   0.35,   0.25,    1.0,    1.0,   0.05),
    ("Foil",             2.7,    0.4,   0.25,    1.0,    1.0,    0.1),
    ("Metal",           7.85,    0.4,   0.25,    1.0,    1.0,    0.1),
    ("Grass",            0.9,    0.4,    0.1,    1.0,    1.5,    0.7),
    ("LeafyGrass",       0.9,    0.4,    0.1,    2.0,    2.0,   0.75),
    ("Sand",           1.602,    0.5,   0.05,    5.0,    2.5,   0.55),
    ("Fabric",           0.7,   0.35,   0.05,    1.0,    1.0,   0.65),
    ("Snow",             0.9,    0.3,   0.03,    3.0,    4.0,    0.8),
    ("Mud",              0.9,    0.3,   0.07,    3.0,    4.0,   0.65),
    ("Ground",           0.9,   0.45,    0.1,    1.0,    1.0,    0.6),
    ("Asphalt",         2.36,    0.8,    0.2,    0.3,    1.0,    0.4),
    ("Salt",           2.165,    0.5,   0.05,    1.0,    1.0,   0.15),
    ("Ice",            0.919,   0.02,   0.15,    3.0,    1.0,    0.2),
    ("Glacier",        0.919,   0.05,   0.15,    2.0,    1.0,    0.7),
    ("Glass",          2.403,   0.25,    0.2,    1.0,    1.0,    0.1),
    ("ForceField",     2.403,   0.25,    0.2,    1.0,    1.0,    0.0),
    ("Air",             0.01,   0.01,   0.01,    1.0,    1.0,   0.01),
    ("Water",            1.0,  0.005,   0.01,    1.0,    1.0,   0.01),
    ("Cardboard",        0.7,    0.5,   0.05,    1.0,    2.0,   0.55),
    ("Carpet",           1.1,    0.4,   0.25,    1.0,    2.0,   0.65),
    ("CeramicTiles",     2.4,   0.51,    0.2,    1.0,    1.0,   0.04),
    ("ClayRoofTiles",    2.0,   0.51,    0.2,    1.0,    1.0,    0.3),
    ("RoofShingles",    2.36,    0.8,    0.2,    0.3,    1.0,    0.3),
    ("Leather",         0.86,   0.35,   0.25,    1.0,    1.0,   0.65),
    ("Plaster",         0.75,    0.6,    0.2,    0.3,    1.0,    0.3),
    ("Rubber",           1.3,    1.5,   0.95,    3.0,    2.0,    0.5),
];
