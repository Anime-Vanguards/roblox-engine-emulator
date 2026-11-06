//! Recordings (and `ree run` runs) prepared ahead: a `ree serve` worker that just ran a harness that
//! calls `Context.Checkpoint()` runs the same command again in a fresh engine, up to that call, and
//! parks it there. When the next job has the same arguments (but `--out` and `--profile`, which no script
//! sees) and everything the run read is as it was, the job goes on from the checkpoint: the mount and
//! whatever the harness did before the call (the game's own setup, mostly) are not done again. A
//! harness with Prepare (record/run.rs) has its checkpoint where Prepare returns, before its Context
//! gets the command's `--context` fields, Seconds and Fps: its prepared run serves any job whose other
//! arguments are the same (every unit, cosmetic and scenario of a harness), and the job puts its own
//! fields in.
//!
//! What the job records is what it records without this. The run made ahead is the job's own run up
//! to the checkpoint (same arguments, a fresh engine, nothing after the call has run: not even the
//! deferred flush that ends the harness's first resumption), and it is used only when what it read
//! reads the same now:
//!
//! - every file and listing it read, or found missing (crate::files::note: the project's settings,
//!   the Studio hierarchy, mounted files, the asset manifest and the bundle files, animation files);
//! - the project's directory listings (project::Listing): the same entries, and the same stamps for
//!   every file but the Luau sources it did not read (those are read later, as they are then);
//! - the wall clock's hour and zone (os.time, DateTime.now, tick read the hour the engine was made);
//! - no image read or written, no bundle or animation that failed to load, no recorded clock (RUR_CLOCK).
//!
//! Anything else is the same by construction: the arguments, the environment and the build are the
//! service's and the job's own, and the run's clocks are virtual. The files and listings are checked
//! on the thread pool while the job goes on from the checkpoint; a stale run's job starts over before
//! anything is written.

use std::{rc::Rc, time::Instant};

use super::{
    Recorder,
    run::{self, Begun, Done, History, RecordArgs},
};
use crate::{Emulator, launch::Host, lua::modules};

/// The jobs a prepared run serves: those of its command (`record`, else `run`) whose [`key`] (`split`:
/// [`split_key`]) is `args`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub split: bool,
    /// Its harness has Prepare and parked between frames (its frames ran at the job's `--fps`)
    pub framed: bool,
    pub record: bool,
    pub args: Vec<String>,
}

/// A job's arguments as each kind of prepared run serves them.
#[derive(Clone, Debug)]
pub struct Keys {
    pub record: bool,
    pub full: Vec<String>,
    pub split: Vec<String>,
    pub framed: Vec<String>,
}

impl Keys {
    pub fn of(args: &[String], record: bool) -> Keys {
        Keys { record, full: key(args), split: split_key(args), framed: framed_key(args) }
    }
}

impl Key {
    /// The key of a run of `args` prepared for its command (`record`) and harness (`split`: it has Prepare).
    pub fn of(args: &[String], split: bool, record: bool) -> Key {
        Key { split, framed: false, record, args: if split { split_key(args) } else { key(args) } }
    }

    /// [`Key::of`] for a run its harness parked between frames (`framed`): with Prepare, the jobs with
    /// the same `--fps` and `--frame-times`.
    pub fn of_framed(args: &[String], split: bool, framed: bool, record: bool) -> Key {
        if split && framed { Key { split, framed, record, args: framed_key(args) } } else { Key::of(args, split, record) }
    }

    /// Whether a run prepared for this key serves a job with `keys`.
    pub fn serves(&self, keys: &Keys) -> bool {
        self.record == keys.record && self.args == *if self.framed { &keys.framed } else if self.split { &keys.split } else { &keys.full }
    }
}

/// A recording (or a run, without a recorder) up to its harness's checkpoint, waiting for its job.
pub struct Prepared {
    key: Key,
    emulator: Emulator,
    recorder: Option<Recorder>,
    begun: Begun,
    history: History,
    noted: crate::files::Noted,
    /// The diagnostics it wrote (the job's stderr starts with them)
    diagnostics: String,
}

/// The arguments a prepared run serves: the command's but `--out <dir>` and `--profile`.
pub fn key(args: &[String]) -> Vec<String> {
    without(args, &["--out"])
}

/// The arguments a prepared run of a harness with Prepare serves: [`key`]'s but `--context`,
/// `--seconds`, `--fps` and `--frame-times` (its Prepare sees none of them).
pub fn split_key(args: &[String]) -> Vec<String> {
    without(args, &["--out", "--context", "--seconds", "--fps", "--frame-times"])
}

/// The arguments a run of a harness with Prepare parked between frames serves: [`split_key`]'s with
/// `--fps` and `--frame-times` (its Prepare ran frames at them).
pub fn framed_key(args: &[String]) -> Vec<String> {
    without(args, &["--out", "--context", "--seconds"])
}

/// `args` without `--profile` and the options `skip` with their values.
fn without(args: &[String], skip: &[&str]) -> Vec<String> {
    let args = crate::launch::split_equals(args);
    let mut key = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--profile" => index += 1,
            option if skip.contains(&option) => index += 2,
            _ => {
                key.push(args[index].clone());
                index += 1;
            }
        }
    }
    key
}

/// `ree record`'s (`record`) or `ree run`'s run of `args` up to its harness's checkpoint (an error: the
/// harness has none before it first yields, or the run read what cannot be checked again).
pub fn prepare(args: &[String], host: &Host, record: bool) -> Result<Prepared, String> {
    if std::env::var_os("RUR_CLOCK").is_some_and(|clock| !clock.is_empty()) {
        return Err("a recorded clock (RUR_CLOCK)".into());
    }
    let parsed = run::parse(&crate::launch::split_equals(args), record)?;
    crate::files::start_noting();
    crate::output::capture();
    let started = start(&parsed, host, record);
    let diagnostics = crate::output::take_captured();
    let noted = crate::files::stop_noting();
    let (emulator, recorder, begun, history) = started?;
    let key = Key::of_framed(args, begun.split(), begun.framed().is_some(), record);
    let prepared = Prepared { key, emulator, recorder, begun, history, noted, diagnostics };
    if !prepared.begun.parked() {
        return Err("the harness has no checkpoint before it first yields or, in a run, between frames".into());
    }
    if let Some(why) = prepared.noted.changed() {
        return Err(why);
    }
    Ok(prepared)
}

fn start(args: &RecordArgs, host: &Host, record: bool) -> Result<(Emulator, Option<Recorder>, Begun, History), String> {
    let (emulator, history) = run::start(args, host, true)?;
    let recorder = record.then(|| Recorder::install(&emulator.engine, args.fps));
    let mut begun = run::begin_run(&emulator.lua, &emulator.engine, args, true)?;
    if !begun.parked() && !record {
        run::advance_to_frame_checkpoint(&emulator.lua, &emulator.engine, args, &mut begun)?;
    }
    Ok((emulator, recorder, begun, history))
}

impl Prepared {
    /// The jobs it serves.
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Its job (`args`, which this run's [`Key`] serves): the harness goes on from its checkpoint,
    /// then the frames and the files, as `ree record` makes them, or the summary `ree run` prints (the
    /// status line counts from `started`). Err when it is stale: something it read reads differently now (the job is to
    /// start over). What it read is checked on the thread pool while the job goes on (the files it
    /// writes wait for the check), and the files it reads from here on are read as they are now.
    pub fn run(self, args: &[String], host: &Host, started: Instant) -> Result<Result<(Done, Emulator), String>, String> {
        let Prepared { emulator, recorder, begun, history, noted, diagnostics, .. } = self;
        let engine: &Rc<crate::Engine> = &emulator.engine;
        if !engine.state.borrow().sched.wall_clock_current() {
            return Err("the wall clock's hour changed".into());
        }
        let read = modules::read_files(engine);
        let listings = engine.state.borrow_mut().project.listings.take().unwrap_or_default();
        crate::timing::tally("prepared: files noted", noted.len() as u64);
        crate::timing::tally("prepared: listings", listings.len() as u64);
        let (sender, receiver) = std::sync::mpsc::channel();
        rayon::spawn(move || {
            use rayon::prelude::*;
            let changed = noted.changed().or_else(|| listings.par_iter().find_any(|listing| !listing.unchanged(&read)).map(|listing| format!("{} changed", listing.path())));
            let _ = sender.send(changed);
        });
        // (the listing's stamps are the prepared run's: a module read from here on is stamped now)
        engine.state.borrow_mut().project.stamps.clear();
        let parsed = run::parse(&crate::launch::split_equals(args), recorder.is_some()).map_err(|error| format!("its arguments: {error}"))?;
        if begun.framed().is_some_and(|frames| frames as usize > parsed.frame_schedule().len()) {
            return Err("the job ends before its frame checkpoint".into());
        }
        // (a harness with Prepare gets the job's own fields, Seconds and Fps after the checkpoint;
        // nothing has been sampled yet)
        begun.set_late(run::Late::of(&parsed));
        if let Some(recorder) = &recorder {
            recorder.set_fps(parsed.fps);
            recorder.set_frame_times(&parsed);
        }
        crate::output::replay(&diagnostics);
        let ran = run::finish_run(&emulator.lua, engine, &parsed, recorder.as_ref(), begun);
        let changed = crate::timing::time("prepared: check (waited)", || receiver.recv().unwrap_or_else(|_| Some("the check failed".into())));
        if let Some(why) = changed {
            return Err(why);
        }
        let done = ran.and_then(|ran| match &recorder {
            Some(recorder) => run::record_done(&emulator, &parsed, host, recorder, ran, started),
            None => run::run_done(&emulator, &parsed, host, ran, started),
        });
        run::finished(&emulator, &history);
        Ok(done.map(|done| (done, emulator)))
    }
}
