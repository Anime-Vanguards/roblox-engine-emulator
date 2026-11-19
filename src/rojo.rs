//! Rojo project files (`*.project.json`): the tree a project describes, as the mounts it comes to
//! ([`crate::project::Mount`]: `--mount Service/Name=<path>` and `--instance Service/Name=<ClassName>`). `ree run` and
//! `ree record` mount one with `--rojo <file>` ([`Tree`], [`crate::launch`]). The module is shared with
//! roblox-ui-renderer's native commands, and its rules are those of that tool's Node CLI (cli/rojo.mjs):
//!
//! - A DataModel tree (`"$className": "DataModel"`): each node with a `$path` mounts that directory or file at its
//!   instance path (ReplicatedStorage/Packages, StarterPlayer/StarterPlayerScripts/...); `$className` nodes without
//!   a `$path` below a service become instances of that class.
//! - Any other tree (a plugin or model project) is mounted at `ReplicatedStorage/<project name>`, or where
//!   `--rojo-at Service/Name` says.
//! - `$path` to another `*.project.json` nests that project's tree; `{ "optional": path }` is skipped when missing;
//!   `$properties`, `$attributes` and `$ignoreUnknownInstances` are ignored.
//!
//! What a mounted file or folder becomes (scripts by suffix, `init` files, `.model.json`, `.rbxm` / `.rbxmx`, JSON
//! modules, a folder's own `default.project.json`) is the mount's business ([`crate::project::mount_at`]), the same for a
//! file a project maps as for one below a `--roots` folder.
use std::path::{Component, Path, PathBuf};

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// The file name a folder's Rojo project has when it has several
pub const DEFAULT_PROJECT_FILE: &str = "default.project.json";

/// The folders Wally installs packages into (realms shared, server and dev)
const WALLY_FOLDERS: [&str; 3] = ["Packages", "ServerPackages", "DevPackages"];

/// The *.project.json files in `dir`, by name
pub fn project_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| is_project_file(&entry.file_name().to_string_lossy()) && entry.path().is_file())
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

/// The Rojo project a folder names by itself: its default.project.json, else its only *.project.json. `Err` lists
/// the folder's project files when there are several and none is default.project.json.
pub fn project_file_in(dir: &Path) -> Result<Option<PathBuf>, Vec<PathBuf>> {
    let default = dir.join(DEFAULT_PROJECT_FILE);
    if default.is_file() {
        return Ok(Some(default));
    }
    let mut files = project_files_in(dir);
    match files.len() {
        0 => Ok(None),
        1 => Ok(files.pop()),
        _ => Err(files),
    }
}

/// What to say about a folder with several project files and no default.project.json
pub fn several_projects(files: &[PathBuf]) -> String {
    let names: Vec<String> = files.iter().filter_map(|file| file.file_name()).map(|name| name.to_string_lossy().into_owned()).collect();
    let dir = files.first().and_then(|file| file.parent()).map(Path::display);
    format!("{} has several Rojo projects ({}), so none was mounted: pass --rojo <file> to mount one", dir.map(|dir| dir.to_string()).unwrap_or_default(), names.join(", "))
}

/// `wally install` when `source` is a Wally package folder of a project beside a wally.toml
pub fn wally_advice(project_dir: &Path, source: &Path) -> Option<String> {
    let name = source.file_name()?.to_string_lossy();
    (WALLY_FOLDERS.contains(&name.as_ref()) && project_dir.join("wally.toml").is_file()).then(|| format!("run `wally install` in {}", project_dir.display()))
}

/// `a/b` as an instance path (`a.b`)
pub fn dotted(path: &str) -> String {
    path.replace('/', ".")
}

/// `path` relative to `dir` when it is inside it
fn shown(path: &Path, dir: &Path) -> String {
    path.strip_prefix(dir).ok().filter(|rest| !rest.as_os_str().is_empty()).unwrap_or(path).display().to_string()
}

/// What to say about a `$path` that does not exist (and, for a Wally folder, how to make it)
pub fn missing_path(file: &Path, at: &str, source: &Path) -> String {
    let dir = file.parent().unwrap_or(Path::new(""));
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let advice = wally_advice(dir, source).map(|advice| format!(": {advice}")).unwrap_or_default();
    format!("{name} maps {} to {}, which doesn't exist{advice}", dotted(at), shown(source, dir))
}

/// A JSON value with its objects' keys in file order (serde_json's own map sorts them, and the mount order is the
/// order instances are created in, which `GetChildren` shows)
enum Node {
    Object(Vec<(String, Node)>),
    String(String),
    Other,
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Node, D::Error> {
        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("JSON")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut entries: Vec<(String, Node)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Node>()? {
                    // JSON.parse keeps a repeated key at its first place with the last value
                    match entries.iter_mut().find(|(name, _)| *name == key) {
                        Some(entry) => entry.1 = value,
                        None => entries.push((key, value)),
                    }
                }
                Ok(Node::Object(entries))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
                while seq.next_element::<Node>()?.is_some() {}
                Ok(Node::Other)
            }
            fn visit_str<E>(self, value: &str) -> Result<Node, E> {
                Ok(Node::String(value.to_string()))
            }
            fn visit_string<E>(self, value: String) -> Result<Node, E> {
                Ok(Node::String(value))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Node, E> {
                Ok(Node::Other)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Node, E> {
                Ok(Node::Other)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Node, E> {
                Ok(Node::Other)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Node, E> {
                Ok(Node::Other)
            }
            fn visit_unit<E>(self) -> Result<Node, E> {
                Ok(Node::Other)
            }
        }
        deserializer.deserialize_any(NodeVisitor)
    }
}

impl Node {
    fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Object(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    fn string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Node::String(value)) => Some(value),
            _ => None,
        }
    }

    /// The children a tree node names: its keys without `$`, with object values
    fn children(&self) -> impl Iterator<Item = (&String, &Node)> {
        let entries: &[(String, Node)] = match self {
            Node::Object(entries) => entries,
            _ => &[],
        };
        entries.iter().filter(|(name, value)| !name.starts_with('$') && matches!(value, Node::Object(_))).map(|(name, value)| (name, value))
    }
}

/// One mount of a project: `--mount path=value` (a directory or file) or `--instance path=value` (a class name)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub instance: bool,
    /// The instance path below the DataModel, `/`-separated, starting with a service
    pub path: String,
    pub value: String,
}

impl Mount {
    /// The engine arguments of this mount
    pub fn arguments(&self) -> [String; 2] {
        [if self.instance { "--instance" } else { "--mount" }.into(), format!("{}={}", self.path, self.value)]
    }

    /// The mount as the engine applies it ([`crate::project::mount_at`]), its instance names as the project spells them
    pub fn engine_mount(&self) -> crate::project::Mount {
        use crate::project::MountTarget;
        let target = if self.instance { MountTarget::Class(self.value.clone()) } else { MountTarget::Path(self.value.clone()) };
        crate::project::Mount { segments: self.path.split('/').map(str::to_string).collect(), target }
    }
}

/// Whether `file` names a Rojo project (`*.project.json`)
pub fn is_project_file(file: &str) -> bool {
    file.to_ascii_lowercase().ends_with(".project.json")
}

/// `path` made absolute against the current directory with `.` and `..` removed, as Node's `path.resolve` does
pub fn resolve(base: &Path, path: &str) -> PathBuf {
    let joined = base.join(path);
    let joined = std::path::absolute(&joined).unwrap_or(joined);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // (never above the root, as path.resolve)
                if !matches!(out.components().next_back(), Some(Component::RootDir | Component::Prefix(_)) | None) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn read_project(file: &Path) -> Result<Node, String> {
    // (a run prepared ahead checks the project files it read: record/prepare.rs)
    crate::files::note(file);
    let text = std::fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    serde_json::from_str::<Node>(&text).map_err(|error| format!("{}: {error}", file.display()))
}

/// A top-level name of a DataModel tree that can be a service (the engine checks the rest)
fn is_service(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

/// The mounts of a project file, in the order cli/rojo.mjs gives them. `at` is `--rojo-at Service/Name` (for a tree
/// that is not a DataModel). A non-optional `$path` that is missing is skipped and reported in `warnings`
/// ([`missing_path`]; the caller adds its name in front).
pub fn mounts(project_file: &Path, at: Option<&str>, warnings: &mut Vec<String>) -> Result<Vec<Mount>, String> {
    mounts_with(project_file, at, warnings, &mut |path| Ok(path))
}

/// [`mounts`], each `$path` first handed to `source`, which can stand another file or folder in for it
/// (roblox-ui-renderer's `--git-ref`: the path as it is at a commit, extracted there)
pub fn mounts_with(
    project_file: &Path,
    at: Option<&str>,
    warnings: &mut Vec<String>,
    source: &mut dyn FnMut(PathBuf) -> Result<PathBuf, String>,
) -> Result<Vec<Mount>, String> {
    let walk = walk(project_file, at, source)?;
    warnings.extend(walk.missing.iter().map(|mount| missing_path(&walk.file, &mount.path, Path::new(&mount.value))));
    Ok(walk.mounts)
}

/// Every `$path` of a project file, also the ones that do not exist: (instance path `a/b`, file or folder, exists)
pub fn mappings(project_file: &Path) -> Result<Vec<(String, PathBuf, bool)>, String> {
    let mut unchanged = |path| Ok(path);
    let walk = walk(project_file, None, &mut unchanged)?;
    let present = walk.mounts.into_iter().filter(|mount| !mount.instance).map(|mount| (mount.path, PathBuf::from(mount.value), true));
    Ok(present.chain(walk.missing.into_iter().map(|mount| (mount.path, PathBuf::from(mount.value), false))).collect())
}

/// The instance paths a project's files and folders mount at, as a note lists them
pub fn mounted_list(mounts: &[Mount]) -> String {
    let places: Vec<String> = mounts.iter().filter(|mount| !mount.instance).map(|mount| dotted(&mount.path)).collect();
    if places.is_empty() { "nothing".to_string() } else { places.join(", ") }
}

fn walk<'a>(project_file: &Path, at: Option<&str>, source: &'a mut dyn FnMut(PathBuf) -> Result<PathBuf, String>) -> Result<Walk<'a>, String> {
    let file = resolve(Path::new(""), &project_file.to_string_lossy());
    let project = read_project(&file)?;
    let Some(tree @ Node::Object(_)) = project.get("tree") else {
        return Err(format!("{}: not a Rojo project (no \"tree\")", file.display()));
    };
    let mut walk = Walk { file: file.clone(), mounts: Vec::new(), missing: Vec::new(), source };
    let base = file.parent().map(Path::to_path_buf).unwrap_or_default();
    if tree.string("$className") == Some("DataModel") {
        for (name, child) in tree.children() {
            if !is_service(name) {
                return Err(format!("{}: {name} at the top of a DataModel tree is not a service", file.display()));
            }
            walk.visit(child, &mut vec![name.clone()], &base, 0)?;
        }
    } else {
        let mut place: Vec<String> = match at {
            Some(at) => at.split(['/', '\\']).filter(|part| !part.is_empty()).map(str::to_string).collect(),
            None => vec!["ReplicatedStorage".into(), project.string("name").filter(|name| !name.is_empty()).unwrap_or("Project").to_string()],
        };
        if place.len() < 2 {
            return Err(format!("--rojo-at {}: needs Service/Name", at.unwrap_or_default()));
        }
        walk.visit(tree, &mut place, &base, 0)?;
    }
    Ok(walk)
}

struct Walk<'a> {
    file: PathBuf,
    mounts: Vec<Mount>,
    /// The non-optional `$path`s that do not exist
    missing: Vec<Mount>,
    /// What each `$path` stands for ([`mounts_with`])
    source: &'a mut dyn FnMut(PathBuf) -> Result<PathBuf, String>,
}

impl Walk<'_> {
    fn visit(&mut self, node: &Node, segments: &mut Vec<String>, base: &Path, depth: u32) -> Result<(), String> {
        if depth > 32 {
            return Err(format!("{}: project nesting too deep", self.file.display()));
        }
        let (path_value, optional) = match node.get("$path") {
            Some(Node::String(path)) => (Some(path.as_str()), false),
            Some(optional @ Node::Object(_)) => (optional.string("optional"), true),
            _ => (None, false),
        };
        if let Some(path_value) = path_value {
            let source = (self.source)(resolve(base, path_value))?;
            let source_text = source.to_string_lossy().into_owned();
            // (found or missing, as a run prepared ahead checks it again)
            crate::files::note(&source);
            if !source.exists() {
                if !optional {
                    let at = if segments.is_empty() { "tree".to_string() } else { segments.join("/") };
                    self.missing.push(Mount { instance: false, path: at, value: source_text });
                }
            } else if is_project_file(&source_text) {
                if let Some(nested_tree @ Node::Object(_)) = read_project(&source)?.get("tree") {
                    let nested_base = source.parent().map(Path::to_path_buf).unwrap_or_default();
                    self.visit(nested_tree, segments, &nested_base, depth + 1)?;
                }
            } else if !segments.is_empty() {
                self.mounts.push(Mount { instance: false, path: segments.join("/"), value: source_text });
            }
        } else if let (Some(class), true) = (node.string("$className"), segments.len() > 1) {
            self.mounts.push(Mount { instance: true, path: segments.join("/"), value: class.to_string() });
        }
        for (name, child) in node.children() {
            segments.push(name.clone());
            let visited = self.visit(child, segments, base, depth);
            segments.pop();
            visited?;
        }
        Ok(())
    }
}

/// A Rojo project file read for a run (`--rojo <file>`, [`crate::launch::prepare`]): the mounts its tree comes to, in
/// file order, as the engine applies them, and what to say about the `$path`s it maps that don't exist. The mount
/// ([`crate::mount::begin_project`]) applies them where `--roots` folders mount.
#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    /// The project file, absolute
    pub file: PathBuf,
    pub mounts: Vec<crate::project::Mount>,
    /// One line per non-optional `$path` that doesn't exist ([`missing_path`]); an optional one is left out quietly
    pub missing: Vec<String>,
}

impl Tree {
    /// The tree of project file `file` (relative to the current directory); `at` places a tree that is not a DataModel
    /// (`--rojo-at Service/Name`; else ReplicatedStorage/<project name>)
    pub fn read(file: &Path, at: Option<&str>) -> Result<Tree, String> {
        let file = resolve(Path::new(""), &file.to_string_lossy());
        if !file.is_file() {
            crate::files::note(&file);
            return Err(format!("Rojo project {}: no such file", file.display()));
        }
        let mut missing = Vec::new();
        let mounts = mounts(&file, at, &mut missing)?;
        Ok(Tree { mounts: mounts.iter().map(Mount::engine_mount).collect(), file, missing })
    }

    /// The project file's folder: the folder content paths are read from unless `--project` names one, and the place's
    /// root wherever ree looks a place up by its root folder ([`crate::mount::ProjectSpec::place_roots`])
    pub fn folder(&self) -> String {
        self.file.parent().unwrap_or(Path::new("")).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ree-rojo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A DataModel tree mounts its `$path`s at their instance paths in file order, `$className` nodes without a path
    /// as instances, nested projects in place; missing paths are skipped (reported unless optional)
    #[test]
    fn a_datamodel_tree_becomes_mounts_in_file_order() {
        let dir = temp_dir("datamodel");
        write(&dir.join("src/shared/init.luau"), "return {}");
        write(&dir.join("Packages/Fusion.luau"), "return {}");
        write(&dir.join("nested/inner.project.json"), r#"{"name":"inner","tree":{"$path":"lib"}}"#);
        write(&dir.join("nested/lib/a.luau"), "return 1");
        write(
            &dir.join("default.project.json"),
            r#"{"name":"game","tree":{"$className":"DataModel",
                "ReplicatedStorage":{"$className":"ReplicatedStorage","Shared":{"$path":"src/shared"},"Packages":{"$path":"Packages"},
                    "Inner":{"$path":"nested/inner.project.json"},"Remotes":{"$className":"Folder"}},
                "ServerStorage":{"Gone":{"$path":"missing"},"Maybe":{"$path":{"optional":"missing-too"}}}}}"#,
        );
        let mut warnings = Vec::new();
        let mounts = mounts(&dir.join("default.project.json"), None, &mut warnings).unwrap();
        let listed: Vec<(bool, &str, String)> = mounts
            .iter()
            .map(|mount| (mount.instance, mount.path.as_str(), if mount.instance { mount.value.clone() } else { Path::new(&mount.value).strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/") }))
            .collect();
        assert_eq!(
            listed,
            vec![
                (false, "ReplicatedStorage/Shared", "src/shared".into()),
                (false, "ReplicatedStorage/Packages", "Packages".into()),
                (false, "ReplicatedStorage/Inner", "nested/lib".into()),
                (true, "ReplicatedStorage/Remotes", "Folder".into()),
            ]
        );
        assert_eq!(warnings, ["default.project.json maps ServerStorage.Gone to missing, which doesn't exist"]);
        assert_eq!(mounted_list(&mounts), "ReplicatedStorage.Shared, ReplicatedStorage.Packages, ReplicatedStorage.Inner");
        assert_eq!(mounts[3].arguments(), [String::from("--instance"), "ReplicatedStorage/Remotes=Folder".into()]);
        // Every $path, also the missing ones (optional too), with whether it exists
        let mapped: Vec<(String, bool)> = mappings(&dir.join("default.project.json")).unwrap().into_iter().map(|(at, _, exists)| (at, exists)).collect();
        assert_eq!(mapped.iter().filter(|(_, exists)| !exists).map(|(at, _)| at.as_str()).collect::<Vec<_>>(), ["ServerStorage/Gone"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tree that is not a DataModel mounts at ReplicatedStorage/<name>, or at --rojo-at
    #[test]
    fn a_plugin_tree_mounts_under_its_name_or_rojo_at() {
        let dir = temp_dir("plugin");
        write(&dir.join("src/init.luau"), "return {}");
        write(&dir.join("plugin.project.json"), r#"{"name":"MyPlugin","tree":{"$path":"src"}}"#);
        let file = dir.join("plugin.project.json");
        let mut warnings = Vec::new();
        assert_eq!(mounts(&file, None, &mut warnings).unwrap()[0].path, "ReplicatedStorage/MyPlugin");
        assert_eq!(mounts(&file, Some("ServerStorage/Tools"), &mut warnings).unwrap()[0].path, "ServerStorage/Tools");
        assert!(mounts(&file, Some("ServerStorage"), &mut warnings).unwrap_err().contains("needs Service/Name"));
        write(&dir.join("bad.project.json"), r#"{"name":"x","tree":{"$className":"DataModel","lower":{}}}"#);
        assert!(mounts(&dir.join("bad.project.json"), None, &mut warnings).unwrap_err().contains("is not a service"));
        write(&dir.join("treeless.project.json"), r#"{"name":"x"}"#);
        assert!(mounts(&dir.join("treeless.project.json"), None, &mut warnings).unwrap_err().contains("not a Rojo project (no \"tree\")"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder's project file is its default.project.json, else its only *.project.json; several others are named
    #[test]
    fn a_folder_names_its_project_file_by_itself_unless_it_has_several() {
        let dir = temp_dir("project-file");
        assert_eq!(project_file_in(&dir), Ok(None));
        write(&dir.join("place.project.json"), "{}");
        assert_eq!(project_file_in(&dir), Ok(Some(dir.join("place.project.json"))));
        write(&dir.join("dev.project.json"), "{}");
        assert_eq!(project_file_in(&dir), Err(vec![dir.join("dev.project.json"), dir.join("place.project.json")]));
        assert_eq!(
            several_projects(&project_files_in(&dir)),
            format!("{} has several Rojo projects (dev.project.json, place.project.json), so none was mounted: pass --rojo <file> to mount one", dir.display())
        );
        write(&dir.join("default.project.json"), "{}");
        assert_eq!(project_file_in(&dir), Ok(Some(dir.join("default.project.json"))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Wally folder that doesn't exist yet says how to make it
    #[test]
    fn a_missing_wally_folder_says_to_run_wally_install() {
        let dir = temp_dir("wally");
        write(&dir.join("game.project.json"), r#"{"name":"g","tree":{"$className":"DataModel","ReplicatedStorage":{"Packages":{"$path":"Packages"}}}}"#);
        let mut warnings = Vec::new();
        mounts(&dir.join("game.project.json"), None, &mut warnings).unwrap();
        assert_eq!(warnings, ["game.project.json maps ReplicatedStorage.Packages to Packages, which doesn't exist"]);
        write(&dir.join("wally.toml"), "[package]");
        warnings.clear();
        mounts(&dir.join("game.project.json"), None, &mut warnings).unwrap();
        assert_eq!(warnings, [format!("game.project.json maps ReplicatedStorage.Packages to Packages, which doesn't exist: run `wally install` in {}", dir.display())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A run's tree: the engine's mounts (absolute paths, the project's instance names), the missing paths' lines, the
    /// project's folder; a file that isn't there is refused
    #[test]
    fn a_run_reads_a_project_file_into_engine_mounts() {
        let dir = temp_dir("tree");
        write(&dir.join("src/server/main.server.luau"), "print('server')");
        write(&dir.join("game.project.json"), r#"{"name":"g","tree":{"$className":"DataModel","ServerScriptService":{"Server":{"$path":"src/server"},"Gone":{"$path":"gone"}},"Workspace":{"Spawn":{"$className":"SpawnLocation"}}}}"#);
        let tree = Tree::read(&dir.join("game.project.json"), None).unwrap();
        use crate::project::{Mount as EngineMount, MountTarget};
        assert_eq!(
            tree.mounts,
            vec![
                EngineMount { segments: vec!["ServerScriptService".into(), "Server".into()], target: MountTarget::Path(dir.join("src").join("server").to_string_lossy().into_owned()) },
                EngineMount { segments: vec!["Workspace".into(), "Spawn".into()], target: MountTarget::Class("SpawnLocation".into()) },
            ]
        );
        assert_eq!(tree.missing, ["game.project.json maps ServerScriptService.Gone to gone, which doesn't exist"]);
        assert_eq!(Path::new(&tree.folder()), dir.as_path());
        assert!(Tree::read(&dir.join("none.project.json"), None).unwrap_err().contains("no such file"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
