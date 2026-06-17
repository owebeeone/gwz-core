



pub(crate) fn git_transfer_progress(stats: &git2::Progress) -> crate::GitTransferProgress {
    let received_objects = stats.received_objects();
    let total_objects = stats.total_objects();
    // libgit2's transfer callback hands the same counters for both phases; once
    // every object is received, remaining work is delta resolution.
    let phase = if total_objects > 0 && received_objects >= total_objects {
        crate::GitProgressPhase::Resolving
    } else {
        crate::GitProgressPhase::Receiving
    };
    crate::GitTransferProgress {
        phase,
        received_objects: Some(received_objects as i64),
        total_objects: Some(total_objects as i64),
        received_bytes: Some(stats.received_bytes() as i64),
        indexed_deltas: Some(stats.indexed_deltas() as i64),
        total_deltas: Some(stats.total_deltas() as i64),
    }
}

