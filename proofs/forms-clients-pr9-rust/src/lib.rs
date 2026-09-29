#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

pub mod wire;

pub const SOURCE_PR_HEAD: &str = "c75075a7eb2086a17ef717310a24f72b288f5b99";
