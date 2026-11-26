//! The place's Studio-only instances (project.rs `materialize`: the placeholders a Studio hierarchy snapshot adds below an
//! instance once something looks at its children) stay lazy on both sides of a loopback pair. The join copies what the
//! server has made, each copy keeping its hierarchy node and whether its Studio children were made; each side then makes
//! a copy's Studio children on its own when its scripts look, from the same snapshot, so the two sides' are the same
//! instances. The two are paired when a message first names one: by the instance it was made under (paired itself, or
//! paired the same way) and its node in the snapshot, the other side making that instance's Studio children first if it
//! has not. A server instance made this way under a sent one is replicated (its changes go to the client) and counts as
//! sent; one the client cannot make itself (a plan's) is sent as a script's new instance is.
//!
//! Manifest bundles (project::model_assets) stay lazy too, but only the server imports them (one source, one decode, one set
//! of instances): the join copies a waiting bundle's placeholder (or the folder its members go into) as it is, and the
//! client's copy notes that bundles wait for it. When the client finds, touches or looks inside the copy, the server's
//! instance imports them ([`import_on_server`]); when either side's import ends, the client's copy takes what it made at
//! once ([`bundle_imported`], replicate.rs `copy_import`), as the join would have copied it. A bundle nothing looked at is
//! never fetched or made on either side.

use std::rc::Rc;

use rustc_hash::FxHashMap;

use super::{Endpoint, Side, bridge::Msg, replicate::Change, wire::Wire};
use crate::{
    Engine, InstanceId,
    dom::{StudioNode, flags},
};

/// What one side made of the snapshot after the join
#[derive(Default)]
pub struct Lazy {
    /// A made instance: the instance it was made under, its node
    origin: FxHashMap<InstanceId, (InstanceId, u32)>,
    /// (the instance made under, the node) to the made instance
    made: FxHashMap<(InstanceId, u32), InstanceId>,
}

impl Lazy {
    /// How many instances this side made of the snapshot since the join
    pub fn made(&self) -> usize {
        self.made.len()
    }
}

/// After `id`'s children were made (lua/instance.rs `materialize`) on a linked engine: the children past `before` are new.
/// A server's new children of a replicated instance replicate: a Studio-only one under a sent instance is the client's to
/// make (lazily paired), any other is sent as a new instance.
pub fn materialized(engine: &Engine, id: InstanceId, before: usize) {
    let Some(endpoint) = engine.loopback.borrow().clone() else { return };
    let mut state = engine.state.borrow_mut();
    let Some(instance) = state.dom.get(id) else { return };
    if instance.children.len() <= before {
        return;
    }
    let made: Vec<InstanceId> = instance.children.iter().skip(before).copied().collect();
    let parent_flags = instance.flags;
    let server = endpoint.side == Side::Server;
    if server && parent_flags & flags::REPLICATED == 0 {
        return;
    }
    let shared = !server || parent_flags & flags::SENT != 0;
    let mut lazy = endpoint.lazy.borrow_mut();
    for child in made {
        let node = match state.dom.at(child).studio_node {
            StudioNode::Node(node) if state.dom.at(child).flags & flags::STUDIO_ONLY != 0 => Some(node),
            _ => None,
        };
        match (shared, node) {
            (true, Some(node)) => {
                lazy.origin.insert(child, (id, node));
                lazy.made.insert((id, node), child);
                if server {
                    state.dom.at_mut(child).flags |= flags::REPLICATED | flags::SENT;
                }
            }
            // (a bundle's instance the client has a copy of already: copy_import sent it)
            _ if server && state.dom.at(child).flags & flags::SENT != 0 => {}
            _ if server => {
                // (a parent not sent yet takes it along; otherwise it goes as a new instance)
                super::replicate::mark(&mut state, child, true, &[]);
                if parent_flags & flags::SENT != 0
                    && let Some(log) = state.loopback_log.as_mut()
                {
                    log.push(Change::Moved { id: child, parent: Some(id), inside: true, was: false });
                }
            }
            _ => {}
        }
    }
}

/// The client's instance for the server's `id`: the paired one, or the one the client makes of the snapshot where the
/// server made `id` (paired now). None for an instance the client has no copy of.
pub fn to_client(client: &Engine, endpoint: &Endpoint, id: InstanceId) -> Option<InstanceId> {
    if let Some(copy) = endpoint.bridge.ids.borrow().to_client.get(id) {
        return Some(copy);
    }
    let (_, server) = client.peer_server()?;
    let server_end = server.loopback()?;
    pair_lazily(&server, &server_end, client, endpoint, id, Side::Client)
}

/// The server's instance for the client's `id`: the paired one, or the one the server makes of the snapshot where the
/// client made `id` (paired now)
pub fn to_server(client: &Engine, endpoint: &Endpoint, id: InstanceId) -> Option<InstanceId> {
    if let Some(original) = endpoint.bridge.ids.borrow().to_server.get(id) {
        return Some(original);
    }
    let (_, server) = client.peer_server()?;
    let server_end = server.loopback()?;
    pair_lazily(client, endpoint, &server, &server_end, id, Side::Server)
}

/// `id` of `from`, made of the snapshot, paired with `to`'s instance of the same node under the counterpart of what it was
/// made under (`to` makes that one's Studio children first)
fn pair_lazily(from: &Engine, from_end: &Endpoint, to: &Engine, to_end: &Endpoint, id: InstanceId, side: Side) -> Option<InstanceId> {
    let (under, node) = from_end.lazy.borrow().origin.get(&id).copied()?;
    let under = match side {
        Side::Client => to_client(to, to_end, under)?,
        Side::Server => to_server(from, from_end, under)?,
    };
    crate::lua::instance::materialize(to, under);
    let made = to_end.lazy.borrow().made.get(&(under, node)).copied()?;
    to.state.borrow().dom.get(made)?;
    let mut ids = from_end.bridge.ids.borrow_mut();
    match side {
        Side::Client => ids.pair(id, made),
        Side::Server => ids.pair(made, id),
    }
    Some(made)
}

/// A loopback client's copy `id` of a server instance where manifest bundles wait (project::model_assets: the join's copy of
/// a bundle's placeholder, or of the folder a bundle's roots or a loose one's members go into), about to be found, touched
/// or looked inside: the server's instance imports them, as the client's look would in a plain run (`own_only`: the bundle
/// it stands for, for a find or a touch; else the loose ones too, for a look inside), which gives the copy what the
/// import made at once ([`bundle_imported`]); then the copy waits for what still waits there. The server makes nothing a
/// script sees (a bundle is made as its scripts would find it), so a bundle nothing looked at is never fetched or made on
/// either side.
#[cfg(feature = "bundles")]
pub fn import_on_server(client: &Engine, id: InstanceId, own_only: bool) {
    let original = client.loopback().and_then(|endpoint| endpoint.bridge.ids.borrow().to_server.get(id));
    let server = client.peer_server().map(|(_, server)| server);
    // (a destroyed server instance still imports for a destroyed copy: its root, which the copy takes; a copy that is not
    // destroyed has the bundles of the server's destroyed instance whole, imported below)
    let copy_destroyed = client.state.borrow().dom.get(id).is_some_and(|instance| instance.flags & flags::DESTROYED != 0);
    match (original, server) {
        (Some(original), Some(server)) if server.state.borrow().dom.get(original).is_some_and(|instance| copy_destroyed || instance.flags & flags::DESTROYED == 0) => {
            if own_only {
                crate::project::model_assets::reveal(&server, original);
            } else {
                crate::project::model_assets::materialize(&server, original);
            }
            // (an import that made nothing gave the copy nothing: it takes the flag and the waiting the server's has now)
            let (keys, placeholder) = {
                let state = server.state.borrow();
                (state.project.model_assets.waiting_keys(original).cloned(), state.dom.get(original).is_some_and(|instance| instance.has(flags::BUNDLE_ROOT)))
            };
            let mut state = client.state.borrow_mut();
            state.project.model_assets.set_remote(id, keys);
            if let Some(instance) = state.dom.get_mut(id)
                && !placeholder
            {
                instance.set_flag(flags::BUNDLE_ROOT, false);
            }
        }
        // (the server's instance is gone: the copy's bundles are its own now. A destroyed copy a script holds takes its own
        // bundle's root from the server's source, as the game's destroyed instance holds it; one that is not destroyed yet (its
        // server's Destroy on the way, or being applied) imports them from that source, as the game's client holds the whole
        // tree the server sent)
        _ => {
            let keys = {
                let mut state = client.state.borrow_mut();
                let keys = state.project.model_assets.waiting_keys(id).cloned().unwrap_or_default();
                state.project.model_assets.set_remote(id, None);
                keys
            };
            let (destroyed, alive) = match client.state.borrow().dom.get(id) {
                Some(instance) => (instance.flags & flags::DESTROYED != 0 && instance.has(flags::BUNDLE_ROOT), instance.flags & flags::DESTROYED == 0),
                None => (false, false),
            };
            match client.peer_server() {
                Some((_, server)) if destroyed => crate::project::model_assets::import_root(&server, client, id, &keys),
                Some((_, server)) if alive && !keys.is_empty() => crate::project::model_assets::import_from(&server, client, id, &keys),
                _ => {
                    if let Some(instance) = client.state.borrow_mut().dom.get_mut(id) {
                        instance.set_flag(flags::BUNDLE_ROOT, false);
                    }
                }
            }
        }
    }
}

/// After a manifest bundle import at a loopback server's instance `id` (project::model_assets, whichever side looked; `before`:
/// `id`'s children before it): what the import made there (its new children, nothing else changes there while it runs) is made
/// whole, and replicates as `id` does. When the client holds a copy of `id`, the copy takes it now (replicate.rs `copy_import`),
/// so the client finds the bundle as the server made it; when the copy is on its way (in a tree sealed before the import), it
/// takes it when it arrives; when `id` came into the containers in this frame, its tree takes it whole.
#[cfg(feature = "bundles")]
pub fn bundle_imported(server: &Engine, id: InstanceId, before: &[InstanceId]) {
    let Some(endpoint) = server.loopback() else { return };
    if endpoint.side != Side::Server {
        return;
    }
    let bridge = &endpoint.bridge;
    let (made, removed, original) = {
        let mut state = server.state.borrow_mut();
        let Some(flags_of) = state.dom.get(id).map(|instance| instance.flags) else { return };
        // (an instance the client has a copy of, or will have: its own level the import left for later is made now, with it)
        if flags_of & (flags::REPLICATED | flags::SENT) != 0 && flags_of & flags::LAZY_PENDING != 0 {
            crate::project::materialize_lazy(&mut state, &server.reflection, id);
        }
        let instance = state.dom.at(id);
        let had: rustc_hash::FxHashSet<InstanceId> = before.iter().copied().collect();
        let made: Vec<InstanceId> = instance.children.iter().copied().filter(|child| !had.contains(child)).collect();
        let removed: Vec<InstanceId> = before.iter().copied().filter(|&child| state.dom.get(child).is_none_or(|child| child.parent != Some(id))).collect();
        (made, removed, instance.flags)
    };
    // (an instance the client has no copy of and will not have: nothing to send)
    if original & (flags::REPLICATED | flags::SENT) == 0 {
        return;
    }
    super::replicate::make_whole(server, &made);
    if original & flags::REPLICATED != 0 {
        let mut state = server.state.borrow_mut();
        for &root in &made {
            super::replicate::mark(&mut state, root, true, &[]);
        }
    }
    // (not sent yet: the tree that sends `id` takes them; a client that left has no copy to take them)
    if original & flags::SENT == 0 || bridge.left.get() {
        return;
    }
    let Some(client) = bridge.client.borrow().upgrade() else { return };
    let Some(client_end) = client.loopback() else { return };
    let copy = to_client(&client, &client_end, id);
    match copy {
        Some(copy) => {
            let destroyed = match client.state.borrow().dom.get(copy) {
                None => return,
                Some(instance) => instance.flags & flags::DESTROYED != 0,
            };
            // (a destroyed copy a script holds takes the root alone, as the game's destroyed instance holds it)
            let (made, removed): (&[InstanceId], &[InstanceId]) = if destroyed { (&[], &[]) } else { (&made, &removed) };
            crate::timing::time("loopback: a bundle's copy", || super::replicate::copy_import(server, &client, bridge, id, copy, made, removed));
        }
        None => {
            // (the client's copy is in a tree on its way: it takes them as they are now when it arrives, and the server's changes
            // to them after this follow as messages)
            let Some(filter) = bridge.filter.borrow().clone() else { return };
            let snapshot = super::replicate::snapshot_import(server, &filter, id, &made, removed);
            super::replicate::mark_sent(&mut server.state.borrow_mut(), id, &made);
            let mut deferred = bridge.deferred.borrow_mut();
            match deferred.get_mut(&id) {
                // (a second import there before the copy arrived: the root as the first import left it, which the server's
                // writes since then follow as messages, and what waits there now; the instances of both)
                Some(earlier) => {
                    earlier.root.waits = snapshot.root.waits;
                    let offset = earlier.nodes.len() as u32;
                    earlier.nodes.extend(snapshot.nodes.into_iter().map(|mut node| {
                        node.parent = node.parent.map(|parent| parent + offset);
                        node
                    }));
                    earlier.groups.extend(snapshot.groups.into_iter().map(|(index, groups)| (index + offset, groups)));
                    earlier.removed.extend(snapshot.removed);
                }
                None => {
                    deferred.insert(id, snapshot);
                }
            }
        }
    }
}

/// A loopback client's CollectionService lookup by tag (`tag`; any: None, project::model_assets `import_tagged`): the server's
/// waiting bundles that carry it and that the client holds copies of import, and the copies take them
#[cfg(feature = "bundles")]
pub fn import_tagged_on_server(client: &Engine, tag: Option<&str>) {
    let (Some((_, server)), Some(endpoint)) = (client.peer_server(), client.loopback()) else { return };
    // (only the server's instances the client holds copies of: one under a container that does not replicate to it is not its)
    let sent = |state: &crate::State, id: InstanceId| state.dom.at(id).flags & flags::SENT != 0;
    for original in crate::project::model_assets::tagged_waiting(&server, tag, &sent) {
        let copy = endpoint.bridge.ids.borrow().to_client.get(original);
        // (a copy out of the client's DataModel, or destroyed, is not one its lookup lists)
        let listed = copy.is_some_and(|copy| {
            let state = client.state.borrow();
            state.project.model_assets.waits_remotely(copy) && state.dom.get(copy).is_some_and(|instance| instance.flags & flags::DESTROYED == 0) && crate::services::in_data_model(&state, copy)
        });
        if let Some(copy) = copy.filter(|_| listed) {
            import_on_server(client, copy, false);
        }
    }
}

/// A client's message about to go: every instance it names that the client made of the snapshot is paired with the
/// server's now (the server's mapping reads only the pairs)
pub fn pair_sent(client: &Engine, endpoint: &Rc<Endpoint>, message: &Msg) {
    if endpoint.side != Side::Client || endpoint.lazy.borrow().origin.is_empty() {
        return;
    }
    let mut pair = |id: InstanceId| {
        let _ = to_server(client, endpoint, id);
    };
    match message {
        Msg::Event { remote, args } | Msg::Invoke { remote, args, .. } => {
            pair(*remote);
            args.iter().for_each(|wire| each_instance(wire, &mut pair));
        }
        Msg::Return { result: Ok(results), .. } => results.iter().for_each(|wire| each_instance(wire, &mut pair)),
        _ => {}
    }
}

fn each_instance(wire: &Wire, f: &mut dyn FnMut(InstanceId)) {
    match wire {
        Wire::Instance(id) => f(*id),
        Wire::Table(entries) => entries.iter().for_each(|(_, item)| each_instance(item, f)),
        _ => {}
    }
}
