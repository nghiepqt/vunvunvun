/// Sealed proof token that can only be constructed by an explicit user action in the UI layer.
///
/// This type intentionally does not implement `serde::Deserialize` or `Default`, preventing
/// deserialized AI outputs or background jobs from ever auto-setting `accepted_at`
/// (CONTEXT.md Section 6).
#[derive(Debug, Clone, Copy)]
pub struct UserAcceptedProof {
    _private: (),
}

impl UserAcceptedProof {
    /// Constructs a proof token at the boundary of an explicit user confirmation gesture.
    #[must_use]
    pub fn from_explicit_user_action() -> Self {
        Self { _private: () }
    }
}
