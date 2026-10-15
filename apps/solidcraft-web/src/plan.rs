//! What to do with one design when the browser's copy and the server's copy may differ. Pure, so
//! it is tested without a browser; `sync.rs` does the fetching and storing.
//!
//! Per design the app remembers `synced = (server modified, browser modified)` as of the last
//! time the two were known to be the same. A side "changed" when its modified time is no longer
//! the remembered one. Nothing is ever dropped silently: a design changed on both sides keeps
//! both versions (the older one under a new name), and the server keeps every replaced version.

/// Longest design name the server takes (it refuses more).
const MAX_NAME_BYTES: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Store the server's design in the browser.
    Pull,
    /// Send the browser's design to the server.
    Push,
    /// The browser's design was deleted here since the last sync: delete it on the server too.
    DeleteRemote,
    /// The server's design was deleted elsewhere and not changed here: delete it here too.
    DeleteLocal,
    /// Changed on both sides.
    Conflict,
}

/// `local` / `remote`: modified times (ms) of the design where it exists.
pub fn decide(local: Option<u64>, remote: Option<u64>, synced: Option<(u64, u64)>) -> Action {
    match (local, remote) {
        (None, None) => Action::Nothing,
        (None, Some(r)) => match synced {
            // Gone here, untouched on the server since: it was deleted here.
            Some((sr, _)) if sr == r => Action::DeleteRemote,
            _ => Action::Pull,
        },
        (Some(l), None) => match synced {
            // Gone from the server, untouched here since: it was deleted elsewhere.
            Some((_, sl)) if sl == l => Action::DeleteLocal,
            _ => Action::Push,
        },
        (Some(l), Some(r)) => {
            let remote_changed = synced.is_none_or(|(sr, _)| sr != r);
            let local_changed = synced.is_none_or(|(_, sl)| sl != l);
            match (remote_changed, local_changed) {
                (false, false) => Action::Nothing,
                (true, false) => Action::Pull,
                (false, true) => Action::Push,
                (true, true) => Action::Conflict,
            }
        }
    }
}

/// Is `name` one the server accepts (one plain file name)?
pub fn syncable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.starts_with('.')
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}

/// `name` with a label before its extension: `part.solidcraft` -> `part (label).solidcraft`.
pub fn labelled(name: &str, label: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({label}).{ext}"),
        _ => format!("{name} ({label})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_contact_with_equal_sides_is_a_conflict_to_compare_bytes() {
        // Never synced, present on both sides: the caller compares the bytes before deciding.
        assert_eq!(decide(Some(10), Some(20), None), Action::Conflict);
    }

    #[test]
    fn unchanged_sides_do_nothing() {
        assert_eq!(decide(Some(10), Some(20), Some((20, 10))), Action::Nothing);
    }

    #[test]
    fn one_changed_side_wins() {
        assert_eq!(decide(Some(10), Some(30), Some((20, 10))), Action::Pull);
        assert_eq!(decide(Some(15), Some(20), Some((20, 10))), Action::Push);
    }

    #[test]
    fn both_changed_is_a_conflict() {
        assert_eq!(decide(Some(15), Some(30), Some((20, 10))), Action::Conflict);
    }

    #[test]
    fn new_designs_move_to_the_other_side() {
        assert_eq!(decide(None, Some(20), None), Action::Pull);
        assert_eq!(decide(Some(10), None, None), Action::Push);
    }

    #[test]
    fn deletes_follow_only_when_the_other_side_is_untouched() {
        // Deleted here, server copy unchanged since the last sync.
        assert_eq!(decide(None, Some(20), Some((20, 10))), Action::DeleteRemote);
        // Deleted here, but the server's copy changed meanwhile: keep it.
        assert_eq!(decide(None, Some(25), Some((20, 10))), Action::Pull);
        // Deleted on the server, browser copy unchanged since the last sync.
        assert_eq!(decide(Some(10), None, Some((20, 10))), Action::DeleteLocal);
        // Deleted on the server, but edited here since: send it back.
        assert_eq!(decide(Some(12), None, Some((20, 10))), Action::Push);
        assert_eq!(decide(None, None, Some((20, 10))), Action::Nothing);
    }

    #[test]
    fn names_the_server_refuses_are_not_synced() {
        for bad in ["", ".hidden", "a/b", "a\\b", "c:x", "tab\tx", &"x".repeat(201)] {
            assert!(!syncable(bad), "{bad:?}");
        }
        assert!(syncable("My part (2).solidcraft"));
    }

    #[test]
    fn labels_go_before_the_extension() {
        assert_eq!(labelled("part.solidcraft", "server copy"), "part (server copy).solidcraft");
        assert_eq!(labelled("part", "server copy"), "part (server copy)");
        assert_eq!(labelled(".solidcraft", "x"), ".solidcraft (x)");
    }
}
