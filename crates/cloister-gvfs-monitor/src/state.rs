//! Turns two snapshots of the vault list into GVfs events, in the order
//! clients expect (mount before volume on unlock, the reverse on removal).

use std::collections::BTreeMap;

use cloister_dbus::VaultInfo;

pub type Snapshot = BTreeMap<String, VaultInfo>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    VolumeAdded(VaultInfo),
    VolumeChanged(VaultInfo),
    VolumeRemoved(VaultInfo),
    MountAdded(VaultInfo),
    MountChanged(VaultInfo),
    MountRemoved(VaultInfo),
}

pub fn snapshot(vaults: Vec<VaultInfo>) -> Snapshot {
    vaults.into_iter().map(|v| (v.id.clone(), v)).collect()
}

/// Every vault shown as locked (the session service is gone).
pub fn all_locked(s: &Snapshot) -> Snapshot {
    s.iter()
        .map(|(id, v)| {
            let mut v = v.clone();
            v.state = cloister_dbus::STATE_LOCKED.into();
            v.mount_point.clear();
            (id.clone(), v)
        })
        .collect()
}

pub fn diff(old: &Snapshot, new: &Snapshot) -> Vec<Change> {
    use Change::*;
    let mut changes = Vec::new();
    for (id, o) in old {
        if !new.contains_key(id) {
            if o.is_unlocked() {
                changes.push(MountRemoved(o.clone()));
            }
            changes.push(VolumeRemoved(o.clone()));
        }
    }
    for (id, n) in new {
        match old.get(id) {
            None => {
                changes.push(VolumeAdded(n.clone()));
                if n.is_unlocked() {
                    changes.push(MountAdded(n.clone()));
                }
            }
            Some(o) if o == n => {}
            Some(o) => {
                match (o.is_unlocked(), n.is_unlocked()) {
                    (false, true) => changes.push(MountAdded(n.clone())),
                    (true, false) => changes.push(MountRemoved(o.clone())),
                    (true, true) if o.mount_point != n.mount_point => {
                        changes.push(MountRemoved(o.clone()));
                        changes.push(MountAdded(n.clone()));
                    }
                    (true, true) => changes.push(MountChanged(n.clone())),
                    (false, false) => {}
                }
                changes.push(VolumeChanged(n.clone()));
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::Change::*;
    use super::*;

    fn v(id: &str, name: &str, unlocked: bool) -> VaultInfo {
        VaultInfo {
            id: id.into(),
            name: name.into(),
            state: if unlocked { "unlocked" } else { "locked" }.into(),
            mount_point: if unlocked { format!("/run/{id}") } else { String::new() },
        }
    }

    #[test]
    fn unlock_then_lock() {
        let locked = snapshot(vec![v("a", "A", false)]);
        let unlocked = snapshot(vec![v("a", "A", true)]);
        assert_eq!(diff(&locked, &unlocked), [MountAdded(v("a", "A", true)), VolumeChanged(v("a", "A", true))]);
        assert_eq!(diff(&unlocked, &locked), [MountRemoved(v("a", "A", true)), VolumeChanged(v("a", "A", false))]);
        assert_eq!(diff(&unlocked, &all_locked(&unlocked)), diff(&unlocked, &locked));
    }

    #[test]
    fn add_remove_rename() {
        let empty = Snapshot::new();
        let one = snapshot(vec![v("a", "A", true)]);
        assert_eq!(diff(&empty, &one), [VolumeAdded(v("a", "A", true)), MountAdded(v("a", "A", true))]);
        assert_eq!(diff(&one, &empty), [MountRemoved(v("a", "A", true)), VolumeRemoved(v("a", "A", true))]);
        let renamed = snapshot(vec![v("a", "B", true)]);
        assert_eq!(diff(&one, &renamed), [MountChanged(v("a", "B", true)), VolumeChanged(v("a", "B", true))]);
        assert!(diff(&one, &one).is_empty());
    }
}
