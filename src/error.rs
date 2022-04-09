// Copyright (C) 2020-2022 Andy Kurnia.

pub struct MyError {
    s: String,
}

impl std::fmt::Display for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.s)
    }
}

impl std::fmt::Debug for MyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self as &dyn std::fmt::Display).fmt(f)
    }
}

impl std::error::Error for MyError {}

pub fn new(s: String) -> MyError {
    MyError { s }
}

pub type BoxAnyError = Box<dyn std::error::Error>;
pub type Returns<T> = Result<T, BoxAnyError>;

#[macro_export]
macro_rules! return_error {
    ($error:expr) => {
        return Err($crate::error::new($error).into());
    };
}
