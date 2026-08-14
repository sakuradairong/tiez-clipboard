//! Storage-neutral clipboard mutation policies shared by native TieZ frontends.
//!
//! Persistence, UI events, and synchronization remain platform-adapter
//! responsibilities. This module owns the deterministic session-history rules
//! and the positive-ID persistence contract.

use std::collections::VecDeque;

/// Minimal view of a session-history entry needed by mutation policies.
pub trait MutationRecord {
    fn id(&self) -> i64;
    fn is_pinned(&self) -> bool;
    fn has_tags(&self) -> bool;
}

/// Mutable entry contract used only by pin planning.
///
/// This remains separate from [`MutationRecord`] so read/delete/clear callers
/// do not inherit pin-specific mutation requirements.
pub trait PinMutationRecord: Clone {
    fn id(&self) -> i64;
    fn set_id(&mut self, id: i64);
    fn set_pinned(&mut self, is_pinned: bool);
}

/// Mutable entry contract used only by session-only tag persistence.
pub trait TagMutationRecord: Clone {
    fn id(&self) -> i64;
    fn set_id(&mut self, id: i64);
    fn set_tags(&mut self, tags: Vec<String>);
}

/// Persistent work required after applying a pin request to session history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PinStoragePlan<T> {
    SessionOnly,
    ToggleExisting { entry_id: i64 },
    PersistThenToggle { session_id: i64, entry: T },
}

/// Prepared session-only entry and requested tags awaiting persistence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTagPlan<T> {
    pub session_id: i64,
    pub entry: T,
    pub requested_tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagPlanError {
    SessionEntryNotFound,
}

/// Privacy work required after a persisted entry's tags change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensitivityTransition {
    Unchanged,
    Encrypt,
    Decrypt,
}

/// Result of planning and applying a delete to session history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeletePlan {
    pub removed_from_session: usize,
    pub persisted_id: Option<i64>,
}

/// Remove the entry from session history and identify whether persistent
/// storage must also be updated.
pub fn plan_delete<T: MutationRecord>(items: &mut VecDeque<T>, entry_id: i64) -> DeletePlan {
    let previous_len = items.len();
    items.retain(|item| item.id() != entry_id);

    DeletePlan {
        removed_from_session: previous_len - items.len(),
        persisted_id: (entry_id > 0).then_some(entry_id),
    }
}

/// Clear unprotected session entries while preserving pinned or tagged items.
///
/// Returns the number of session entries removed.
pub fn clear_unprotected<T: MutationRecord>(items: &mut VecDeque<T>) -> usize {
    let previous_len = items.len();
    items.retain(|item| item.is_pinned() || item.has_tags());
    previous_len - items.len()
}

/// Apply the requested pin state to session history and describe the storage
/// work needed to make that state durable.
pub fn plan_pin<T: PinMutationRecord>(
    items: &mut VecDeque<T>,
    entry_id: i64,
    is_pinned: bool,
) -> PinStoragePlan<T> {
    let session_entry = items.iter_mut().find(|item| item.id() == entry_id);
    if let Some(item) = session_entry {
        item.set_pinned(is_pinned);
        if entry_id < 0 && is_pinned {
            return PinStoragePlan::PersistThenToggle {
                session_id: entry_id,
                entry: item.clone(),
            };
        }
    }

    if entry_id > 0 {
        PinStoragePlan::ToggleExisting { entry_id }
    } else {
        PinStoragePlan::SessionOnly
    }
}

/// Replace a session-only ID after persistence assigns its stable positive ID.
pub fn replace_session_id<T: PinMutationRecord>(
    items: &mut VecDeque<T>,
    session_id: i64,
    persisted_id: i64,
) -> bool {
    let Some(item) = items.iter_mut().find(|item| item.id() == session_id) else {
        return false;
    };
    item.set_id(persisted_id);
    true
}

/// Prepare a session-only entry for persistence without changing the live
/// session until storage succeeds.
pub fn plan_session_tags<T: TagMutationRecord>(
    items: &VecDeque<T>,
    session_id: i64,
    requested_tags: Vec<String>,
) -> Result<SessionTagPlan<T>, TagPlanError> {
    let Some(item) = items.iter().find(|item| item.id() == session_id) else {
        return Err(TagPlanError::SessionEntryNotFound);
    };

    let mut entry = item.clone();
    entry.set_tags(requested_tags.clone());
    Ok(SessionTagPlan {
        session_id,
        entry,
        requested_tags,
    })
}

/// Apply the stable ID and requested tags after session-only persistence
/// succeeds.
pub fn complete_session_tags<T: TagMutationRecord>(
    items: &mut VecDeque<T>,
    session_id: i64,
    persisted_id: i64,
    requested_tags: Vec<String>,
) -> bool {
    let Some(item) = items.iter_mut().find(|item| item.id() == session_id) else {
        return false;
    };
    item.set_id(persisted_id);
    item.set_tags(requested_tags);
    true
}

pub fn plan_sensitivity_transition(
    was_sensitive: bool,
    is_sensitive: bool,
) -> SensitivityTransition {
    match (was_sensitive, is_sensitive) {
        (false, true) => SensitivityTransition::Encrypt,
        (true, false) => SensitivityTransition::Decrypt,
        _ => SensitivityTransition::Unchanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestRecord {
        id: i64,
        is_pinned: bool,
        has_tags: bool,
    }

    impl TestRecord {
        fn new(id: i64) -> Self {
            Self {
                id,
                is_pinned: false,
                has_tags: false,
            }
        }
    }

    impl MutationRecord for TestRecord {
        fn id(&self) -> i64 {
            self.id
        }

        fn is_pinned(&self) -> bool {
            self.is_pinned
        }

        fn has_tags(&self) -> bool {
            self.has_tags
        }
    }

    impl PinMutationRecord for TestRecord {
        fn id(&self) -> i64 {
            self.id
        }

        fn set_id(&mut self, id: i64) {
            self.id = id;
        }

        fn set_pinned(&mut self, is_pinned: bool) {
            self.is_pinned = is_pinned;
        }
    }

    impl TagMutationRecord for TestRecord {
        fn id(&self) -> i64 {
            self.id
        }

        fn set_id(&mut self, id: i64) {
            self.id = id;
        }

        fn set_tags(&mut self, tags: Vec<String>) {
            self.has_tags = !tags.is_empty();
        }
    }

    #[test]
    fn deleting_session_only_entry_does_not_plan_persistent_delete() {
        let mut items = VecDeque::from([TestRecord::new(-1), TestRecord::new(1)]);

        let plan = plan_delete(&mut items, -1);

        assert_eq!(plan.removed_from_session, 1);
        assert_eq!(plan.persisted_id, None);
        assert_eq!(
            items.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn deleting_positive_id_updates_session_and_persistent_storage() {
        let mut items = VecDeque::from([TestRecord::new(1), TestRecord::new(2)]);

        let plan = plan_delete(&mut items, 1);

        assert_eq!(plan.removed_from_session, 1);
        assert_eq!(plan.persisted_id, Some(1));
        assert_eq!(
            items.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn clear_preserves_pinned_and_tagged_session_entries() {
        let mut pinned = TestRecord::new(-2);
        pinned.is_pinned = true;
        let mut tagged = TestRecord::new(-3);
        tagged.has_tags = true;
        let mut items = VecDeque::from([TestRecord::new(-1), pinned, tagged]);

        let removed = clear_unprotected(&mut items);

        assert_eq!(removed, 1);
        assert_eq!(
            items.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![-2, -3]
        );
    }

    #[test]
    fn zero_id_is_never_treated_as_persisted() {
        let mut items = VecDeque::from([TestRecord::new(0)]);

        let plan = plan_delete(&mut items, 0);

        assert_eq!(plan.removed_from_session, 1);
        assert_eq!(plan.persisted_id, None);
    }

    #[test]
    fn pinning_session_entry_plans_persistence_with_updated_state() {
        let mut items = VecDeque::from([TestRecord::new(-1)]);

        let plan = plan_pin(&mut items, -1, true);

        assert_eq!(
            plan,
            PinStoragePlan::PersistThenToggle {
                session_id: -1,
                entry: TestRecord {
                    id: -1,
                    is_pinned: true,
                    has_tags: false,
                },
            }
        );
        assert!(items.front().unwrap().is_pinned);
    }

    #[test]
    fn unpinning_persisted_entry_plans_existing_row_update() {
        let mut pinned = TestRecord::new(1);
        pinned.is_pinned = true;
        let mut items = VecDeque::from([pinned]);

        let plan = plan_pin(&mut items, 1, false);

        assert_eq!(plan, PinStoragePlan::ToggleExisting { entry_id: 1 });
        assert!(!items.front().unwrap().is_pinned);
    }

    #[test]
    fn unpinning_session_entry_stays_session_only() {
        let mut pinned = TestRecord::new(-1);
        pinned.is_pinned = true;
        let mut items = VecDeque::from([pinned]);

        let plan = plan_pin(&mut items, -1, false);

        assert_eq!(plan, PinStoragePlan::SessionOnly);
        assert!(!items.front().unwrap().is_pinned);
    }

    #[test]
    fn persisted_id_replaces_matching_session_id() {
        let mut items = VecDeque::from([TestRecord::new(-1), TestRecord::new(-2)]);

        let replaced = replace_session_id(&mut items, -1, 42);

        assert!(replaced);
        assert_eq!(
            items.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![42, -2]
        );
    }

    #[test]
    fn session_tag_plan_does_not_mutate_live_entry_before_save() {
        let items = VecDeque::from([TestRecord::new(-1)]);

        let plan = plan_session_tags(&items, -1, vec!["work".to_owned()]).unwrap();

        assert_eq!(plan.session_id, -1);
        assert!(plan.entry.has_tags);
        assert_eq!(plan.requested_tags, vec!["work"]);
        assert!(!items.front().unwrap().has_tags);
    }

    #[test]
    fn missing_session_tag_target_is_reported() {
        let items = VecDeque::from([TestRecord::new(-1)]);

        let result = plan_session_tags(&items, -2, vec!["work".to_owned()]);

        assert_eq!(result, Err(TagPlanError::SessionEntryNotFound));
    }

    #[test]
    fn completed_session_tags_apply_stable_id_and_tags() {
        let mut items = VecDeque::from([TestRecord::new(-1)]);

        let completed = complete_session_tags(&mut items, -1, 42, vec!["work".to_owned()]);

        assert!(completed);
        let item = items.front().unwrap();
        assert_eq!(item.id, 42);
        assert!(item.has_tags);
    }

    #[test]
    fn sensitivity_transition_plans_privacy_work() {
        assert_eq!(
            plan_sensitivity_transition(false, true),
            SensitivityTransition::Encrypt
        );
        assert_eq!(
            plan_sensitivity_transition(true, false),
            SensitivityTransition::Decrypt
        );
        assert_eq!(
            plan_sensitivity_transition(false, false),
            SensitivityTransition::Unchanged
        );
        assert_eq!(
            plan_sensitivity_transition(true, true),
            SensitivityTransition::Unchanged
        );
    }
}
