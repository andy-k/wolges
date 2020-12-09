pub struct GaddawgError {
    s: String,
}

impl std::fmt::Display for GaddawgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.s)
    }
}

impl std::fmt::Debug for GaddawgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self as &dyn std::fmt::Display).fmt(f)
    }
}

impl std::error::Error for GaddawgError {}

pub fn new(s: String) -> GaddawgError {
    GaddawgError { s }
}

pub type BoxAnyError = Box<dyn std::error::Error>;
pub type Returns<T> = Result<T, BoxAnyError>;

macro_rules! return_error {
    ($error:expr) => {
        return Err(crate::error::new($error).into());
    };
}
