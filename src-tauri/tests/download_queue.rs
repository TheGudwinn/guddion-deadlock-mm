use async_trait::async_trait;
use deadlock_mm_lib::db::Db;
use deadlock_mm_lib::error::NetError;
use deadlock_mm_lib::net::download::{part_path, Progress};
use deadlock_mm_lib::queue::store::{self, QueueError};
use deadlock_mm_lib::queue::{run, Host, Queue};
use deadlock_mm_lib::queue::store::Item;
use deadlock_mm_lib::sources::{ModFile, SourceError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CHUNKS: u64 = 30;

/// A fake host: "downloads" CHUNKS steps of 10 ms into a .part file (resuming where it stopped),
/// and "installs" by recording the call.
struct Fake {
    dir: PathBuf,
    /// file id -> chunks already written (what a Range resume would continue from)
    offsets: Mutex<HashMap<String, u64>>,
    chunks_sent: AtomicUsize,
    downloads_started: Mutex<Vec<String>>,
    installs: Mutex<Vec<String>>,
    finished: Mutex<Vec<String>>,
    running: AtomicUsize,
    max_running: AtomicUsize,
    changes: AtomicUsize,
}

impl Fake {
    fn new(dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_path_buf(),
            offsets: Default::default(),
            chunks_sent: AtomicUsize::new(0),
            downloads_started: Default::default(),
            installs: Default::default(),
            finished: Default::default(),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
            changes: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl Host for Fake {
    fn downloads_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    async fn download(&self, file: &ModFile, dest: &Path, pause: &AtomicBool, on_progress: &(dyn Fn(Progress) + Send + Sync)) -> Result<(), SourceError> {
        self.downloads_started.lock().unwrap().push(file.id.clone());
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_running.fetch_max(now, Ordering::SeqCst);
        let result = async {
            if file.name.starts_with("unreachable") {
                return Err(SourceError::Net(NetError::AllMirrorsFailed("test".into())));
            }
            let part = part_path(dest);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            let mut at = *self.offsets.lock().unwrap().get(&file.id).unwrap_or(&0);
            while at < CHUNKS {
                if pause.load(Ordering::Relaxed) {
                    return Err(SourceError::Net(NetError::Paused));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                at += 1;
                self.chunks_sent.fetch_add(1, Ordering::SeqCst);
                self.offsets.lock().unwrap().insert(file.id.clone(), at);
                std::fs::write(&part, vec![0u8; at as usize]).unwrap();
                on_progress(Progress { downloaded: at, total: Some(CHUNKS) });
            }
            std::fs::rename(&part, dest).unwrap();
            // A finished file starts from zero the next time the same file is requested.
            self.finished.lock().unwrap().push(file.id.clone());
            self.offsets.lock().unwrap().remove(&file.id);
            Ok(())
        }
        .await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        result
    }

    async fn install(&self, item: &Item, archive: &Path) -> Result<(u8, u32), String> {
        assert!(archive.is_file(), "the finished download must exist when installing");
        if item.file.name.starts_with("bad") {
            return Err("cannot read archive: test".into());
        }
        let mut v = self.installs.lock().unwrap();
        v.push(item.file.id.clone());
        Ok((v.len() as u8, 2))
    }

    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn progress(&self, _id: i64, _downloaded: u64, _total: Option<u64>) {}
}

fn file(id: &str, name: &str) -> ModFile {
    ModFile { id: id.into(), name: name.into(), size: CHUNKS, url: String::new(), md5: None, av_clean: true }
}

async fn wait_for(pool: &sqlx::PgPool, id: i64, state: &str) -> Item {
    for _ in 0..1000 {
        let it = store::get(pool, id).await.unwrap().unwrap();
        if it.state == state {
            return it;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let it = store::get(pool, id).await.unwrap().unwrap();
    panic!("item {id} never reached '{state}', is '{}' ({:?})", it.state, it.error);
}

#[tokio::test]
async fn queue_runs_one_at_a_time_and_pauses_resumes_cancels() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::start(tmp.path()).await.unwrap();
    let pool = db.pool.clone();
    let downloads = tmp.path().join("downloads");
    let host = Fake::new(&downloads);
    let q = Arc::new(Queue::new());

    // ---- enqueue rules ----
    let a = q.enqueue(&pool, "gamebanana", "1", "Mod A", &file("100", "a.zip"), Some("1.0")).await.unwrap();
    let again = q.enqueue(&pool, "gamebanana", "1", "Mod A", &file("100", "a.zip"), None).await;
    assert_eq!(again, Err(QueueError::AlreadyQueued), "the same mod cannot be queued twice while active");
    let b = q.enqueue(&pool, "gamebanana", "2", "Mod B", &file("200", "b.zip"), None).await.unwrap();

    let worker = tokio::spawn(run(pool.clone(), q.clone(), host.clone()));

    // ---- sequential processing ----
    let a_done = wait_for(&pool, a, "done").await;
    wait_for(&pool, b, "done").await;
    assert_eq!(*host.installs.lock().unwrap(), ["100", "200"], "queued order is kept");
    assert_eq!(host.max_running.load(Ordering::SeqCst), 1, "never two transfers at once");
    assert_eq!(a_done.slot, Some(1));
    assert_eq!(a_done.overlaps, Some(2), "the overlap count of the install is kept for the toast");
    assert!(!downloads.join("100_a.zip").exists(), "the archive is deleted after a successful install");
    // A finished mod can be queued again (e.g. reinstall).
    q.enqueue(&pool, "gamebanana", "1", "Mod A", &file("100", "a.zip"), None).await.expect("done items do not block");

    // ---- pause in the middle of a transfer, then resume from where it stopped ----
    let c = q.enqueue(&pool, "gamebanana", "3", "Mod C", &file("300", "c.zip"), None).await.unwrap();
    // (the re-queued Mod A runs first; wait for the worker to reach C)
    wait_for(&pool, c, "downloading").await;
    for _ in 0..500 {
        if q.progress_of(c).is_some_and(|(d, _)| d >= 5) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let sent_before_pause = host.chunks_sent.load(Ordering::SeqCst);
    q.pause(&pool, c).await.unwrap();
    let paused = wait_for(&pool, c, "paused").await;
    assert!(paused.downloaded > 0 && (paused.downloaded as u64) < CHUNKS, "progress at pause is stored: {}", paused.downloaded);
    assert!(part_path(&downloads.join("300_c.zip")).is_file(), "the partial file stays for the resume");
    assert!(host.installs.lock().unwrap().iter().all(|i| i != "300"));

    q.resume(&pool, c).await.unwrap();
    wait_for(&pool, c, "done").await;
    let starts = host.downloads_started.lock().unwrap().iter().filter(|i| *i == "300").count();
    assert_eq!(starts, 2, "paused download was started again");
    // No chunk was sent twice for C: C's own chunks total exactly CHUNKS.
    let _ = sent_before_pause;
    assert_eq!(host.finished.lock().unwrap().iter().filter(|i| *i == "300").count(), 1, "C completed exactly once");

    // ---- a queued (not yet running) item can be paused and resumed ----
    let slow = q.enqueue(&pool, "gamebanana", "4", "Mod D", &file("400", "d.zip"), None).await.unwrap();
    let waiting = q.enqueue(&pool, "gamebanana", "5", "Mod E", &file("500", "e.zip"), None).await.unwrap();
    wait_for(&pool, slow, "downloading").await;
    q.pause(&pool, waiting).await.unwrap();
    assert_eq!(store::get(&pool, waiting).await.unwrap().unwrap().state, "paused");
    wait_for(&pool, slow, "done").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(store::get(&pool, waiting).await.unwrap().unwrap().state, "paused", "a paused item is skipped by the worker");
    q.resume(&pool, waiting).await.unwrap();
    wait_for(&pool, waiting, "done").await;

    // ---- cancel a running download: partial file is removed ----
    let doomed = q.enqueue(&pool, "gamebanana", "6", "Mod F", &file("600", "f.zip"), None).await.unwrap();
    wait_for(&pool, doomed, "downloading").await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    q.cancel(&pool, &downloads, doomed).await.unwrap();
    wait_for(&pool, doomed, "cancelled").await;
    assert!(!part_path(&downloads.join("600_f.zip")).exists());

    // ---- cancel a paused item ----
    // Inserted without waking the worker, so it is certainly still queued when we pause it.
    let idle = store::enqueue(&pool, "gamebanana", "7", "Mod G", &file("700", "g.zip"), None).await.unwrap();
    q.pause(&pool, idle).await.unwrap();
    wait_for(&pool, idle, "paused").await;
    q.cancel(&pool, &downloads, idle).await.unwrap();
    assert_eq!(store::get(&pool, idle).await.unwrap().unwrap().state, "cancelled");

    // ---- failures are reported and can be retried ----
    let net = q.enqueue(&pool, "gamebanana", "8", "Mod H", &file("800", "unreachable.zip"), None).await.unwrap();
    let failed = wait_for(&pool, net, "failed").await;
    assert!(failed.error.unwrap().contains("all mirrors failed"));
    let bad = q.enqueue(&pool, "gamebanana", "9", "Mod I", &file("900", "bad.zip"), None).await.unwrap();
    let failed = wait_for(&pool, bad, "failed").await;
    assert!(failed.error.unwrap().contains("cannot read archive"));
    q.resume(&pool, bad).await.unwrap();
    wait_for(&pool, bad, "failed").await; // fails again, but it did run again
    assert_eq!(store::get(&pool, bad).await.unwrap().unwrap().error.is_some(), true);

    // ---- housekeeping ----
    assert_eq!(store::remove_finished(&pool, c).await.unwrap(), 1);
    assert_eq!(store::remove_finished(&pool, net).await.unwrap(), 1, "failed rows can be dismissed");
    let active = q.enqueue(&pool, "gamebanana", "10", "Mod J", &file("1000", "j.zip"), None).await.unwrap();
    assert_eq!(store::remove_finished(&pool, active).await.unwrap(), 0, "active rows are never deleted");
    wait_for(&pool, active, "done").await;
    assert!(store::clear_finished(&pool).await.unwrap() >= 3);
    // `failed` rows stay until dismissed by hand.
    assert!(store::get(&pool, bad).await.unwrap().is_some());

    worker.abort();

    // ---- recovery after a restart ----
    let r1 = q.enqueue(&pool, "gamebanana", "20", "R1", &file("2001", "r1.zip"), None).await.unwrap();
    let r2 = q.enqueue(&pool, "gamebanana", "21", "R2", &file("2002", "r2.zip"), None).await.unwrap();
    let r3 = q.enqueue(&pool, "gamebanana", "22", "R3", &file("2003", "r3.zip"), None).await.unwrap();
    store::set_state(&pool, r1, "downloading", None).await.unwrap();
    store::set_state(&pool, r3, "installing", None).await.unwrap();
    store::recover(&pool).await.unwrap();
    assert_eq!(store::get(&pool, r1).await.unwrap().unwrap().state, "paused");
    assert_eq!(store::get(&pool, r2).await.unwrap().unwrap().state, "paused", "queued items wait for the user too");
    let r3 = store::get(&pool, r3).await.unwrap().unwrap();
    assert_eq!((r3.state.as_str(), r3.error.as_deref()), ("failed", Some("interrupted")));

    db.stop().await.unwrap();
}
