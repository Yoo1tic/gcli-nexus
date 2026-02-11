use crate::SignatureCacheStore;
use crate::fingerprint::CacheKeyGenerator;
use serde_json::Value;
use std::sync::Arc;

pub enum SniffEvent<'a> {
    ThoughtText(&'a str),
    FunctionCall(&'a Value),
    None,
}

pub trait Sniffable {
    fn data(&self) -> SniffEvent<'_>;
    fn thought_signature(&self) -> Option<&str>;
    fn index(&self) -> Option<u32>;
    fn is_finished(&self) -> bool;
}

#[derive(Debug, Default, Clone)]
pub struct SessionState {
    thought_buffer: String,
    function_buffer: Option<Value>,
    pending_signature: Option<String>,
    current_index: Option<u32>,
}

impl SessionState {
    fn reset(&mut self, new_index: u32) {
        self.thought_buffer.clear();
        self.function_buffer = None;
        self.pending_signature = None;
        self.current_index = Some(new_index);
    }
}

#[derive(Clone)]
pub struct SignatureSniffer {
    pub(crate) moka_cache: SignatureCacheStore,
    pub(crate) state: SessionState,
}

impl SignatureSniffer {
    pub fn new(moka_cache: SignatureCacheStore) -> Self {
        Self {
            moka_cache,
            state: SessionState::default(),
        }
    }

    pub fn inspect<T: Sniffable>(&mut self, item: &T) {
        if let Some(next_index) = item.index() {
            if self.state.current_index != Some(next_index) {
                self.flush();
                self.state.reset(next_index);
            }
        }

        match item.data() {
            SniffEvent::ThoughtText(thought) => self.state.thought_buffer.push_str(thought),
            SniffEvent::FunctionCall(function) => {
                self.state.function_buffer = Some(function.clone())
            }
            SniffEvent::None => {}
        }

        if let Some(signature) = item.thought_signature() {
            self.state.pending_signature = Some(signature.to_string());
        }

        if item.is_finished() {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.state.thought_buffer.is_empty() && self.state.function_buffer.is_none() {
            // No data, so we skip flushing to avoid storing empty keys
            return;
        }

        let Some(signature) = self
            .state
            .pending_signature
            .as_deref()
            .filter(|&s| !s.is_empty())
        else {
            // No signature to store, so we skip flushing
            return;
        };

        let sig_arc: Arc<str> = Arc::from(signature);

        if let Some(text_key) = CacheKeyGenerator::generate_text(&self.state.thought_buffer) {
            self.moka_cache.insert(text_key, sig_arc.clone());
        }

        if let Some(function_key) = self
            .state
            .function_buffer
            .as_ref()
            .and_then(|fc| CacheKeyGenerator::generate_json(fc))
        {
            self.moka_cache.insert(function_key, sig_arc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum DataKind {
        Text(&'static str),
        FunctionCall(Value),
        None,
    }

    struct FakeSniffable {
        data_kind: DataKind,
        signature: Option<&'static str>,
        index: Option<u32>,
        finished: bool,
    }

    impl Sniffable for FakeSniffable {
        fn data(&self) -> SniffEvent<'_> {
            match &self.data_kind {
                DataKind::Text(text) => SniffEvent::ThoughtText(text),
                DataKind::FunctionCall(function_call) => SniffEvent::FunctionCall(function_call),
                DataKind::None => SniffEvent::None,
            }
        }

        fn thought_signature(&self) -> Option<&str> {
            self.signature
        }

        fn index(&self) -> Option<u32> {
            self.index
        }

        fn is_finished(&self) -> bool {
            self.finished
        }
    }

    #[test]
    fn text_signature_is_flushed_into_store() {
        let cache = SignatureCacheStore::builder().max_capacity(128).build();
        let mut sniffer = SignatureSniffer::new(cache.clone());

        let first = FakeSniffable {
            data_kind: DataKind::Text("alpha "),
            signature: None,
            index: Some(0),
            finished: false,
        };
        sniffer.inspect(&first);

        let second = FakeSniffable {
            data_kind: DataKind::Text("beta"),
            signature: Some("sig_001"),
            index: Some(0),
            finished: false,
        };
        sniffer.inspect(&second);

        let third = FakeSniffable {
            data_kind: DataKind::None,
            signature: None,
            index: Some(0),
            finished: true,
        };
        sniffer.inspect(&third);

        let key =
            CacheKeyGenerator::generate_text("alpha beta").expect("text key must be generated");
        let cached = cache.get(&key).expect("text key must be stored");
        assert_eq!(cached, Arc::from("sig_001"));
    }

    #[test]
    fn function_json_hash_is_used_as_key() {
        let cache = SignatureCacheStore::builder().max_capacity(128).build();
        let mut sniffer = SignatureSniffer::new(cache.clone());

        let function_call = serde_json::json!({
            "name": "get_weather",
            "args": { "city": "Berlin", "unit": "c" }
        });

        let item = FakeSniffable {
            data_kind: DataKind::FunctionCall(function_call.clone()),
            signature: Some("sig_fn_001"),
            index: Some(0),
            finished: true,
        };

        sniffer.inspect(&item);

        let key = CacheKeyGenerator::generate_json(&function_call)
            .expect("function hash key must be generated");
        let cached = cache.get(&key).expect("function hash key must be stored");
        assert_eq!(cached, Arc::from("sig_fn_001"));
    }

    #[test]
    fn finished_event_without_signature_does_not_store() {
        let cache = SignatureCacheStore::builder().max_capacity(128).build();
        let mut sniffer = SignatureSniffer::new(cache.clone());

        let item = FakeSniffable {
            data_kind: DataKind::Text("alpha"),
            signature: None,
            index: Some(0),
            finished: true,
        };

        sniffer.inspect(&item);
        let key = CacheKeyGenerator::generate_text("alpha").expect("text key must be generated");
        assert!(cache.get(&key).is_none());
    }
}
