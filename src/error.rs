use shellexpand::LookupError;
use std::env::VarError;
use std::error::Error;
use std::fmt;

pub type DotsResult<T> = Result<T, DotsError>;

#[derive(Debug)]
pub enum DotsError {
    Message(String),
    Upstream(Box<dyn Error>),
}

impl fmt::Display for DotsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Upstream(e) => write!(f, "{}", e),
            Self::Message(e) => write!(f, "{}", e),
        }
    }
}

impl Error for DotsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Upstream(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<&str> for DotsError {
    fn from(e: &str) -> Self {
        Self::Message(e.to_string())
    }
}

impl From<String> for DotsError {
    fn from(e: String) -> Self {
        Self::Message(e)
    }
}

impl From<std::io::Error> for DotsError {
    fn from(e: std::io::Error) -> Self {
        Self::Upstream(Box::new(e))
    }
}

impl From<serde_yaml::Error> for DotsError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::Upstream(Box::new(e))
    }
}

impl From<LookupError<VarError>> for DotsError {
    fn from(e: LookupError<VarError>) -> Self {
        Self::Upstream(Box::new(e))
    }
}