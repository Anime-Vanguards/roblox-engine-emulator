//! How a Roblox client reads a content id that names an asset, measured in Studio 0.747's playing
//! client with its asset network on (2026-11-24; docs/engine.md, "Content ids"). It has two readers:
//!
//! - [`asset`], for images and meshes: `rbxassetid://N&version=V` is version V of asset N (how Roblox
//!   saves a pinned version), and a web URL's `version` pins one too;
//! - [`animation`], the Animator's, KeyframeSequenceProvider's and AnimationClipProvider's: the
//!   leading digits after `rbxassetid://`, never a version.
//!
//! What ree fetches is built from the numbers read here alone ([`AssetRef::worker_uri`]); no text of a
//! content id reaches a request.

/// An asset version a content id names; `version` None is the latest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetRef {
    pub id: u64,
    pub version: Option<u64>,
}

impl AssetRef {
    pub fn latest(id: u64) -> AssetRef {
        AssetRef { id, version: None }
    }

    /// What the asset worker is asked for (its `asset` command): `rbxassetid://<id>`, or
    /// `rbxassetid://<id>?version=<version>`, made of the two numbers.
    pub fn worker_uri(&self) -> String {
        match self.version {
            Some(version) => format!("rbxassetid://{}?version={version}", self.id),
            None => format!("rbxassetid://{}", self.id),
        }
    }

    /// Its file's name in a download cache: `<id>`, or `<id>-v<version>` for a pinned version.
    pub fn file_stem(&self) -> String {
        match self.version {
            Some(version) => format!("{}-v{version}", self.id),
            None => self.id.to_string(),
        }
    }
}

/// The asset version an image or mesh content id names, None for one a client loads nothing for.
///
/// - `rbxassetid://N` (the scheme in lower case, at the very start), with spaces around N at most;
///   `rbxassetid://N&k=v&…` reads the pairs after the `&` as [`pairs`] does: `version` pins one and a
///   lower-case `id` names another asset. Nothing loads for a `#` anywhere, or for anything but digits
///   before the first `&` (`N/`, `N?version=1`, `Nabc`).
/// - A web URL (any other text with `://`, such as `http://www.roblox.com/asset/?id=N`): the id after its
///   first `id=`, in any case, up to the next `&`; its query's `version` pins one as above.
/// - Nothing loads for text before the scheme (whitespace included), the scheme in other cases
///   (`RBXASSETID://N`), bare digits or a URL without a scheme (`www.roblox.com/asset/?id=N`).
pub fn asset(uri: &str) -> Option<AssetRef> {
    let uri = uri.trim_end();
    if let Some(rest) = uri.strip_prefix("rbxassetid://") {
        return pinned(rest);
    }
    if unknown_protocol(uri) {
        return None;
    }
    let at = uri.to_ascii_lowercase().find("id=")?;
    let id = uri[at + 3..].split('&').next().unwrap_or("").trim().parse::<u64>().ok()?;
    let query = uri.split_once('?').map_or("", |(_, query)| query);
    let query = query.split('#').next().unwrap_or("");
    let mut read = Pairs { id, version: None };
    for (key, value) in pairs(query) {
        if !read.version(&key, value.as_deref()) {
            break;
        }
    }
    Some(AssetRef { id, version: read.version })
}

/// Whether an image or mesh content id has no protocol a client knows, so it refuses it before any fetch: text before
/// the scheme (whitespace included), the asset scheme in other cases, or no scheme at all (bare digits,
/// `www.roblox.com/asset/?id=N`). Measured: AssetService:CreateEditableMeshAsync raises "Failed to load mesh <id>" for
/// these, and was refused for the place before it read any other.
pub fn unknown_protocol(uri: &str) -> bool {
    let uri = uri.trim_end();
    if uri.starts_with("rbxassetid://") {
        return false;
    }
    uri.get(..13).is_some_and(|head| head.eq_ignore_ascii_case("rbxassetid://")) || uri.starts_with(char::is_whitespace) || !uri.contains("://")
}

/// `rbxassetid://` and what follows it, for [`asset`].
fn pinned(rest: &str) -> Option<AssetRef> {
    if rest.contains('#') {
        return None;
    }
    let (id, query) = rest.split_once('&').unwrap_or((rest, ""));
    let mut read = Pairs { id: whole_id(id.trim())?, version: None };
    let mut reading = true;
    for (key, value) in pairs(query) {
        // (measured: an asset version's own id loads another asset; the worker fetches by asset and version)
        if key.eq_ignore_ascii_case("assetversionid") {
            return None;
        }
        if key == "id" {
            read.id = value.as_deref().and_then(whole_id).unwrap_or(read.id);
        } else if reading {
            reading = read.version(&key, value.as_deref());
        }
    }
    Some(AssetRef { id: read.id, version: read.version })
}

/// What a content id's pairs read so far.
struct Pairs {
    id: u64,
    version: Option<u64>,
}

impl Pairs {
    /// Reads the pair `key`=`value` if its key is `version` (in any case): an [`integer`] replaces
    /// the version before it, 0 or less being the latest. False when the value is no integer, which
    /// ends the reading of versions (what was read stays).
    fn version(&mut self, key: &str, value: Option<&str>) -> bool {
        if !key.eq_ignore_ascii_case("version") {
            return true;
        }
        match value.and_then(integer) {
            Some(version) => {
                self.version = u64::try_from(version).ok().filter(|&version| version > 0);
                true
            }
            None => false,
        }
    }
}

/// The `key=value` pairs of the text after a `?` or the first `&`, split at each `&` (empty ones
/// skipped), each side form-decoded (`+` a space, then percent escapes) and trimmed; a pair without
/// `=` has no value.
fn pairs(query: &str) -> impl Iterator<Item = (String, Option<String>)> + '_ {
    query.split('&').filter(|pair| !pair.is_empty()).map(|pair| match pair.split_once('=') {
        Some((key, value)) => (form_decode(key).trim().to_string(), Some(form_decode(value).trim().to_string())),
        None => (form_decode(pair).trim().to_string(), None),
    })
}

fn form_decode(text: &str) -> String {
    crate::percent_decode(&text.replace('+', " "))
}

/// An asset id written out whole: 1 to 20 decimal digits (leading zeros allowed), not 0.
fn whole_id(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>().ok().filter(|&id| id != 0)
}

/// A version's integer: an optional sign, then decimal digits (a leading zero is not octal) or `0x`
/// and hexadecimal digits, within i64.
fn integer(text: &str) -> Option<i64> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.strip_prefix('+').unwrap_or(text)),
    };
    let (radix, digits) = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => (16, hex),
        None => (10, digits),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    i64::from_str_radix(&format!("{sign}{digits}"), radix).ok()
}

/// What the Animator and the animation providers read an AnimationId as: `Ok(Some(id))` for the
/// latest version of asset `id`, `Ok(None)` for an id that names no asset to fetch (it loads nothing,
/// as an asset that is not found: `rbxassetid://` alone, asset 0), Err for one they raise about: the
/// error's text ([`INVALID`]). An empty id is the caller's to refuse (each says it in its own words).
///
/// - `rbxassetid://` (in lower case, at the very start) and the digits after it, up to 19; what
///   follows them is never read (`&version=V`, `?version=V`, `/`, `#`, `&id=M`). Anything but a digit
///   right after the scheme raises (`rbxassetid:// N`, `rbxassetid:///N`).
/// - A URL, `http://` or `https://` in any case or `rbxhttp://`: it needs an `id=` (any case) whose value
///   starts with a digit; the asset is the leading digits after the URL's last `=`. So
///   `?id=N&version=V` is asset V (the client asks for `asset?id=V`), and a last value that is no
///   number (`?id=N&foo=bar`) raises.
/// - `rbxasset://` (no file of Studio's: ree has none), `rbxtemp://` (an id this engine did not give
///   out) and `file://` raise; so does anything else (`RBXASSETID://N`, `N`, ` rbxassetid://N`).
pub fn animation(uri: &str) -> Result<Option<u64>, String> {
    if let Some(rest) = uri.strip_prefix("rbxassetid://") {
        if rest.is_empty() {
            return Ok(None);
        }
        return leading_id(rest).map(|id| (id != 0).then_some(id)).ok_or_else(|| invalid(STARTS_WITH_NO_NUMBER));
    }
    let web = ["http://", "https://"].iter().any(|scheme| uri.get(..scheme.len()).is_some_and(|head| head.eq_ignore_ascii_case(scheme)));
    if web || uri.starts_with("rbxhttp://") {
        let Some(at) = uri.to_ascii_lowercase().find("id=") else {
            return Err(invalid("<error: id not found in first 128 char for http AssetId>"));
        };
        let named = leading_id(&uri[at + 3..]).ok_or_else(|| invalid(STARTS_WITH_NO_NUMBER))?;
        let last = uri.rfind('=').expect("an id=");
        return leading_id(&uri[last + 1..]).map(|id| (id != 0).then_some(id)).ok_or_else(|| invalid(&format!("rbxassetid://{named}")));
    }
    let why = if uri.starts_with("rbxasset://") {
        "<error: rbxasset:// file not found>"
    } else if uri.starts_with("rbxtemp://") {
        "<error: rbxtemp:// not allowed>"
    } else if uri.starts_with("file://") {
        "<error: file:// not allowed>"
    } else {
        "<error: unknown AssetId protocol>"
    };
    Err(invalid(why))
}

/// The start of what the Animator and the animation providers raise for an id they cannot read.
pub const INVALID: &str = "Invalid animation id '";

const STARTS_WITH_NO_NUMBER: &str = "<error:  assetid does not start with a number>";

/// What KeyframeSequenceProvider:GetKeyframeSequenceAsync and AnimationClipProvider:GetAnimationClipAsync raise for an
/// empty id (LoadAnimation says it in its own words: [`EMPTY_FOR_LOAD`]).
pub fn null_id() -> String {
    invalid("<error: null id>")
}

/// What Animator:LoadAnimation raises for an Animation whose id is empty.
pub const EMPTY_FOR_LOAD: &str = "LoadAnimation requires the asset id to not be empty";

fn invalid(why: &str) -> String {
    format!("{INVALID}{why}': ")
}

/// The number of `text`'s leading decimal digits, up to 19 of them (a client reads no more); None
/// when it starts with something else.
fn leading_id(text: &str) -> Option<u64> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).take(19).count();
    if digits == 0 {
        return None;
    }
    text[..digits].parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: u64, version: Option<u64>) -> Option<AssetRef> {
        Some(AssetRef { id, version })
    }

    /// Each spelling as an image or mesh loads in Studio 0.747's client (meshes through
    /// AssetService:CreateMeshPartAsync, 2026-11-24, an asset whose only version is 1; images and the
    /// rest of the pinned forms as roblox-object-renderer measured them).
    #[test]
    fn image_and_mesh_ids_read_as_a_client_reads_them() {
        const K: &str = "1699715537";
        const IMAGE: &str = "6196665106";
        const MESH: u64 = 1699715537;
        const IMG: u64 = 6196665106;
        const CASES: &[(&str, Option<(u64, Option<u64>)>)] = &[
            ("rbxassetid://{k}", Some((MESH, None))),
            ("rbxassetid://{k}&version=1", Some((MESH, Some(1)))),
            ("rbxassetid://{k}&version=2", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&Version=2", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&ver%73ion=2", Some((MESH, Some(2)))),
            ("rbxassetid://{k}& version =2", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&version=0", Some((MESH, None))),
            ("rbxassetid://{k}&version=-2", Some((MESH, None))),
            ("rbxassetid://{k}&version=abc&version=2", Some((MESH, None))),
            ("rbxassetid://{k}&version=2&version=abc&version=1", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&version=2&version=1", Some((MESH, Some(1)))),
            ("rbxassetid://{k}&version=%32", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&version=+2", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&version=02", Some((MESH, Some(2)))),
            ("rbxassetid://{k}&version=0x1", Some((MESH, Some(1)))),
            ("rbxassetid://{k}&version=9223372036854775807", Some((MESH, Some(9223372036854775807)))),
            ("rbxassetid://{k}&version=9223372036854775808", Some((MESH, None))),
            ("rbxassetid://{k}&version=2.0", Some((MESH, None))),
            ("rbxassetid://{k}&version", Some((MESH, None))),
            ("rbxassetid://{k}&foo=bar&version=2", Some((MESH, Some(2)))),
            ("rbxassetid://{image}&id={k}", Some((MESH, None))),
            ("rbxassetid://{image}&ID={k}", Some((IMG, None))),
            ("rbxassetid://{image}&id=abc", Some((IMG, None))),
            ("rbxassetid://{k}&assetversionid=1", None),
            ("rbxassetid://{k}&AssetVersionId=1", None),
            ("rbxassetid://{k}#x", None),
            ("rbxassetid://{k}&version=1#", None),
            ("rbxassetid://{k}/&version=1", None),
            ("rbxassetid://{k}%26version=1", None),
            ("rbxassetid:// {k}&version=1", Some((MESH, Some(1)))),
            ("rbxassetid://{k} &version=1", Some((MESH, Some(1)))),
            ("rbxassetid://{k} ", Some((MESH, None))),
            ("rbxassetid://0{k}", Some((MESH, None))),
            ("rbxassetid://{k}abc", None),
            ("rbxassetid://{k}/", None),
            ("rbxassetid://{k}?version=1", None),
            ("rbxassetid://{k}?version=2", None),
            ("rbxassetid:///{k}&version=1", None),
            ("RBXASSETID://{k}", None),
            ("RbxAssetId://{k}&version=1", None),
            (" rbxassetid://{k}", None),
            ("{k}", None),
            ("www.roblox.com/asset/?id={k}", None),
            ("http://www.roblox.com/asset/?id={k}", Some((MESH, None))),
            ("http://www.roblox.com/asset/?id={k}&version=2", Some((MESH, Some(2)))),
            ("https://assetdelivery.roblox.com/v1/asset/?id={k}&version=1", Some((MESH, Some(1)))),
            ("rbxassetid://0", None),
            ("rbxassetid://", None),
            ("", None),
        ];
        for &(template, expected) in CASES {
            let uri = template.replace("{k}", K).replace("{image}", IMAGE);
            assert_eq!(asset(&uri), expected.and_then(|(id, v)| at(id, v)), "{uri:?}");
        }
    }

    /// Each spelling as an Animator (LoadAnimation), KeyframeSequenceProvider and AnimationClipProvider
    /// read it in Studio 0.747's client (2026-11-24): asset 507766666 has 13 versions, its first 100
    /// keyframes long and its 13th 259, and every spelling of it that loaded loaded 259.
    #[test]
    fn animation_ids_read_as_a_client_reads_them() {
        const N: u64 = 507766666;
        const M: u64 = 507777826;
        const N_STR: &str = "507766666";
        const M_STR: &str = "507777826";
        #[derive(Clone, Copy)]
        enum Want {
            Id(Option<u64>),
            Starts,
            Unknown,
            Invalid(&'static str),
        }
        const CASES: &[(&str, Want)] = &[
            ("rbxassetid://{n}", Want::Id(Some(N))),
            ("rbxassetid://{n}&version=1", Want::Id(Some(N))),
            ("rbxassetid://{n}&version=999", Want::Id(Some(N))),
            ("rbxassetid://{n}?version=1", Want::Id(Some(N))),
            ("rbxassetid://{n}&id={m}", Want::Id(Some(N))),
            ("rbxassetid://{n}/&version=1", Want::Id(Some(N))),
            ("rbxassetid://{n}#x", Want::Id(Some(N))),
            ("rbxassetid://{n}/", Want::Id(Some(N))),
            ("rbxassetid://{n}abc", Want::Id(Some(N))),
            ("rbxassetid://{n}&assetversionid=1", Want::Id(Some(N))),
            ("rbxassetid://0{n}", Want::Id(Some(N))),
            ("rbxassetid://{n} ", Want::Id(Some(N))),
            ("rbxassetid://{m}={n}", Want::Id(Some(M))),
            ("rbxassetid://99999999999999999999999", Want::Id(Some(9999999999999999999))),
            ("rbxassetid://18446744073709551615", Want::Id(Some(1844674407370955161))),
            ("rbxassetid://", Want::Id(None)),
            ("rbxassetid://0", Want::Id(None)),
            ("rbxassetid:///{n}&version=1", Want::Starts),
            ("rbxassetid:// {n}", Want::Starts),
            ("rbxassetid://+{n}", Want::Starts),
            ("rbxassetid://-5", Want::Starts),
            ("rbxassetid://abc", Want::Starts),
            (" rbxassetid://{n}", Want::Unknown),
            ("RBXASSETID://{n}", Want::Unknown),
            ("Rbxassetid://{n}", Want::Unknown),
            ("{n}", Want::Unknown),
            ("{n}&version=999", Want::Unknown),
            ("www.roblox.com/asset/?id={n}", Want::Unknown),
            ("rbxthumb://type=Asset&id={n}&w=150&h=150", Want::Unknown),
            ("RBXHTTP://asset/?id={n}", Want::Unknown),
            (" ", Want::Unknown),
            ("\t", Want::Unknown),
            ("http://www.roblox.com/asset/?id={n}", Want::Id(Some(N))),
            ("HTTP://www.roblox.com/asset/?id={n}", Want::Id(Some(N))),
            ("https://www.roblox.com/asset/?id={n}", Want::Id(Some(N))),
            ("http://www.roblox.com/Asset/?ID={n}", Want::Id(Some(N))),
            ("https://example.com/?id={n}", Want::Id(Some(N))),
            ("rbxhttp://asset/?id={n}", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}#x", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}abc", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}&foo", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}/{m}", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?x={m}&id={n}", Want::Id(Some(N))),
            ("http://x/?a={m}&id={n}&b", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}&version={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?id={n}&version={m}abc", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?id={n}&foo={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?id={n}&foo=bar&x={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?id={n}&={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?id={n}?v={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?ID={n}&Id={m}", Want::Id(Some(M))),
            ("http://www.roblox.com/asset/?version={m}&id={n}", Want::Id(Some(N))),
            ("http://www.roblox.com/asset/?id={n}&version=1", Want::Id(Some(1))),
            ("http://www.roblox.com/asset/?id={n}&version=0", Want::Id(None)),
            ("http://www.roblox.com/asset/?id={n}&version=-1", Want::Invalid("rbxassetid://{n}")),
            ("http://www.roblox.com/asset/?id={n}&version=abc", Want::Invalid("rbxassetid://{n}")),
            ("http://www.roblox.com/asset/?id={n}&foo=bar", Want::Invalid("rbxassetid://{n}")),
            ("http://www.roblox.com/asset/?id=abc", Want::Starts),
            ("http://www.roblox.com/asset/?id= {n}", Want::Starts),
            ("https://assetdelivery.roblox.com/v1/assetId/{n}", Want::Invalid("<error: id not found in first 128 char for http AssetId>")),
            ("rbxasset://animations/x.rbxm", Want::Invalid("<error: rbxasset:// file not found>")),
            ("rbxtemp://1", Want::Invalid("<error: rbxtemp:// not allowed>")),
            ("file:///C:/x.rbxm", Want::Invalid("<error: file:// not allowed>")),
        ];
        for &(template, want) in CASES {
            let uri = template.replace("{n}", N_STR).replace("{m}", M_STR);
            let expected = match want {
                Want::Id(id) => Ok(id),
                Want::Starts => Err(invalid(STARTS_WITH_NO_NUMBER)),
                Want::Unknown => Err(invalid("<error: unknown AssetId protocol>")),
                Want::Invalid(msg) => Err(invalid(&msg.replace("{n}", N_STR))),
            };
            assert_eq!(animation(&uri), expected, "{uri:?}");
        }
    }

    #[test]
    fn fetches_and_cache_files_are_made_of_the_numbers() {
        let pinned = asset("rbxassetid://12&version=3&x=\"y\"").unwrap();
        assert_eq!((pinned.worker_uri().as_str(), pinned.file_stem().as_str()), ("rbxassetid://12?version=3", "12-v3"));
        let latest = asset("http://www.roblox.com/asset/?id=12&foo=%2F..%2F").unwrap();
        assert_eq!((latest.worker_uri().as_str(), latest.file_stem().as_str()), ("rbxassetid://12", "12"));
    }
}
