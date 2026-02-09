//! Gemini CLI (Cloud Code / AiStudio) schema.

mod cli_request;
mod cli_response;

pub use cli_request::{GeminiCliRequest, GeminiCliRequestMeta};
pub use cli_response::GeminiCliResponseBody;
