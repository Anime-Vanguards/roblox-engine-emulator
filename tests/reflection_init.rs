//! Reflection::shared() initializes a process-wide OnceLock, so this regression has its own test binary.

use rayon::prelude::*;
use roblox_engine_emulator::reflection::Reflection;

/// Calling Reflection::shared() from inside a rayon parallel iterator on first use must not deadlock
/// by stealing another Reflection::shared() task while holding the OnceLock initialization lock.
#[test]
fn shared_reflection_initializes_inside_rayon_par_iter() {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().expect("rayon pool");
        let counts: Vec<usize> = pool.install(|| {
            (0..32)
                .into_par_iter()
                .map(|_| Reflection::shared().expect("reflection").class_count())
                .collect()
        });
        let _ = done_tx.send(counts);
    });
    let counts = done_rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .expect("Reflection::shared() deadlocked inside rayon par_iter");
    worker.join().expect("worker thread panicked");
    assert_eq!(counts.len(), 32);
    assert!(counts[0] > 0);
    assert!(counts.iter().all(|&count| count == counts[0]));
}
