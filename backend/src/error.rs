//! The one error type used across the backend. Commands return it to the UI as a plain message.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error("clipboard: {0}")]
    Clipboard(#[from] arboard::Error),
    #[error("couldn't encode the image: {0}")]
    Png(#[from] png::EncodingError),
    /// The UI sent something the command can't use.
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
