pub mod git;
pub mod model;
pub mod output;

use std::fmt;

#[derive(Debug)]
pub struct AppError(pub String);

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;
