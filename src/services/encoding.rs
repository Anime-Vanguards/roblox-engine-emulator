//! EncodingService: CompressBuffer / DecompressBuffer / GetDecompressedBufferSize (Zstd, the one CompressionAlgorithm)
//! and Base64Encode / Base64Decode, on a client and a server alike, as Roblox's EncodingService documentation gives them:
//! compressionLevel 1 by default, -7 to 22. The zstd library is the one Roblox's documentation names. Measured in a 0.747
//! playing client and server (tools/parity/probes/encoding.luau): every member answers a buffer; an argument that is no
//! buffer is "missing or nil"; Base64Decode refuses input that is not padded Base64; the errors' texts are the client's.
//! ComputeBufferHash / ComputeStringHash answer each HashAlgorithm's digest (digest.rs), as measured in the same client and
//! server (tools/parity/probes/encoding_hash.luau): see [`hash_algorithm`] and [`install`].

mod digest;

use std::rc::Rc;

use mlua::prelude::*;

use super::{Behaviors, FIRST, cast_strings, method, values};
use crate::{
    Engine,
    lua::{Args, Flow, convert, raise, types::LEnumItem},
    world::camera,
};

const MAX_DECOMPRESSED: u64 = 1 << 30;

/// (measured: a string, like nil, is "missing or nil")
fn buffer_argument(args: &Args, index: usize) -> Result<Vec<u8>, Flow> {
    match args.get(index) {
        LuaValue::Buffer(buffer) => Ok(buffer.to_vec()),
        _ => raise(format!("Argument {index} missing or nil")),
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

/// ComputeBufferHash and ComputeStringHash's algorithm, argument 2, cast before the input as a client casts it (measured, 0.747 client
/// and server): an item of HashAlgorithm, an item's exact name ("sha1" and "4" are refused), or a number cut toward zero to an item's
/// value (2.5 is Md5, -0.5 Blake2b; 5, -1, 2^32 + 4, NaN and infinities are refused); nil is "Argument 2 missing or nil" and anything
/// else "Unable to cast <what it is> to HashAlgorithm", named as a cast names it ("double", "bool", "CompressionAlgorithm", "null"
/// for an Enum: camera::cast_name). The answer is the item's name.
fn hash_algorithm(lua: &Lua, engine: &Engine, args: &Args) -> Result<Box<str>, Flow> {
    let value = args.get(2);
    if value.is_nil() {
        return raise("Argument 2 missing or nil");
    }
    let cut = match value {
        LuaValue::Number(number) => LuaValue::Number(number.trunc()),
        other => other.clone(),
    };
    match engine.reflection.enum_id("HashAlgorithm").and_then(|id| convert::coerce_enum(lua, engine, id, &cut)) {
        Some(item) => Ok(engine.reflection.enum_item_info(item).name.clone()),
        None => raise(format!("Unable to cast {} to HashAlgorithm", camera::cast_name(engine, value))),
    }
}

/// The digest an algorithm (HashAlgorithm's item name) answers for `input`: Blake2b's is 32 bytes (measured), the others their
/// standard lengths (Blake3 32, Md5 16, Sha1 20, Sha256 32). None for an item this does not know.
fn digest_of(algorithm: &str, input: &[u8]) -> Option<Vec<u8>> {
    Some(match algorithm {
        "Blake2b" => digest::blake2b(input, 32),
        "Blake3" => digest::blake3(input).to_vec(),
        "Md5" => digest::md5(input).to_vec(),
        "Sha1" => digest::sha1(input).to_vec(),
        "Sha256" => digest::sha256(input).to_vec(),
        _ => return None,
    })
}

/// The digest, or nil with Output `missing` for an algorithm [`digest_of`] does not know (an item a newer API dump adds)
fn hashed(lua: &Lua, engine: &Engine, member: &str, algorithm: &str, input: &[u8]) -> Option<Vec<u8>> {
    let digest = digest_of(algorithm, input);
    if digest.is_none() {
        engine.missing(Some(lua), "EncodingService", &format!("{member} with HashAlgorithm.{algorithm}"));
    }
    digest
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
            let input = buffer_argument(args, 1)?;
            algorithm_argument(engine, args, 2, "CompressBuffer")?;
            let level = match (args.get(3), args.number(3)) {
                (LuaValue::Nil, _) => 1.0,
                (_, Some(level)) => level,
                _ => return raise("CompressBuffer: compressionLevel must be a number"),
            };
            // (an int argument: a fraction is cut off; measured: 23 is refused)
            let level = level.trunc();
            if !(-7.0..=22.0).contains(&level) {
                return raise("CompressBuffer: unsupported compression level for Zstd");
            }
            let compressed = zstd::bulk::compress(&input, level as i32).or_else(|error| raise(format!("CompressBuffer: {error}")))?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(compressed)?)]))
        }));
    }
    if let Some(n) = name("DecompressBuffer") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let input = buffer_argument(args, 1)?;
            algorithm_argument(engine, args, 2, "DecompressBuffer")?;
            // (measured: data that is no Zstd frame; a frame without its size, or one that does not hold what it says, is
            // refused the same way here)
            const FAILED: &str = "DecompressBuffer: decompression failed - Unspecified error code";
            let Some(size) = decompressed_size(&input) else { return raise(FAILED) };
            let output = zstd::bulk::decompress(&input, size as usize).or_else(|_| raise(FAILED))?;
            if output.len() as u64 != size {
                return raise(FAILED);
            }
            Ok(values([LuaValue::Buffer(lua.create_buffer(output)?)]))
        }));
    }
    if let Some(n) = name("GetDecompressedBufferSize") {
        behaviors.add_method(service, n, method(|_, engine, _, args| {
            let input = buffer_argument(args, 1)?;
            algorithm_argument(engine, args, 2, "GetDecompressedBufferSize")?;
            Ok(values([decompressed_size(&input).map_or(LuaValue::Nil, |size| LuaValue::Number(size as f64))]))
        }));
    }
    if let Some(n) = name("Base64Encode") {
        behaviors.add_method(service, n, method(|lua, _, _, args| {
            let input = buffer_argument(args, 1)?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(base64::encode(input))?)]))
        }));
    }
    // (measured: a new buffer each call; no input is too large: a client hashed a 1 GiB buffer and a 200 MiB string)
    if let Some(n) = name("ComputeBufferHash") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let algorithm = hash_algorithm(lua, engine, args)?;
            let input = buffer_argument(args, 1)?;
            let digest = hashed(lua, engine, "ComputeBufferHash", &algorithm, &input);
            Ok(values([digest.map_or(Ok(LuaValue::Nil), |digest| lua.create_buffer(digest).map(LuaValue::Buffer))?]))
        }));
    }
    // (measured: the digest's bytes as a string; an input that is no string is given the text a string parameter casts it to,
    // with the parameter's warning: "argument #1 expects a string, but boolean was passed", none for a number)
    if let Some(n) = name("ComputeStringHash") {
        behaviors.add_method(service, n, method(|lua, engine, _, args| {
            let algorithm = hash_algorithm(lua, engine, args)?;
            let cast = cast_strings(lua, engine, args, FIRST)?;
            let input = match cast.as_ref().unwrap_or(args).get(1) {
                LuaValue::String(text) => text.as_bytes().to_vec(),
                _ => return raise("Argument 1 missing or nil"),
            };
            let digest = hashed(lua, engine, "ComputeStringHash", &algorithm, &input);
            Ok(values([digest.map_or(Ok(LuaValue::Nil), |digest| lua.create_string(digest).map(LuaValue::String))?]))
        }));
    }
    if let Some(n) = name("Base64Decode") {
        behaviors.add_method(service, n, method(|lua, _, _, args| {
            let input = buffer_argument(args, 1)?;
            // (measured: unpadded input is refused)
            const INVALID: &str = "Base64Decode: input is not a base64 string";
            if input.len() % 4 != 0 {
                return raise(INVALID);
            }
            let decoded = base64::decode(input).or_else(|_| raise(INVALID))?;
            Ok(values([LuaValue::Buffer(lua.create_buffer(decoded)?)]))
        }));
    }
}
