use super::*;

/// The message the compaction pipeline injects in place of the compacted turns.
const SUMMARY: &str = "This session is being continued from a previous conversation that ran out of context. \
                       The summary below covers the earlier portion of the conversation.\n\nSummary:\n1. Fix the parser";

fn checkpoint(message: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "checkpoint_id": "ckpt",
        "prompt_index_at_compaction": 3,
        "compacted_history": [message],
        "schema_version": 1,
        "created_at": "2026-07-01T00:00:00Z",
    }))
    .expect("serialize checkpoint")
}

fn user_message(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "user", "content": [{ "type": "text", "text": text }] })
}

/// Write `bytes` as `<dir>/<stem>.json`, creating `dir`.
fn write_checkpoint(dir: &Path, stem: &str, bytes: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).expect("checkpoints dir");
    let path = dir.join(format!("{stem}.json"));
    std::fs::write(&path, bytes).expect("write checkpoint");
    path
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn reads_summary_from_array_content() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    write_checkpoint(&dir, "ckpt", &checkpoint(user_message(SUMMARY)));

    assert_eq!(
        load_summary_from_dir(&dir).as_deref(),
        Some(SUMMARY.trim()),
        "a user message carries the summary as text parts"
    );
}

#[test]
fn reads_summary_from_string_content() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    write_checkpoint(
        &dir,
        "ckpt",
        &checkpoint(serde_json::json!({ "type": "assistant", "content": SUMMARY })),
    );

    assert_eq!(
        load_summary_from_dir(&dir).as_deref(),
        Some(SUMMARY.trim()),
        "an assistant message carries the summary as a plain string"
    );
}

#[test]
fn missing_directory_yields_none() {
    let home = tempdir();

    assert_eq!(
        load_summary_from_dir(&home.path().join("compaction_checkpoints")),
        None
    );
}

#[test]
fn invalid_json_yields_none() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    write_checkpoint(&dir, "ckpt", b"{ not json");

    assert_eq!(load_summary_from_dir(&dir), None);
}

#[test]
fn checkpoint_without_a_summary_message_yields_none() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    write_checkpoint(
        &dir,
        "ckpt",
        &checkpoint(user_message("just a normal turn")),
    );

    assert_eq!(load_summary_from_dir(&dir), None);
}

#[test]
fn newest_checkpoint_wins() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    let older = write_checkpoint(&dir, "older", &checkpoint(user_message("older summary")));
    let newer = write_checkpoint(&dir, "newer", &checkpoint(user_message(SUMMARY)));
    // Explicit mtimes: same-second writes are not ordered by the filesystem on every platform.
    set_mtime(&older, 1_700_000_000);
    set_mtime(&newer, 1_700_000_100);

    assert_eq!(load_summary_from_dir(&dir).as_deref(), Some(SUMMARY.trim()));
}

#[test]
fn oversized_checkpoint_is_skipped_for_the_next_one() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    let small = write_checkpoint(&dir, "small", &checkpoint(user_message(SUMMARY)));
    let huge = write_checkpoint(&dir, "huge", &checkpoint(user_message("huge summary")));
    // Sparse: only the reported length matters to the size cap.
    let file = std::fs::File::options()
        .write(true)
        .open(&huge)
        .expect("open huge checkpoint");
    file.set_len(MAX_CHECKPOINT_BYTES + 1).expect("grow file");
    drop(file);
    set_mtime(&small, 1_700_000_000);
    set_mtime(&huge, 1_700_000_100);

    assert_eq!(
        load_summary_from_dir(&dir).as_deref(),
        Some(SUMMARY.trim()),
        "the newest file is over the size cap, so the next one answers"
    );
}

#[test]
fn wait_for_summary_waits_for_a_checkpoint_that_lands_late() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");
    let writer_dir = dir.clone();
    let bytes = checkpoint(user_message(SUMMARY));
    // The shell queues the checkpoint to its persistence thread, so the file can appear after the compaction is
    // already reported.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        write_checkpoint(&writer_dir, "ckpt", &bytes);
    });

    assert_eq!(
        wait_for_summary(&dir, Duration::from_secs(2)).as_deref(),
        Some(SUMMARY.trim()),
        "a checkpoint that lands late is still picked up"
    );
}

#[test]
fn wait_for_summary_gives_up_after_its_budget() {
    let home = tempdir();
    let dir = home.path().join("compaction_checkpoints");

    assert_eq!(wait_for_summary(&dir, Duration::from_millis(100)), None);
    assert_eq!(
        wait_for_summary(&dir, Duration::ZERO),
        None,
        "a zero budget reads once"
    );
}

fn set_mtime(path: &Path, unix_secs: u64) {
    let modified = std::time::UNIX_EPOCH + Duration::from_secs(unix_secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(modified)
        .expect("set mtime");
}
