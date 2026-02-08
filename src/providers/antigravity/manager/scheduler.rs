use crate::model_catalog::ModelCapabilities;
use crate::providers::antigravity::resource::AntigravityResource;
use crate::providers::manifest::AntigravityLease;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

/// Runtime credential = base data + dynamic capabilities.
#[derive(Debug, Clone)]
pub struct RuntimeCredential {
    /// Base credential data (persisted).
    pub inner: AntigravityResource,

    /// Dynamic capability bitset (runtime-only unless persisted elsewhere).
    pub caps: ModelCapabilities,
}

impl RuntimeCredential {
    /// Constructor: assign initial capabilities on load.
    /// Typically `ModelCapabilities::all()` to start optimistic and disable on errors.
    pub fn new(inner: AntigravityResource, initial_caps: ModelCapabilities) -> Self {
        Self {
            inner,
            caps: initial_caps,
        }
    }

    /// Proxy: check expiration via the inner credential.
    #[inline(always)]
    pub fn is_expired(&self) -> bool {
        self.inner.is_expired()
    }
}

pub type CredentialId = u64;
pub type ModelIndex = usize;

#[derive(Debug, Default)]
pub struct AssignmentResult {
    pub assigned: Option<AntigravityLease>,
    pub refresh_ids: Vec<CredentialId>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CooldownTicket(Reverse<Instant>, CredentialId, ModelIndex);

/// Core scheduling logic for credentials (no IO, no locks).
pub struct CredentialManager {
    creds: HashMap<CredentialId, RuntimeCredential>,
    queues: Vec<VecDeque<CredentialId>>,
    waiting_room: BinaryHeap<CooldownTicket>,
    cooldown_map: HashMap<(CredentialId, ModelIndex), Instant>,
    refreshing: HashSet<CredentialId>,
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new(0)
    }
}

impl CredentialManager {
    pub fn new(model_count: usize) -> Self {
        Self {
            creds: HashMap::new(),
            queues: vec![VecDeque::new(); model_count],
            waiting_room: BinaryHeap::new(),
            cooldown_map: HashMap::new(),
            refreshing: HashSet::new(),
        }
    }

    pub fn add_credential(
        &mut self,
        id: CredentialId,
        cred: AntigravityResource,
        initial_caps_bits: u64,
    ) {
        let initial_caps = ModelCapabilities::from_bits(initial_caps_bits);
        let caps = self
            .creds
            .get(&id)
            .map(|cred| cred.caps)
            .unwrap_or(initial_caps);

        self.creds.insert(id, RuntimeCredential::new(cred, caps));
        self.refreshing.remove(&id);

        for (index, queue) in self.queues.iter_mut().enumerate() {
            if !caps.supports(index) {
                continue;
            }

            if !queue.contains(&id) {
                queue.push_back(id);
            }
        }
    }

    fn index_from_mask(&self, model_mask: u64) -> Option<ModelIndex> {
        if model_mask == 0 || (model_mask & (model_mask - 1)) != 0 {
            return None;
        }
        let index = model_mask.trailing_zeros() as usize;
        if index >= self.queues.len() {
            return None;
        }
        Some(index)
    }

    pub fn mark_refreshing(&mut self, id: CredentialId) {
        self.refreshing.insert(id);
        self.clear_cooldowns_for(id);
    }

    pub fn mark_model_unsupported(
        &mut self,
        id: CredentialId,
        model_mask: u64,
    ) -> Option<(u64, u64)> {
        if model_mask == 0 {
            return None;
        }
        let cred = self.creds.get_mut(&id)?;
        let before = cred.caps.bits();
        cred.caps.disable_mask(model_mask);
        let after = cred.caps.bits();
        Some((before, after))
    }

    pub fn delete_credential(&mut self, id: CredentialId) {
        self.creds.remove(&id);
        self.refreshing.remove(&id);
        self.clear_cooldowns_for(id);
    }

    pub fn report_rate_limit(&mut self, id: CredentialId, model_mask: u64, cooldown: Duration) {
        let Some(model_index) = self.index_from_mask(model_mask) else {
            return;
        };
        let deadline = Instant::now() + cooldown;

        self.cooldown_map.insert((id, model_index), deadline);
        self.waiting_room
            .push(CooldownTicket(Reverse(deadline), id, model_index));
    }

    pub fn get_full_credential_copy(&self, id: CredentialId) -> Option<AntigravityResource> {
        self.creds.get(&id).map(|cred| cred.inner.clone())
    }

    pub fn project_id_of(&self, id: CredentialId) -> Option<String> {
        self.creds
            .get(&id)
            .map(|cred| cred.inner.project_id().to_string())
    }

    pub fn contains(&self, id: CredentialId) -> bool {
        self.creds.contains_key(&id)
    }

    pub fn get_assigned(&mut self, model_mask: u64) -> AssignmentResult {
        self.process_waiting_room();

        let mut result = AssignmentResult::default();

        let Some(model_index) = self.index_from_mask(model_mask) else {
            return result;
        };

        while let Some(id) = self.queues.get_mut(model_index).and_then(|q| q.pop_front()) {
            let Some(cred) = self.creds.get(&id) else {
                continue;
            };

            if !cred.caps.supports(model_index) {
                continue;
            }

            if self.refreshing.contains(&id) || self.is_model_cooling(id, model_index) {
                continue;
            }

            let Some(token) = cred
                .inner
                .access_token()
                .filter(|_| !cred.is_expired())
                .map(str::to_owned)
            else {
                result.refresh_ids.push(id);
                continue;
            };

            if let Some(queue) = self.queues.get_mut(model_index) {
                queue.push_back(id);
            }

            result.assigned = Some(AntigravityLease {
                id,
                project_id: cred.inner.project_id().to_string(),
                access_token: token,
            });
            return result;
        }
        result
    }

    fn process_waiting_room(&mut self) {
        let now = Instant::now();

        while self.waiting_room.peek().is_some_and(|t| (t.0).0 <= now) {
            let CooldownTicket(Reverse(ticket_deadline), credential_id, model_index) =
                self.waiting_room.pop().expect("peek guaranteed existence");

            match self.cooldown_map.entry((credential_id, model_index)) {
                std::collections::hash_map::Entry::Occupied(entry)
                    if ticket_deadline >= *entry.get() =>
                {
                    let ((reclaimed_cred_id, reclaimed_model_index), _) = entry.remove_entry();
                    if let Some(target_queue) = self.queues.get_mut(reclaimed_model_index) {
                        target_queue.push_back(reclaimed_cred_id);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn queue_len(&self, model_mask: u64) -> usize {
        self.index_from_mask(model_mask)
            .and_then(|model_index| self.queues.get(model_index).map(|q| q.len()))
            .unwrap_or(0)
    }

    pub fn total_creds(&self) -> usize {
        self.creds.len()
    }

    pub fn refreshing_len(&self) -> usize {
        self.refreshing.len()
    }

    pub fn is_refreshing(&self, id: CredentialId) -> bool {
        self.refreshing.contains(&id)
    }

    pub fn cooldown_len(&self) -> usize {
        self.cooldown_map.len()
    }

    fn is_model_cooling(&self, id: CredentialId, model_index: ModelIndex) -> bool {
        match self.cooldown_map.get(&(id, model_index)) {
            Some(deadline) => Instant::now() < *deadline,
            None => false,
        }
    }

    fn clear_cooldowns_for(&mut self, id: CredentialId) {
        self.cooldown_map.retain(|(cid, _), _| *cid != id);
    }
}
