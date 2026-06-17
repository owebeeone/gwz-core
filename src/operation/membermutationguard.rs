use std::collections::HashSet;
use std::sync::Mutex;

use crate::model;



#[derive(Default)]
pub struct MemberLockManager {
    pub(crate) locked: Mutex<HashSet<String>>,
}

impl MemberLockManager {
    pub fn try_lock<'a>(&'a self, member_id: &model::MemberId) -> Option<MemberMutationGuard<'a>> {
        let mut locked = self.locked.lock().expect("member lock manager poisoned");
        if locked.insert(member_id.to_string()) {
            Some(MemberMutationGuard {
                manager: self,
                member_id: member_id.to_string(),
            })
        } else {
            None
        }
    }
}

pub struct MemberMutationGuard<'a> {
    pub(crate) manager: &'a MemberLockManager,
    pub(crate) member_id: String,
}

impl Drop for MemberMutationGuard<'_> {
    fn drop(&mut self) {
        self.manager
            .locked
            .lock()
            .expect("member lock manager poisoned")
            .remove(&self.member_id);
    }
}

