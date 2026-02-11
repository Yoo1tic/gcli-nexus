use crate::ThoughtSignatureEngine;

pub trait ThoughtSigPatchable {
    fn should_patch(&self) -> bool;

    fn patch_thought_signatures(&mut self, engine: &ThoughtSignatureEngine);
}
