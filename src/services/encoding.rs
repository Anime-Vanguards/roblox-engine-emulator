//! EncodingService in a server run (`--run-context server`): CompressBuffer / DecompressBuffer /
//! GetDecompressedBufferSize (Zstd, the one CompressionAlgorithm) and Base64Encode / Base64Decode, as Roblox's
//! EncodingService documentation gives them: compressionLevel 1 by default, -7 to 22; a frame that does not hold its decompressed size, or holds
//! one past 1 GB, is refused. The zstd library is the one Roblox's documentation names; the error texts are
//! not measured. ComputeBufferHash / ComputeStringHash are not emulated (Output `missing`).

use std::rc::Rc;

use mlua::prelude::*;

use super::{Behaviors, method, values};
use crate::{
    Engine,
    lua::{Args, Flow, raise, types::LEnumItem},
};

const MAX_DECOMPRESSED: u64 = 1 << 30;

fn buffer_argument(args: &Args, index: usize, member: &str) -> Result<Vec<u8>, Flow> {
    match args.get(index) {
        LuaValue::Buffer(buffer) => Ok(buffer.to_vec()),
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        other => raise(format!("{member}: buffer expected, got {}", other.type_name())),
    }
}

fn algorithm_argument(engine: &Engine, args: &Args, index: usize, member: &str) -> Result<(), Flow> {
    let zstd = engine.reflection.enum_item("CompressionAlgorithm", "Zstd");
    match args.get(index) {
        LuaValue::UserData(ud) if ud.borrow::<LEnumItem>().is_ok_and(|item| Some(item.0) == zstd) => Ok(()),
        LuaValue::Nil => raise(format!("Argument {index} missing or nil")),
        _ => raise(format!("{member}: Enum.CompressionAlgorithm expected")),
    }
}

/// The decompressed size a Zstd frame holds, when it holds one within the limit.
fn decompressed_size(input: &[u8]) -> Option<u64> {
    zstd::zstd_safe::get_frame_content_size(input).ok().flatten().filter(|&size| size <= MAX_DECOMPRESSED)
}

pub fn install(engine: &Rc<Engine>, behaviors: &mut Behaviors) {
    let reflection = engine.reflection.clone();
    let Some(service) = reflection.class_id("EncodingService") else { return };
    let name = |member: &str| reflection.name_id(member);
    if let Some(n) = name("CompressBuffer") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let input = buffer_argument(args, 1, "CompressBuffer")?;
            algorithm_argument(engine, args, 2, "CompressBuffer")?;
            let level = match (args.get(3), args.number(3)) {
                (LuaValue::Nil, _) => 1.0,
                (_, Some(level)) => level,
                _ => return raise("CompressBuffer: compressionLevel must be a number"),
            };
            if !(-7.0..=22.0).contains(&level) || level.fract() != 0.0 {
                return raise("CompressBuffer: compressionLevel must be an integer from -7 to 22");
            }
            let compressed = zstd::bulk::compress(&input, level as i32).or_else(|error| raise(format!("CompressBuffer: {error}")))?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(compressed)?)]))
        }));
    }
    if let Some(n) = name("DecompressBuffer") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let input = buffer_argument(args, 1, "DecompressBuffer")?;
            algorithm_argument(engine, args, 2, "DecompressBuffer")?;
            let Some(size) = decompressed_size(&input) else {
                return raise("DecompressBuffer: the data does not hold a valid decompressed size");
            };
            let output = zstd::bulk::decompress(&input, size as usize).or_else(|error| raise(format!("DecompressBuffer: invalid data ({error})")))?;
            if output.len() as u64 != size {
                return raise("DecompressBuffer: the data does not match its decompressed size");
            }
            Ok(values([LuaValue::Buffer(lua.create_buffer(output)?)]))
        }));
    }
    if let Some(n) = name("GetDecompressedBufferSize") {
        behaviors.add_method(service, n, method(|_, engine, _, args| {
            let input = buffer_argument(args, 1, "GetDecompressedBufferSize")?;
            algorithm_argument(engine, args, 2, "GetDecompressedBufferSize")?;
            Ok(values([decompressed_size(&input).map_or(LuaValue::Nil, |size| LuaValue::Number(size as f64))]))
        }));
    }
    if let Some(n) = name("Base64Encode") {
        behaviors.add_method(service, n, method(|lua, _, _, args| {
            let input = buffer_argument(args, 1, "Base64Encode")?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(base64::encode(input))?)]))
        }));
    }
    if let Some(n) = name("Base64Decode") {
        behaviors.add_method(service, n, method(|lua, _, _, args| {
            let input = buffer_argument(args, 1, "Base64Decode")?;
            let decoded = base64::decode(input).or_else(|_| raise("Base64Decode: the input is not valid Base64"))?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(decoded)?)]))
        }));
    }
}
