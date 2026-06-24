use std::fmt;

#[derive(Debug, Clone)]
pub enum AppError {
    ConfigParse(String),
    XrayProcess(String),
    Network(String),
    Io(String),
    ConfigGeneration(String),
}

impl std::error::Error for AppError {}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::ConfigParse(msg) => write!(f, "Configuration Parsing Error: {}", msg),
            AppError::XrayProcess(msg) => write!(f, "Xray Core Error: {}", msg),
            AppError::Network(msg) => write!(f, "Network/HTTP Error: {}", msg),
            AppError::Io(msg) => write!(f, "File Input/Output Error: {}", msg),
            AppError::ConfigGeneration(msg) => write!(f, "Configuration Generation Error: {}", msg),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::Network(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::ConfigParse(err.to_string())
    }
}
