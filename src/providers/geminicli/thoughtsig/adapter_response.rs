use pollux_schema::gemini::{GeminiResponseBody, Part};
use pollux_thoughtsig_core::{SniffEvent, Sniffable};

pub(super) struct GeminiResponseAdapter<'a>(pub &'a GeminiResponseBody);

impl Sniffable for GeminiResponseAdapter<'_> {
    fn data(&self) -> SniffEvent<'_> {
        let part = self
            .0
            .candidates
            .first()
            .and_then(|candidate| candidate.content.as_ref())
            .and_then(|content| content.parts.first());

        let Some(part) = part else {
            return SniffEvent::None;
        };

        match part {
            Part {
                function_call: Some(function_call),
                ..
            } => SniffEvent::FunctionCall(function_call),
            Part {
                thought: Some(true),
                text: Some(text),
                ..
            } => SniffEvent::ThoughtText(text),
            _ => SniffEvent::None,
        }
    }

    fn thought_signature(&self) -> Option<&str> {
        self.0
            .candidates
            .first()
            .and_then(|candidate| candidate.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| part.thought_signature.as_deref())
    }

    fn index(&self) -> Option<u32> {
        self.0
            .candidates
            .first()
            .and_then(|candidate| candidate.index)
    }

    fn is_finished(&self) -> bool {
        self.0
            .candidates
            .first()
            .and_then(|candidate| candidate.finish_reason.as_ref())
            .is_some()
    }
}
