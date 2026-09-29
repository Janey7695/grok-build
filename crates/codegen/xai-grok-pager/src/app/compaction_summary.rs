//! Reads the summary a finished compaction left in the session's checkpoint file.
//!
//! The compaction notifications the shell sends carry only token counts, so the summary text has to come
//! back off disk (`<session_dir>/compaction_checkpoints/<uuid>.json`). Best-effort by design: every
//! failure returns `None` so a missing or still-arriving checkpoint can never break the transcript.

use std::path::{Path, PathBuf};
use std::time::Duration;

use xai_grok_shell::extensions::notification::CompactionCheckpointFile;
use xai_grok_shell::sampling::{ContentPart, ConversationItem};

use crate::app::agent::AgentSession;
use crate::scrollback::RenderBlock;
use crate::scrollback::blocks::SessionEvent;
use crate::scrollback::state::ScrollbackState;

/// Opens the summary user message the compaction pipeline injects in place of the compacted turns.
const SUMMARY_MARKER: &str = "This session is being continued from a previous conversation";

/// Skip checkpoints above this size. Real files are tens of KB; a larger one is not worth reading on the UI thread.
const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

/// Poll interval while waiting for a just-finished compaction's checkpoint.
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// How long the manual `/compact` path waits for its checkpoint. That read runs on the blocking pool, so the
/// budget can be generous: the shell queues the file to its persistence thread and the RPC can answer first.
pub(crate) const MANUAL_COMPACT_WAIT: Duration = Duration::from_secs(2);

/// How long a path that pushes straight at the completion notification waits. Short, because it runs on the
/// event loop and a checkpoint that is already on disk answers on the first attempt.
const IMMEDIATE_WAIT: Duration = Duration::from_millis(120);

/// `compaction_checkpoints` directory of `session_id` under `cwd`, built the same way the shell builds its session directory.
pub(crate) fn checkpoints_dir(session_id: &str, cwd: &Path) -> PathBuf {
    let encoded = xai_grok_shell::util::grok_home::encode_cwd_dirname(&cwd.to_string_lossy());
    xai_grok_shell::util::grok_home::grok_home()
        .join("sessions")
        .join(encoded)
        .join(session_id)
        .join("compaction_checkpoints")
}

/// Summary body from the newest checkpoint of a session, waiting up to `budget` for one to appear.
pub(crate) fn load_compaction_summary(
    session_id: &str,
    cwd: &Path,
    budget: Duration,
) -> Option<String> {
    wait_for_summary(&checkpoints_dir(session_id, cwd), budget)
}

/// Summary body from the newest checkpoint in `dir`, waiting up to `budget` for one to appear.
/// The checkpoint is queued to the shell's persistence thread, so a caller that runs the moment a compaction
/// finishes can beat its own file. `Duration::ZERO` reads once.
pub(crate) fn wait_for_summary(dir: &Path, budget: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if let Some(summary) = read_newest_summary(dir) {
            return Some(summary);
        }
        if std::time::Instant::now() >= deadline {
            tracing::debug!(dir = %dir.display(), "no compaction summary before the wait ran out");
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Summary body from the newest checkpoint in an already-resolved checkpoints directory. `None` when it cannot be read.
pub(crate) fn load_summary_from_dir(dir: &Path) -> Option<String> {
    read_newest_summary(dir)
}

/// Push the folded summary block for `session`'s newest checkpoint, when one can be read.
/// Called right below the "Context compacted" line of a finished compaction.
pub(crate) fn push_session_summary(scrollback: &mut ScrollbackState, session: &AgentSession) {
    let Some(session_id) = session.session_id.as_ref() else {
        return;
    };
    if let Some(summary) =
        load_compaction_summary(session_id.0.as_ref(), &session.cwd, IMMEDIATE_WAIT)
    {
        push_summary(scrollback, summary);
    }
}

/// Push the folded summary block for an already-resolved checkpoints directory.
/// The tracker defers the completion past the session's context, so it carries the directory instead.
pub(crate) fn push_summary_from_dir(scrollback: &mut ScrollbackState, dir: &Path) {
    if let Some(summary) = wait_for_summary(dir, IMMEDIATE_WAIT) {
        push_summary(scrollback, summary);
    }
}

fn push_summary(scrollback: &mut ScrollbackState, summary: String) {
    scrollback.push_block(RenderBlock::session_event(
        SessionEvent::CompactionSummary { summary },
    ));
}

fn read_newest_summary(dir: &Path) -> Option<String> {
    let path = newest_checkpoint(dir)?;
    match std::fs::read(&path) {
        Ok(bytes) => parse_summary(&bytes).or_else(|| {
            tracing::debug!(path = %path.display(), "compaction checkpoint has no summary message");
            None
        }),
        Err(error) => {
            tracing::debug!(path = %path.display(), %error, "failed to read compaction checkpoint");
            None
        }
    }
}

/// Newest `*.json` in `dir` by mtime, ignoring oversized files.
fn newest_checkpoint(dir: &Path) -> Option<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::debug!(dir = %dir.display(), %error, "no compaction checkpoints to read");
            return None;
        }
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            if metadata.len() > MAX_CHECKPOINT_BYTES {
                tracing::debug!(
                    path = %entry.path().display(),
                    bytes = metadata.len(),
                    "skipping oversized compaction checkpoint"
                );
                return None;
            }
            Some((metadata.modified().ok()?, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// The summary message of a checkpoint file, or `None` when it holds none.
fn parse_summary(bytes: &[u8]) -> Option<String> {
    match serde_json::from_slice::<CompactionCheckpointFile>(bytes) {
        Ok(checkpoint) => checkpoint
            .compacted_history
            .iter()
            .find_map(summary_message),
        Err(error) => {
            tracing::debug!(%error, "failed to parse compaction checkpoint");
            None
        }
    }
}

/// The text of `item` when it is the compaction summary message.
fn summary_message(item: &ConversationItem) -> Option<String> {
    let text = match item {
        ConversationItem::User(user) => user
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_ref()),
                ContentPart::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ConversationItem::Assistant(assistant) => assistant.content.to_string(),
        _ => return None,
    };
    text.contains(SUMMARY_MARKER)
        .then(|| text.trim().to_string())
}

#[cfg(test)]
#[path = "compaction_summary_tests.rs"]
mod tests;
