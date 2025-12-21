use crate::google_oauth::credentials::GoogleCredential;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

pub type CredentialId = i64;
pub type QueueKey = String;

#[derive(Debug, Clone, Serialize)]
pub struct AssignedCredential {
    pub id: CredentialId,
    pub project_id: String,
    pub access_token: String,
}

#[derive(Debug, Default)]
pub struct AssignmentResult {
    pub assigned: Option<AssignedCredential>,
    pub refresh_ids: Vec<CredentialId>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CooldownTicket(Reverse<Instant>, CredentialId, QueueKey);

/// Core scheduling logic for credentials (no IO, no locks).
pub struct CredentialManager {
    creds: HashMap<CredentialId, GoogleCredential>,
    queues: HashMap<QueueKey, VecDeque<CredentialId>>,
    waiting_room: BinaryHeap<CooldownTicket>,
    cooldown_map: HashMap<(CredentialId, QueueKey), Instant>,
    refreshing: HashSet<CredentialId>,
    model_blacklist: HashMap<CredentialId, HashSet<String>>,
}

impl CredentialManager {
    pub fn new() -> Self {
        Self {
            creds: HashMap::new(),
            queues: HashMap::new(),
            waiting_room: BinaryHeap::new(),
            cooldown_map: HashMap::new(),
            refreshing: HashSet::new(),
            model_blacklist: HashMap::new(),
        }
    }

    pub fn add_credential(
        &mut self,
        id: CredentialId,
        cred: GoogleCredential,
        all_keys: &[String],
    ) {
        self.creds.insert(id, cred);
        self.refreshing.remove(&id);

        let blacklist = self.model_blacklist.get(&id);
        for queue_key in all_keys {
            if let Some(set) = blacklist {
                if set.contains(queue_key) {
                    tracing::debug!(
                        "Skipping model {} for credential {} (unsupported)",
                        queue_key,
                        id
                    );
                    continue;
                }
            }

            let queue = self.queues.entry(queue_key.clone()).or_default();
            if !queue.contains(&id) {
                queue.push_back(id);
            }
        }
    }

    pub fn mark_refreshing(&mut self, id: CredentialId) {
        self.refreshing.insert(id);
        self.clear_cooldowns_for(id);
    }

    pub fn mark_model_unsupported(&mut self, id: CredentialId, model: impl AsRef<str>) {
        let model = model.as_ref();
        self.model_blacklist
            .entry(id)
            .or_default()
            .insert(model.to_string());
        tracing::warn!(
            "Credential {} marked as unsupported for model {}",
            id,
            model
        );
    }

    pub fn delete_credential(&mut self, id: CredentialId) {
        self.creds.remove(&id);
        self.refreshing.remove(&id);
        self.clear_cooldowns_for(id);
        self.model_blacklist.remove(&id);
    }

    pub fn report_rate_limit(&mut self, id: CredentialId, queue_key: &str, cooldown: Duration) {
        let deadline = Instant::now() + cooldown;
        let key = queue_key.to_string();

        self.cooldown_map.insert((id, key.clone()), deadline);
        self.waiting_room
            .push(CooldownTicket(Reverse(deadline), id, key));
    }

    pub fn refresh_success(&mut self, id: CredentialId, new_cred: GoogleCredential) {
        if let Some(cred) = self.creds.get_mut(&id) {
            *cred = new_cred;
        }
        self.refreshing.remove(&id);
    }

    pub fn get_full_credential_copy(&self, id: CredentialId) -> Option<GoogleCredential> {
        self.creds.get(&id).cloned()
    }

    pub fn project_id_of(&self, id: CredentialId) -> Option<String> {
        self.creds.get(&id).map(|c| c.project_id.clone())
    }

    pub fn contains(&self, id: CredentialId) -> bool {
        self.creds.contains_key(&id)
    }

    pub fn get_assigned(&mut self, queue_key: impl AsRef<str>) -> AssignmentResult {
        let queue_key = queue_key.as_ref();
        self.process_waiting_room();

        let mut result = AssignmentResult::default();

        while let Some(id) = self.queues.get_mut(queue_key).and_then(|q| q.pop_front()) {
            let Some(cred) = Some(id)
                .filter(|id| !self.refreshing.contains(id))
                .filter(|id| !self.is_model_cooling(*id, queue_key))
                .and_then(|id| self.creds.get(&id))
            else {
                continue;
            };

            let Some(token) = cred
                .access_token
                .as_ref()
                .filter(|_| !cred.is_expired())
                .cloned()
            else {
                tracing::debug!(
                    "Credential {} is unavailable (expired or missing token), scheduling refresh.",
                    id
                );
                result.refresh_ids.push(id);
                continue;
            };

            if let Some(queue) = self.queues.get_mut(queue_key) {
                queue.push_back(id);
            }

            result.assigned = Some(AssignedCredential {
                id,
                project_id: cred.project_id.clone(),
                access_token: token,
            });
            return result;
        }
        result
    }

    fn process_waiting_room(&mut self) {
        let now = Instant::now();

        while self.waiting_room.peek().map_or(false, |t| (t.0).0 <= now) {
            let CooldownTicket(Reverse(ticket_deadline), credential_id, queue_key) =
                self.waiting_room.pop().expect("peek guaranteed existence");

            match self.cooldown_map.entry((credential_id, queue_key)) {
                std::collections::hash_map::Entry::Occupied(entry)
                    if ticket_deadline >= *entry.get() =>
                {
                    let ((reclaimed_cred_id, reclaimed_queue_key), _) = entry.remove_entry();
                    self.queues
                        .get_mut(&reclaimed_queue_key)
                        .map(|target_queue| {
                            target_queue.push_back(reclaimed_cred_id);
                            tracing::info!(
                                "Reclaiming credential {} from cooldown for queue {}",
                                reclaimed_cred_id,
                                reclaimed_queue_key
                            );
                        });
                }
                _ => {}
            }
        }
    }

    pub fn queue_len(&self, key: impl AsRef<str>) -> usize {
        self.queues.get(key.as_ref()).map(|q| q.len()).unwrap_or(0)
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

    fn is_model_cooling(&self, id: CredentialId, key: impl AsRef<str>) -> bool {
        match self.cooldown_map.get(&(id, key.as_ref().to_string())) {
            Some(deadline) => Instant::now() < *deadline,
            None => false,
        }
    }

    fn clear_cooldowns_for(&mut self, id: CredentialId) {
        self.cooldown_map.retain(|(cid, _), _| *cid != id);
    }
}
