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
        let (k, image) = ("1699715537", "6196665106");
        let mesh = 1699715537;
        let cases: Vec<(String, Option<AssetRef>)> = vec![
            (format!("rbxassetid://{k}"), at(mesh, None)),
            (format!("rbxassetid://{k}&version=1"), at(mesh, Some(1))),
            (format!("rbxassetid://{k}&version=2"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&Version=2"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&ver%73ion=2"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}& version =2"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&version=0"), at(mesh, None)),
            (format!("rbxassetid://{k}&version=-2"), at(mesh, None)),
            (format!("rbxassetid://{k}&version=abc&version=2"), at(mesh, None)),
            (format!("rbxassetid://{k}&version=2&version=abc&version=1"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&version=2&version=1"), at(mesh, Some(1))),
            (format!("rbxassetid://{k}&version=%32"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&version=+2"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&version=02"), at(mesh, Some(2))),
            (format!("rbxassetid://{k}&version=0x1"), at(mesh, Some(1))),
            (format!("rbxassetid://{k}&version=9223372036854775807"), at(mesh, Some(9223372036854775807))),
            (format!("rbxassetid://{k}&version=9223372036854775808"), at(mesh, None)),
            (format!("rbxassetid://{k}&version=2.0"), at(mesh, None)),
            (format!("rbxassetid://{k}&version"), at(mesh, None)),
            (format!("rbxassetid://{k}&foo=bar&version=2"), at(mesh, Some(2))),
            (format!("rbxassetid://{image}&id={k}"), at(mesh, None)),
            (format!("rbxassetid://{image}&ID={k}"), at(6196665106, None)),
            (format!("rbxassetid://{image}&id=abc"), at(6196665106, None)),
            (format!("rbxassetid://{k}&assetversionid=1"), None),
            (format!("rbxassetid://{k}&AssetVersionId=1"), None),
            (format!("rbxassetid://{k}#x"), None),
            (format!("rbxassetid://{k}&version=1#"), None),
            (format!("rbxassetid://{k}/&version=1"), None),
            (format!("rbxassetid://{k}%26version=1"), None),
            (format!("rbxassetid:// {k}&version=1"), at(mesh, Some(1))),
            (format!("rbxassetid://{k} &version=1"), at(mesh, Some(1))),
            (format!("rbxassetid://{k} "), at(mesh, None)),
            (format!("rbxassetid://0{k}"), at(mesh, None)),
            (format!("rbxassetid://{k}abc"), None),
            (format!("rbxassetid://{k}/"), None),
            (format!("rbxassetid://{k}?version=1"), None),
            (format!("rbxassetid://{k}?version=2"), None),
            (format!("rbxassetid:///{k}&version=1"), None),
            (format!("RBXASSETID://{k}"), None),
            (format!("RbxAssetId://{k}&version=1"), None),
            (format!(" rbxassetid://{k}"), None),
            (k.to_string(), None),
            (format!("www.roblox.com/asset/?id={k}"), None),
            (format!("http://www.roblox.com/asset/?id={k}"), at(mesh, None)),
            (format!("http://www.roblox.com/asset/?id={k}&version=2"), at(mesh, Some(2))),
            (format!("https://assetdelivery.roblox.com/v1/asset/?id={k}&version=1"), at(mesh, Some(1))),
            ("rbxassetid://0".to_string(), None),
            ("rbxassetid://".to_string(), None),
            (String::new(), None),
        ];
        for (uri, expected) in cases {
            assert_eq!(asset(&uri), expected, "{uri:?}");
        }
    }

    /// Each spelling as an Animator (LoadAnimation), KeyframeSequenceProvider and AnimationClipProvider
    /// read it in Studio 0.747's client (2026-11-24): asset 507766666 has 13 versions, its first 100
    /// keyframes long and its 13th 259, and every spelling of it that loaded loaded 259.
    #[test]
    fn animation_ids_read_as_a_client_reads_them() {
        let (n, m) = (507766666u64, 507777826u64);
        let starts = Err(invalid(STARTS_WITH_NO_NUMBER));
        let unknown = Err(invalid("<error: unknown AssetId protocol>"));
        let cases: Vec<(String, Result<Option<u64>, String>)> = vec![
            (format!("rbxassetid://{n}"), Ok(Some(n))),
            (format!("rbxassetid://{n}&version=1"), Ok(Some(n))),
            (format!("rbxassetid://{n}&version=999"), Ok(Some(n))),
            (format!("rbxassetid://{n}?version=1"), Ok(Some(n))),
            (format!("rbxassetid://{n}&id={m}"), Ok(Some(n))),
            (format!("rbxassetid://{n}/&version=1"), Ok(Some(n))),
            (format!("rbxassetid://{n}#x"), Ok(Some(n))),
            (format!("rbxassetid://{n}/"), Ok(Some(n))),
            (format!("rbxassetid://{n}abc"), Ok(Some(n))),
            (format!("rbxassetid://{n}&assetversionid=1"), Ok(Some(n))),
            (format!("rbxassetid://0{n}"), Ok(Some(n))),
            (format!("rbxassetid://{n} "), Ok(Some(n))),
            (format!("rbxassetid://{m}={n}"), Ok(Some(m))),
            ("rbxassetid://99999999999999999999999".to_string(), Ok(Some(9999999999999999999))),
            ("rbxassetid://18446744073709551615".to_string(), Ok(Some(1844674407370955161))),
            ("rbxassetid://".to_string(), Ok(None)),
            ("rbxassetid://0".to_string(), Ok(None)),
            (format!("rbxassetid:///{n}&version=1"), starts.clone()),
            (format!("rbxassetid:// {n}"), starts.clone()),
            (format!("rbxassetid://+{n}"), starts.clone()),
            ("rbxassetid://-5".to_string(), starts.clone()),
            ("rbxassetid://abc".to_string(), starts.clone()),
            (format!(" rbxassetid://{n}"), unknown.clone()),
            (format!("RBXASSETID://{n}"), unknown.clone()),
            (format!("Rbxassetid://{n}"), unknown.clone()),
            (n.to_string(), unknown.clone()),
            (format!("{n}&version=999"), unknown.clone()),
            (format!("www.roblox.com/asset/?id={n}"), unknown.clone()),
            (format!("rbxthumb://type=Asset&id={n}&w=150&h=150"), unknown.clone()),
            (format!("RBXHTTP://asset/?id={n}"), unknown.clone()),
            (" ".to_string(), unknown.clone()),
            ("\t".to_string(), unknown.clone()),
            (format!("http://www.roblox.com/asset/?id={n}"), Ok(Some(n))),
            (format!("HTTP://www.roblox.com/asset/?id={n}"), Ok(Some(n))),
            (format!("https://www.roblox.com/asset/?id={n}"), Ok(Some(n))),
            (format!("http://www.roblox.com/Asset/?ID={n}"), Ok(Some(n))),
            (format!("https://example.com/?id={n}"), Ok(Some(n))),
            (format!("rbxhttp://asset/?id={n}"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}#x"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}abc"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}&foo"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}/{m}"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?x={m}&id={n}"), Ok(Some(n))),
            (format!("http://x/?a={m}&id={n}&b"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}&version={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?id={n}&version={m}abc"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?id={n}&foo={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?id={n}&foo=bar&x={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?id={n}&={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?id={n}?v={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?ID={n}&Id={m}"), Ok(Some(m))),
            (format!("http://www.roblox.com/asset/?version={m}&id={n}"), Ok(Some(n))),
            (format!("http://www.roblox.com/asset/?id={n}&version=1"), Ok(Some(1))),
            (format!("http://www.roblox.com/asset/?id={n}&version=0"), Ok(None)),
            (format!("http://www.roblox.com/asset/?id={n}&version=-1"), Err(invalid(&format!("rbxassetid://{n}")))),
            (format!("http://www.roblox.com/asset/?id={n}&version=abc"), Err(invalid(&format!("rbxassetid://{n}")))),
            (format!("http://www.roblox.com/asset/?id={n}&foo=bar"), Err(invalid(&format!("rbxassetid://{n}")))),
            ("http://www.roblox.com/asset/?id=abc".to_string(), starts.clone()),
            (format!("http://www.roblox.com/asset/?id= {n}"), starts.clone()),
            (format!("https://assetdelivery.roblox.com/v1/assetId/{n}"), Err(invalid("<error: id not found in first 128 char for http AssetId>"))),
            ("rbxasset://animations/x.rbxm".to_string(), Err(invalid("<error: rbxasset:// file not found>"))),
            ("rbxtemp://1".to_string(), Err(invalid("<error: rbxtemp:// not allowed>"))),
            ("file:///C:/x.rbxm".to_string(), Err(invalid("<error: file:// not allowed>"))),
        ];
        for (uri, expected) in cases {
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
