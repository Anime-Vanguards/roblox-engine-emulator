//! Every view and the key paths through them, drawn headless at a fixed size and compared with snapshots
//! (tests/snapshots; REE_VIEW_BLESS=1 rewrites them), and the files `render` writes: text, SVG and PNG.

mod common;

use common::*;
use ree_profile_view::{
    app::View,
    cli,
    export::{self, CellFont, Format},
    theme::{Mark, Theme, ThemeKind},
};

const COLS: u16 = 120;
const ROWS: u16 = 40;

#[test]
fn every_view_matches_its_snapshot() {
    let temp = Temp::new("views");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    for view in [View::Overview, View::Flame, View::Calls, View::Source, View::Alloc, View::Heap, View::Timeline, View::Caches, View::Issues] {
        let mut app = open(&[&dir]);
        app.view = view;
        let text = screen(&mut app, "", COLS, ROWS);
        snapshot(&format!("view-{}", view.name()), &text, temp.path());
    }
}

/// (snapshot, key script): each a path a user takes, from the Overview
const PATHS: &[(&str, &str)] = &[
    // the first issue (a sort every frame) opens its function in Calls
    ("overview-enter", "enter"),
    // the flame starts on the heaviest phase; down its heaviest callees twice, enter zooms in there, esc back out
    ("flame-zoom", "2 down down enter"),
    ("flame-zoom-out", "2 down down enter esc"),
    // Calls on rank (found by name): down to its callee (sort), enter centres on it, esc back to rank
    ("calls-centre", "3 /rank down down enter"),
    ("calls-back", "3 /rank down down enter esc"),
    // Source lists lines by heat with s
    ("source-hot", "4 s"),
    // Allocations sorted by bytes a frame in the steady state, then enter opens the site's line
    ("alloc-steady", "5 s s"),
    ("alloc-enter", "5 enter"),
    // the Heap's next pane and the snapshot before the last
    ("heap-pane", "6 tab left"),
    // the Timeline: three frames left of the worst one, then zoomed in
    ("timeline-move", "7 left left left"),
    ("timeline-zoom", "7 enter"),
    // Caches by miss rate
    ("caches-rate", "8 s"),
    // Issues: the second one opened
    ("issues-open", "9 down enter"),
    // the metric switched to allocated bytes on the Flame, and to instructions on the Overview
    ("flame-alloc", "2 m m m"),
    ("overview-insns", "m"),
    // a search in the Flame finds compare's node
    ("flame-search", "2 /anonymous n"),
    // scripts only folds C functions into their callers
    ("flame-scripts", "2 f"),
    // the help over the Flame
    ("flame-help", "2 ?"),
    // the light theme draws the same text (colours are the PNG test's)
    ("overview-light", "t"),
];

#[test]
fn key_paths_match_their_snapshots() {
    let temp = Temp::new("paths");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    for (name, keys) in PATHS {
        let mut app = open(&[&dir]);
        let text = screen(&mut app, keys, COLS, ROWS);
        snapshot(&format!("path-{name}"), &text, temp.path());
    }
}

#[test]
fn a_pair_opens_on_the_diff_and_its_tables() {
    let temp = Temp::new("pair");
    let before = write_profile(&temp.path().join("before"), Shape::BEFORE);
    let after = write_profile(&temp.path().join("after"), Shape::AFTER);
    for (name, keys) in [("diff-functions", ""), ("diff-lines", "tab"), ("diff-sites", "tab tab"), ("diff-grew", "s"), ("diff-enter", "enter"), ("diff-side", "x 1")] {
        let mut app = open(&[&before, &after]);
        assert_eq!(app.view, View::Diff, "a pair starts on the diff");
        let text = screen(&mut app, keys, COLS, ROWS);
        snapshot(&format!("pair-{name}"), &text, temp.path());
    }
}

#[test]
fn a_comparison_opens_on_compare_and_its_runs_open() {
    let temp = Temp::new("compare");
    let dir = write_comparison(&temp.path().join("what-if"));
    for (name, keys) in [("compare", ""), ("compare-alloc", "m m"), ("compare-size", "left down"), ("compare-open", "down enter"), ("compare-diff", "down D"), ("compare-issues", "9")] {
        let mut app = open(&[&dir]);
        assert_eq!(app.view, View::Compare, "a what-if folder starts on Compare");
        let text = screen(&mut app, keys, COLS, ROWS);
        snapshot(&format!("compare-{name}"), &text, temp.path());
    }
    // the baseline's run at the largest size is open in the other views
    let mut app = open(&[&dir]);
    assert!(app.has_profile());
    assert_eq!(app.profile().raw.run.frames, 120);
    // enter on a run with no profile folder says so and stays
    let text = screen(&mut app, "left enter", COLS, ROWS);
    assert_eq!(app.view, View::Compare, "{text}");
    assert!(text.contains("no profile folder"), "{text}");
}

#[test]
fn the_same_screen_draws_the_same_bytes() {
    let temp = Temp::new("determinism");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let draw = || {
        let mut app = open(&[&dir]);
        app.view = View::Flame;
        cli::screen(&mut app, COLS, ROWS)
    };
    let (a, b) = (draw(), draw());
    let theme = Theme::new(ThemeKind::Dark);
    for format in [Format::Png, Format::Svg, Format::Text] {
        assert_eq!(export::encode(&a, &theme, format, 1).unwrap(), export::encode(&b, &theme, format, 1).unwrap(), "{format:?}");
    }
}

/// The PNG is the cells rasterized: its size is the grid's, a cell's background and a bar's colour are where the buffer
/// puts them, and a bar is drawn as a band through the middle of its cell (a gap above and below)
#[test]
fn the_png_is_the_buffer_rasterized() {
    let temp = Temp::new("png");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let mut app = open(&[&dir]);
    let buf = cli::screen(&mut app, COLS, ROWS);
    let theme = app.theme.clone();
    let bytes = export::encode(&buf, &theme, Format::Png, 1).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    let (cw, ch) = (export::CELL_W, export::CELL_H);
    assert_eq!((info.width, info.height), (COLS as u32 * cw, ROWS as u32 * ch));
    let pixel = |x: u32, y: u32| -> (u8, u8, u8) {
        let at = ((y * info.width + x) * 3) as usize;
        (pixels[at], pixels[at + 1], pixels[at + 2])
    };
    let rgb = |color: ratatui::style::Color| match color {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        _ => panic!("the themes use RGB colours"),
    };
    // the brand's pill, at the first cell's corner
    assert_eq!(pixel(0, 0), rgb(theme.accent));
    // a full bar cell: the bar's colour across the middle of the cell, its row's background above and below it
    let (bx, by) = (0..ROWS)
        .flat_map(|y| (0..COLS).map(move |x| (x, y)))
        .find(|&(x, y)| buf[(x, y)].symbol() == "█" && Theme::unmark(buf[(x, y)].bg).1 == Mark::Bar)
        .expect("a bar on the Overview");
    let fg = rgb(buf[(bx, by)].fg);
    let row_bg = rgb(Theme::unmark(buf[(bx, by)].bg).0);
    let (x0, y0) = (bx as u32 * cw, by as u32 * ch);
    assert_eq!(pixel(x0 + cw / 2, y0 + ch / 2), fg, "the bar's middle");
    assert_eq!(pixel(x0 + cw / 2, y0 + 1), row_bg, "a gap above the bar");
    assert_eq!(pixel(x0 + cw / 2, y0 + ch - 2), row_bg, "a gap below the bar");
    // a letter's cell has ink in it
    let (lx, ly) = (0..COLS).map(|x| (x, 0)).find(|&(x, y)| buf[(x, y)].symbol() == "r").unwrap();
    let ink = rgb(buf[(lx, ly)].fg);
    let inked = (0..cw).flat_map(|dx| (0..ch).map(move |dy| (dx, dy))).filter(|&(dx, dy)| pixel(lx as u32 * cw + dx, ly as u32 * ch + dy) == ink).count();
    assert!(inked >= 3, "the r of the header is drawn ({inked} pixels in ink)");
    // and twice as large at scale 2
    let bytes = export::encode(&buf, &theme, Format::Png, 2).unwrap();
    let info = png::Decoder::new(std::io::Cursor::new(&bytes)).read_info().unwrap().info().clone();
    assert_eq!((info.width, info.height), (COLS as u32 * cw * 2, ROWS as u32 * ch * 2));
}

/// Every character any view draws is in the bundled font, or drawn as a shape (blocks, box lines, braille)
#[test]
fn every_character_the_views_draw_has_a_glyph() {
    let temp = Temp::new("glyphs");
    let before = write_profile(&temp.path().join("before"), Shape::BEFORE);
    let after = write_profile(&temp.path().join("after"), Shape::AFTER);
    let what_if = write_comparison(&temp.path().join("what-if"));
    let mut seen = std::collections::BTreeSet::new();
    let mut take = |app: &mut ree_profile_view::app::App, keys: &str| {
        let text = screen(app, keys, COLS, ROWS);
        seen.extend(text.chars().filter(|c| *c != '\n'));
    };
    for view in View::ALL {
        let mut app = open(&[&before]);
        if app.available(view) {
            app.view = view;
            take(&mut app, "");
            take(&mut app, "?");
        }
    }
    let mut pair = open(&[&before, &after]);
    take(&mut pair, "");
    take(&mut pair, "tab tab");
    let mut compare = open(&[&what_if]);
    take(&mut compare, "");
    take(&mut compare, "9");
    let font = CellFont::new(1);
    let missing: Vec<char> = seen.into_iter().filter(|c| !font.covers(*c)).collect();
    assert!(missing.is_empty(), "characters the font lacks: {missing:?}");
}

#[test]
fn render_writes_a_file_a_view_and_prints_each() {
    let temp = Temp::new("render");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let out = temp.path().join("shots");
    let args: Vec<String> = [dir.to_str().unwrap(), "--view", "all", "--out", out.to_str().unwrap(), "--size", "100x30"].iter().map(|s| s.to_string()).collect();
    assert_eq!(cli::render("ree profile", &args), 0);
    for view in ["overview", "flame", "calls", "source", "alloc", "heap", "timeline", "caches", "issues"] {
        let file = out.join(format!("{view}.png"));
        assert!(file.is_file(), "{}", file.display());
    }
    assert!(!out.join("diff.png").exists(), "one profile has no diff");
    // one view to text and to SVG, by the file's extension
    for (file, check) in [("issues.txt", "Sort calls"), ("flame.svg", "<svg ")] {
        let path = temp.path().join(file);
        let view = file.split('.').next().unwrap();
        let args: Vec<String> = [dir.to_str().unwrap(), "--view", view, "--out", path.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        assert_eq!(cli::render("ree profile", &args), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(check), "{file}: {text}");
    }
    // what it refuses, before it writes anything
    for (args, error) in [
        (vec![dir.to_str().unwrap(), "--view", "flame"], "--out"),
        (vec![dir.to_str().unwrap(), "--view", "diff", "--out", "x.png"], "two profiles"),
        (vec![dir.to_str().unwrap(), "--view", "nope", "--out", "x.png"], "no view"),
        (vec![dir.to_str().unwrap(), "--metric", "nope", "--out", "x.png"], "no metric"),
        (vec![dir.to_str().unwrap(), "--select", "nothing_like_this", "--out", "x.png"], "--select"),
        (vec![dir.to_str().unwrap(), "--view", "all", "--out", "x.png"], "a folder"),
        (vec![dir.to_str().unwrap(), "--size", "10x5", "--out", "x.png"], "--size"),
        (vec![temp.path().join("nothing").to_str().unwrap(), "--out", "x.png"], "no profile in"),
    ] {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        assert_eq!(cli::render("ree profile", &args), 2, "{args:?} ({error})");
    }
    assert!(!std::path::Path::new("x.png").exists());
}

#[test]
fn select_finds_a_function_or_a_line() {
    let temp = Temp::new("select");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let mut app = open(&[&dir]);
    assert!(app.select("layout"));
    let focus = app.focus.unwrap();
    assert_eq!(app.profile().funcs[focus.func as usize].name, "layout");
    let mut app = open(&[&dir]);
    assert!(app.select("Main.luau:21"));
    let focus = app.focus.unwrap();
    assert_eq!((app.profile().funcs[focus.func as usize].name.as_str(), focus.line), ("rank", Some(21)));
    app.view = View::Source;
    let text = screen(&mut app, "", COLS, ROWS);
    assert!(text.lines().any(|line| line.contains("▌") && line.contains("21")), "the line is selected:\n{text}");
    assert!(!open(&[&dir]).select("nothing_like_this"));
}

#[test]
fn the_games_code_leads_and_the_harnesss_comes_apart() {
    use ree_profile_view::ui::overview::{Side, sides};
    let temp = Temp::new("sides");
    let dir = write_profile(&temp.path().join("run"), Shape::HARNESS);
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "", COLS, ROWS);
    snapshot("harness-overview", &text, temp.path());
    assert!(text.contains("The game's code") && text.contains("The harness's code"), "{text}");
    // the steady phase's self work: the game's code 10.63 ms (rank's call from the harness is the game's: a script
    // function is its module's side), the harness's 4.000 ms (its scan, a C function, goes with the harness)
    let labels = |app: &ree_profile_view::app::App, side: &Side| side.rows.iter().map(|r| (app.profile().funcs[r.func as usize].label(), (r.own * 1e6).round() / 1e6)).collect::<Vec<_>>();
    let got = sides(&app);
    assert!(got.apart);
    assert!((got.lead.total - 10.63).abs() < 1e-9 && (got.harness.total - 4.0).abs() < 1e-9, "{} {}", got.lead.total, got.harness.total);
    assert_eq!(labels(&app, &got.harness), [("GetDescendants [C]".to_string(), 2.5), ("watch  Fixture:5".to_string(), 1.5)]);
    assert_eq!(labels(&app, &got.lead)[..3], [("anonymous  Main:22".to_string(), 4.0), ("rank  Main:20".to_string(), 3.03), ("layout  Main:30".to_string(), 2.0)]);
    // a profile from before ree c692e40 (no harness flags, no figures per phase, no scope) reads the same from its call
    // tree, its steady phase and the harness's folder
    let path = dir.join("profile.json");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for f in json["functions"].as_array_mut().unwrap() {
        let f = f.as_object_mut().unwrap();
        f.remove("harness");
        f.remove("phases");
    }
    json.as_object_mut().unwrap().remove("scope");
    let old = temp.path().join("old");
    std::fs::create_dir_all(&old).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), old.join(entry.file_name())).unwrap();
    }
    std::fs::write(old.join("profile.json"), json.to_string()).unwrap();
    let older = open(&[&old]);
    let from_tree = sides(&older);
    assert!(from_tree.apart);
    assert_eq!((from_tree.lead.total * 1e6).round(), (got.lead.total * 1e6).round());
    assert_eq!(labels(&older, &from_tree.harness), labels(&app, &got.harness));
    assert_eq!(labels(&older, &from_tree.lead), labels(&app, &got.lead));
}

#[test]
fn what_the_scripts_asked_the_engine_to_do_reads_by_phase() {
    // the steady phase's counts are its mark's to the run's end: 100 clones, 3,000 writes (2,400 changed, 600 moving a
    // part), 50 emits; the change clones a tenth and writes less
    let temp = Temp::new("engine");
    let before = write_profile(&temp.path().join("before"), Shape::BEFORE);
    let after = write_profile(&temp.path().join("after"), Shape::AFTER);
    let mut app = open(&[&before]);
    let ops = app.profile().engine_ops(&app.profile().scope()).expect("the counts");
    assert_eq!((ops["cloned"], ops["writes"], ops["changed"], ops["geometry"], ops["emits"]), (100.0, 3000.0, 2400.0, 600.0, 50.0));
    let every = app.profile().scope_of(&[]);
    assert_eq!(app.profile().engine_ops(&every).unwrap()["cloned"], 140.0, "the mount's 40 and the steady state's 100");
    let text = screen(&mut app, "", 200, ROWS);
    assert!(text.contains("engine  phase steady  100 cloned · 100 destroyed · 100 into the Workspace · 3,000 writes (2,400 changed: 600 moved a part, 20 switched an effect) · 10 pivots · 50 emits · 200 queries"), "{text}");
    // the diff compares them in the phases both report, as ree profile diff groups them
    let mut pair = open(&[&before, &after]);
    let text = screen(&mut pair, "", 160, 48);
    for row in ["engine: instances made", "engine: property writes", "engine: parts moved", "engine: pivots"] {
        assert!(text.contains(row), "{row}: {text}");
    }
    assert!(text.contains("the first figure, the engine's counts and the functions' work cover phase steady"), "{text}");
    // a profile from before the counts shows none
    let path = before.join("profile.json");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for mark in json["marks"].as_array_mut().unwrap() {
        mark.as_object_mut().unwrap().remove("engine");
    }
    json.as_object_mut().unwrap().remove("end");
    std::fs::write(&path, json.to_string()).unwrap();
    let mut older = open(&[&before]);
    assert!(older.profile().engine_ops(&older.profile().scope()).is_none());
    assert!(!screen(&mut older, "", COLS, ROWS).contains("◇ engine"));
}

/// A profile's JSON changed by `edit`, in a folder of its own
fn edited(temp: &Temp, name: &str, shape: Shape, edit: impl FnOnce(&mut serde_json::Value)) -> std::path::PathBuf {
    let dir = write_profile(&temp.path().join(name), shape);
    let path = dir.join("profile.json");
    let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut json);
    std::fs::write(&path, json.to_string()).unwrap();
    dir
}

#[test]
fn the_header_marks_only_a_failure_and_says_what_was_not_measured() {
    let temp = Temp::new("header");
    // a harness still waiting when the run ends (`yielded`) is a run's usual end: no ✗
    for (status, mark) in [("ok", "✓ run ok"), ("yielded", "○ run yielded"), ("error", "✗ run error")] {
        let dir = edited(&temp, status, Shape::BEFORE, |json| json["run"]["status"] = serde_json::json!(status));
        let text = screen(&mut open(&[&dir]), "", COLS, ROWS);
        assert!(text.lines().next().unwrap().contains(mark), "{status}: {}", text.lines().next().unwrap());
    }
    // without the work kind: the frames' work clock where the timeline has it, else not measured, never 0 ms
    let no_work = |json: &mut serde_json::Value| {
        let object = json.as_object_mut().unwrap();
        for key in ["work", "tree", "luau_caches", "alloc"] {
            object.remove(key);
        }
        json["kinds"] = serde_json::json!(["timeline"]);
    };
    let dir = edited(&temp, "timeline", Shape::BEFORE, no_work);
    let header = screen(&mut open(&[&dir]), "", COLS, ROWS).lines().next().unwrap().to_string();
    assert!(header.contains("Studio work") && header.contains("in frames") && !header.contains(" 0 ms"), "{header}");
    let dir = edited(&temp, "nothing", Shape::BEFORE, |json| {
        no_work(json);
        json.as_object_mut().unwrap().remove("timeline");
    });
    let header = screen(&mut open(&[&dir]), "", COLS, ROWS).lines().next().unwrap().to_string();
    assert!(header.contains("Studio work not measured"), "{header}");
}

#[test]
fn a_stepping_profile_shows_no_host_time_a_frame() {
    // the VM stepped (work): its host time a frame is the slowed interpreter's, so the Timeline leaves it out and says so
    let temp = Temp::new("host");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let app = open(&[&dir]);
    let series = ree_profile_view::ui::timeline::series(app.profile());
    assert!(!series.iter().any(|s| s == "host_ms" || s == "cycles" || s == "part"), "{series:?}");
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "7", COLS, ROWS);
    assert!(text.contains("host time not shown: stepping slows the interpreter"), "{text}");
    // a timeline of its own keeps it
    let dir = edited(&temp, "plain", Shape::BEFORE, |json| json["kinds"] = serde_json::json!(["timeline"]));
    let app = open(&[&dir]);
    assert!(ree_profile_view::ui::timeline::series(app.profile()).iter().any(|s| s == "host_ms"));
}

/// The harness fixture with each timeline row's work split by side, as a stepping profile's timeline has it: the game's
/// code 0.06 ms a frame in the steady phase (0.6 at frame 60) and the harness's 0.04 ms
fn sided_timeline(json: &mut serde_json::Value) {
    json["timeline"]["columns"].as_array_mut().unwrap().extend([serde_json::json!("part"), serde_json::json!("game_ms"), serde_json::json!("harness_ms")]);
    for row in json["timeline"]["rows"].as_array_mut().unwrap() {
        let frame = row[0].as_f64().unwrap();
        let game = if frame == 60.0 { 0.6 } else { 0.06 };
        row.as_array_mut().unwrap().extend([serde_json::json!(0), serde_json::json!(game), serde_json::json!(0.04)]);
    }
}

#[test]
fn a_frames_figures_are_the_games_code_when_the_harness_is_apart() {
    let temp = Temp::new("frames");
    let dir = edited(&temp, "run", Shape::HARNESS, sided_timeline);
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "", 160, ROWS);
    // the steady phase's 100 frames: 99 at 0.06 ms and one at 0.6 ms, a mean of 0.0654 ms
    assert!(text.contains("game code/frame"), "{text}");
    assert!(text.contains("65.4 µs") && text.contains("max 0.600 ms"), "{text}");
    assert!(text.contains("The game's code a frame"), "the chart is the game's code a frame: {text}");
    // the Timeline opens on the same series
    assert_eq!(ree_profile_view::ui::timeline::current(&app).as_deref(), Some("game_ms"));
}

#[test]
fn both_sides_hottest_functions_share_one_heat_scale() {
    // the harness's scan (2.5 ms) against the game's comparator (4.0 ms): heat and bars are shares of both sides'
    // 14.63 ms, so the harness's top row is not drawn as its side's whole
    use ree_profile_view::ui::overview::sides;
    let temp = Temp::new("heat");
    let dir = write_profile(&temp.path().join("run"), Shape::HARNESS);
    let mut app = open(&[&dir]);
    let got = sides(&app);
    let theme = app.theme.clone();
    let text = screen(&mut app, "", COLS, ROWS);
    assert!(text.contains("both sides"), "{text}");
    // (the colours: the harness's top row at its share of both sides, not of its own side)
    let whole = got.lead.total + got.harness.total;
    assert_ne!(theme.heat_of(2.5 / whole), theme.heat_of(2.5 / got.harness.total), "the two scales differ for this fixture");
}

#[test]
fn a_function_c_calls_says_its_calls_are_counted_once_a_c_call() {
    // the comparator (Main:22) is called by sort [C]: 100 sorts read as 50,100 calls in the fixture's call tree, which a
    // profile without invocation counts cannot tell from runs
    let temp = Temp::new("from-c");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "3 /anonymous", 200, ROWS);
    assert!(text.contains("calls, counted once a C call (needs invocation counts)"), "{text}");
    let dir = edited(&temp, "counted", Shape::BEFORE, |json| json["work"]["c_invocations"] = serde_json::json!(true));
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "3 /anonymous", 200, ROWS);
    assert!(!text.contains("counted once a C call") && text.contains(" each"), "{text}");
}

#[test]
fn lines_counted_per_phase_and_the_collectors_work_show_where_a_profile_has_them() {
    // a newer ree counts lines per phase (lines[].phases) and prices the collector inside the work clock
    // (work.collector_ms): the Calls view's hottest lines are then the reported phase's, and the Overview has the
    // collector's card; a profile without them says its lines are the whole run's and shows no such card
    let temp = Temp::new("newer");
    let plain = write_profile(&temp.path().join("plain"), Shape::BEFORE);
    let mut app = open(&[&plain]);
    let text = screen(&mut app, "3 /layout", 200, ROWS);
    assert!(text.contains("the whole run (lines are not counted per phase)"), "{text}");
    assert!(!screen(&mut open(&[&plain]), "", 200, ROWS).contains("the collector"));
    let dir = edited(&temp, "newer", Shape::BEFORE, |json| {
        json["work"]["collector_ms"] = serde_json::json!(1.5);
        for line in json["lines"].as_array_mut().unwrap() {
            let work = line["work_ms"].as_f64().unwrap();
            line["phases"] = serde_json::json!({"steady": {"work_ms": work * 0.75, "total_work_ms": work * 0.75, "insns": 1}});
        }
    });
    let mut app = open(&[&dir]);
    let text = screen(&mut app, "3 /layout", 200, ROWS);
    assert!(text.contains("own Studio work · phase steady") && !text.contains("not counted per phase"), "{text}");
    // layout's line 31: 1.7 ms over the run, 1.275 ms in the steady phase
    assert!(text.contains("1.275 ms"), "{text}");
    let text = screen(&mut open(&[&dir]), "", 200, ROWS);
    assert!(text.contains("the collector") && text.contains("1.500 ms"), "{text}");
}

#[test]
fn host_time_a_frame_shows_when_a_run_of_its_own_measured_it() {
    // a newer ree takes a stepping profile's CPU figures in a pass of its own (`cpu.pass`) and fills the timeline's host
    // time from it: then it is the run's and shown; where it writes null, it is not
    let temp = Temp::new("cpu-pass");
    let dir = edited(&temp, "pass", Shape::BEFORE, |json| json["cpu"] = serde_json::json!({"pass": "a run of its own"}));
    let app = open(&[&dir]);
    assert!(ree_profile_view::ui::timeline::series(app.profile()).iter().any(|s| s == "host_ms"));
    let dir = edited(&temp, "null", Shape::BEFORE, |json| {
        json["kinds"] = serde_json::json!(["timeline"]);
        for row in json["timeline"]["rows"].as_array_mut().unwrap() {
            row[3] = serde_json::Value::Null;
        }
    });
    let app = open(&[&dir]);
    assert!(!ree_profile_view::ui::timeline::series(app.profile()).iter().any(|s| s == "host_ms"));
}

#[test]
fn a_timeline_profiles_header_never_reads_zero_work_from_its_empty_call_tree() {
    // a real timeline profile keeps a call tree of one empty node and no work section
    let temp = Temp::new("one-node-header");
    let dir = edited(&temp, "run", Shape::BEFORE, |json| {
        let object = json.as_object_mut().unwrap();
        for key in ["work", "luau_caches", "alloc"] {
            object.remove(key);
        }
        json["tree"] = serde_json::json!([[-1, -1, 0, 0.0, 0, "startup"]]);
        json["kinds"] = serde_json::json!(["timeline"]);
    });
    let header = screen(&mut open(&[&dir]), "", COLS, ROWS).lines().next().unwrap().to_string();
    assert!(header.contains("in frames") && !header.contains(" 0 ms"), "{header}");
}

#[test]
fn every_not_checked_note_shows_in_the_issues_view() {
    // a profile missing most kinds: each note is shown, wrapped, none dropped
    let temp = Temp::new("notes");
    let dir = edited(&temp, "run", Shape::HARNESS, |json| {
        let object = json.as_object_mut().unwrap();
        for key in ["work", "alloc", "timeline", "luau_caches"] {
            object.remove(key);
        }
        json["tree"] = serde_json::json!([[-1, -1, 0, 0.0, 0, "startup"]]);
    });
    std::fs::remove_file(dir.join("boundary.json")).unwrap();
    let mut app = open(&[&dir]);
    let notes = app.issues().1.clone();
    assert!(notes.len() >= 4, "{notes:?}");
    let text = screen(&mut app, "9", COLS, 60);
    let flat = text.replace('│', " ").split_whitespace().collect::<Vec<_>>().join(" ");
    for note in &notes {
        let start: String = note.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
        assert!(flat.contains(&start), "{start:?} missing:\n{text}");
    }
    assert!(text.contains(&format!("Not checked  {} ·", notes.len())), "{text}");
}

#[test]
fn a_short_screen_shows_whole_not_checked_notes_and_counts_the_rest() {
    // at 60 by 16 the panel holds a few lines: each note it shows is whole and wrapped inside the panel, and the count says
    // how many are left, a note that did not fit among them, never half of one on screen
    let temp = Temp::new("notes-short");
    let dir = edited(&temp, "run", Shape::HARNESS, |json| {
        let object = json.as_object_mut().unwrap();
        for key in ["work", "alloc", "timeline", "luau_caches"] {
            object.remove(key);
        }
        json["tree"] = serde_json::json!([[-1, -1, 0, 0.0, 0, "startup"]]);
    });
    std::fs::remove_file(dir.join("boundary.json")).unwrap();
    let mut app = open(&[&dir]);
    let notes = app.issues().1.clone();
    let text = screen(&mut app, "9", 60, 16);
    let lines: Vec<&str> = text.lines().collect();
    let top = lines.iter().position(|line| line.contains("Not checked")).unwrap_or_else(|| panic!("{text}"));
    let (mut shown, mut more): (Vec<String>, Option<usize>) = (Vec::new(), None);
    for line in &lines[top + 1..] {
        if line.starts_with('╰') {
            break;
        }
        let inner = line.trim_start_matches('│').trim_end_matches('│').trim_end();
        let inner = inner.strip_prefix(' ').unwrap_or(inner);
        if let Some(rest) = inner.strip_prefix("○ ") {
            shown.push(rest.trim().to_string());
        } else if inner.starts_with("… and ") {
            more = inner.split_whitespace().nth(2).and_then(|count| count.parse().ok());
        } else if let Some(rest) = inner.strip_prefix("  ") {
            let last = shown.last_mut().unwrap_or_else(|| panic!("{text}"));
            last.push(' ');
            last.push_str(rest.trim());
        }
    }
    assert!(!shown.is_empty() && more.is_some(), "{text}");
    for note in &shown {
        assert!(notes.contains(note), "{note:?} is not a whole note:\n{text}");
    }
    assert_eq!(more, Some(notes.len() - shown.len()), "{text}");
}

#[test]
fn a_small_screen_still_lists_the_first_issues() {
    // at 60 by 16 and 80 by 20 the issues list keeps room for its first issues: the detail and the notes take what is left
    let temp = Temp::new("issues-small");
    let dir = write_profile(&temp.path().join("run"), Shape::BEFORE);
    let mut app = open(&[&dir]);
    let found = app.issues().0.len();
    assert!(found >= 3, "{found}");
    for (cols, rows) in [(60, 16), (80, 20)] {
        let text = screen(&mut app, "9", cols, rows);
        let lines: Vec<&str> = text.lines().collect();
        let top = lines.iter().position(|line| line.contains("Issues")).unwrap_or_else(|| panic!("{text}"));
        let listed = lines[top + 1..].iter().take_while(|line| !line.starts_with('╰')).filter(|line| ["info", "warning", "serious", "critical"].iter().any(|word| line.contains(word))).count();
        assert!(listed >= found.min(3), "{cols}x{rows}: {listed} issues listed of {found}:\n{text}");
    }
}
